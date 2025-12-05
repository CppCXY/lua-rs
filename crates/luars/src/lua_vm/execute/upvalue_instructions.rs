/// Upvalue and closure operations
///
/// These instructions handle upvalues, closures, and variable captures.
use crate::{
    LuaValue, get_a, get_b, get_bx, get_c,
    lua_value::LuaThread,
    lua_vm::{LuaCallFrame, LuaResult, LuaVM},
};

/// GETUPVAL A B
/// R[A] := UpValue[B]
/// Get upvalue from the closure's upvalue list
/// OPTIMIZED: Uses cached upvalues_ptr in frame for direct access
#[allow(dead_code)]
#[inline(always)]
pub fn exec_getupval(thread: &mut LuaThread, vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);

    // FAST PATH: Use cached upvalues_ptr for direct access
    let upvalue_id = unsafe { *(*frame_ptr).upvalues_ptr.add(b) };

    // Read the upvalue value
    let value = unsafe { vm.read_upvalue_unchecked(upvalue_id) };
    unsafe {
        *thread.register_stack.get_unchecked_mut(base_ptr + a) = value;
    }
}

/// SETUPVAL A B
/// UpValue[B] := R[A]
/// Set upvalue in the closure's upvalue list
/// OPTIMIZED: Uses cached upvalues_ptr in frame for direct access
#[allow(dead_code)]
#[inline(always)]
pub fn exec_setupval(thread: &mut LuaThread, vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);

    // FAST PATH: Use cached upvalues_ptr for direct access
    let upvalue_id = unsafe { *(*frame_ptr).upvalues_ptr.add(b) };

    let value = unsafe { *thread.register_stack.get_unchecked(base_ptr + a) };

    // Write to the upvalue
    unsafe {
        vm.write_upvalue_unchecked(upvalue_id, value);
    }
}

/// CLOSE A
/// close all upvalues >= R[A]
#[inline(always)]
pub fn exec_close(thread: &mut LuaThread, vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);
    let close_from = base_ptr + a;

    vm.close_upvalues_from(close_from);
}

/// CLOSURE A Bx
/// R[A] := closure(KPROTO[Bx])
/// OPTIMIZED: Fast path for closures without upvalues, avoid unnecessary clones
#[inline(always)]
pub fn exec_closure(
    thread: &mut LuaThread,
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: usize,
) -> LuaResult<()> {
    use crate::gc::UpvalueId;

    let a = get_a!(instr);
    let bx = get_bx!(instr);

    // Get prototype and parent upvalues without cloning upvalues early
    let func_id = unsafe { (*frame_ptr).get_function_id_unchecked() };
    let func_ref = unsafe { vm.object_pool.get_function_unchecked(func_id) };

    let proto = func_ref.lua_chunk().child_protos.get(bx).cloned();
    let proto = match proto {
        Some(p) => p,
        None => return Err(vm.error(format!("Invalid prototype index: {}", bx))),
    };

    // FAST PATH: No upvalues (most common for simple lambdas)
    let upvalue_count = proto.upvalue_descs.len();
    if upvalue_count == 0 {
        let closure = vm.create_function(proto, Vec::new());
        unsafe {
            *thread.register_stack.get_unchecked_mut(base_ptr + a) = closure;
        }
        vm.check_gc();
        return Ok(());
    }

    // Pre-allocate upvalue_ids with exact capacity (no separate new_open_upvalue_ids Vec)
    let mut upvalue_ids: Vec<UpvalueId> = Vec::with_capacity(upvalue_count);

    // Process upvalue descriptors without cloning
    for desc in proto.upvalue_descs.iter() {
        if desc.is_local {
            // Upvalue refers to a register in current function
            let stack_index = base_ptr + desc.index as usize;

            // Check if this upvalue is already open - use linear search (usually small list)
            let mut found = None;
            for &uv_id in thread.open_upvalues.iter() {
                // SAFETY: upvalue IDs in open_upvalues are always valid
                let uv = unsafe { vm.object_pool.get_upvalue_unchecked(uv_id) };
                if uv.points_to_index(stack_index) {
                    found = Some(uv_id);
                    break;
                }
            }

            if let Some(existing_uv_id) = found {
                upvalue_ids.push(existing_uv_id);
            } else {
                // Create new open upvalue and add to open list directly
                let new_uv_id = vm.create_upvalue_open(stack_index);
                upvalue_ids.push(new_uv_id);
                thread.open_upvalues.push(new_uv_id);
            }
        } else {
            // Upvalue refers to an upvalue in the enclosing function
            // Need to get parent upvalues again (func_ref may be invalidated by alloc)
            let parent_func = unsafe { vm.object_pool.get_function_unchecked(func_id) };
            if let Some(&parent_uv_id) = parent_func.upvalues.get(desc.index as usize) {
                upvalue_ids.push(parent_uv_id);
            } else {
                return Err(vm.error(format!("Invalid upvalue index in parent: {}", desc.index)));
            }
        }
    }

    let closure = vm.create_function(proto, upvalue_ids);
    unsafe {
        *thread.register_stack.get_unchecked_mut(base_ptr + a) = closure;
    }

    // GC checkpoint: closure now safely stored in register
    vm.check_gc();

    Ok(())
}

/// VARARG A C
/// R[A], R[A+1], ..., R[A+C-2] = vararg
///
/// Vararg arguments are stored at frame.vararg_start (set by VARARGPREP).
/// This instruction copies them to the target registers.
#[inline(always)]
pub fn exec_vararg(
    thread: &mut LuaThread,
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let c = get_c!(instr);

    let (vararg_start, vararg_count, top) = unsafe {
        let frame = &*frame_ptr;
        (
            frame.get_vararg_start(),
            frame.get_vararg_count(),
            frame.top as usize,
        )
    };

    let dest_base = base_ptr + a;
    let reg_ptr = thread.register_stack.as_mut_ptr();

    if c == 0 {
        // Variable number of results - copy all varargs
        // Update frame top to accommodate all varargs
        let new_top = a + vararg_count;
        unsafe {
            (*frame_ptr).top = (new_top.max(top)) as u32;
        }

        // OPTIMIZED: Use ptr::copy for bulk transfer when possible
        if vararg_count > 0 && vararg_start + vararg_count <= thread.register_stack.len() {
            unsafe {
                std::ptr::copy(
                    reg_ptr.add(vararg_start),
                    reg_ptr.add(dest_base),
                    vararg_count,
                );
            }
        } else {
            // Fallback: copy with bounds checking
            let nil_val = LuaValue::nil();
            for i in 0..vararg_count {
                let value = if vararg_start + i < thread.register_stack.len() {
                    unsafe { *reg_ptr.add(vararg_start + i) }
                } else {
                    nil_val
                };
                unsafe {
                    *reg_ptr.add(dest_base + i) = value;
                }
            }
        }
    } else {
        // Fixed number of results (c-1 values)
        let count = c - 1;
        let copy_count = count.min(vararg_count);
        let nil_count = count.saturating_sub(vararg_count);

        // OPTIMIZED: Bulk copy available varargs
        if copy_count > 0 && vararg_start + copy_count <= thread.register_stack.len() {
            unsafe {
                std::ptr::copy(
                    reg_ptr.add(vararg_start),
                    reg_ptr.add(dest_base),
                    copy_count,
                );
            }
        }

        // Fill remaining with nil
        if nil_count > 0 {
            let nil_val = LuaValue::nil();
            for i in copy_count..count {
                unsafe {
                    *reg_ptr.add(dest_base + i) = nil_val;
                }
            }
        }
    }

    Ok(())
}

/// CONCAT A B
/// R[A] := R[A].. ... ..R[A+B]
/// OPTIMIZED: Pre-allocation for string/number combinations
#[inline(always)]
pub fn exec_concat(thread: &mut LuaThread, vm: &mut LuaVM, instr: u32, base_ptr: usize) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);

    // ULTRA-OPTIMIZED: Build result string directly without intermediate allocations
    // Estimate total capacity and format numbers inline with itoa
    let mut total_capacity = 0usize;
    let mut all_simple = true;

    // First pass: check types and estimate capacity
    for i in 0..=b {
        let value = thread.register_stack[base_ptr + a + i];

        if let Some(str_id) = value.as_string_id() {
            if let Some(s) = vm.object_pool.get_string(str_id) {
                total_capacity += s.as_str().len();
            }
        } else if value.is_integer() {
            total_capacity += 20; // Max digits for i64
        } else if value.is_number() {
            total_capacity += 30; // Max digits for f64
        } else {
            all_simple = false;
            break;
        }
    }

    // Fast path: all strings/numbers, concatenate directly
    if all_simple {
        let mut result = String::with_capacity(total_capacity);
        let mut int_buffer = itoa::Buffer::new();
        let mut float_buffer = ryu::Buffer::new();

        for i in 0..=b {
            let value = thread.register_stack[base_ptr + a + i];

            if let Some(str_id) = value.as_string_id() {
                if let Some(s) = vm.object_pool.get_string(str_id) {
                    result.push_str(s.as_str());
                }
            } else if let Some(int_val) = value.as_integer() {
                // OPTIMIZED: Direct formatting with itoa
                result.push_str(int_buffer.format(int_val));
            } else if let Some(float_val) = value.as_number() {
                // OPTIMIZED: Direct formatting with ryu
                result.push_str(float_buffer.format(float_val));
            }
        }

        // OPTIMIZED: Use create_string_owned to avoid extra clone
        let result_value = vm.create_string_owned(result);
        thread.register_stack[base_ptr + a] = result_value;

        // GC checkpoint - Lua checks GC after CONCAT
        vm.check_gc();
        return Ok(());
    }

    // Slow path: need to handle metamethods
    let mut result_value = thread.register_stack[base_ptr + a];

    for i in 1..=b {
        let next_value = thread.register_stack[base_ptr + a + i];

        // Try direct concatenation first
        let left_str = if let Some(str_id) = result_value.as_string_id() {
            vm.object_pool
                .get_string(str_id)
                .map(|s| s.as_str().to_string())
        } else if let Some(int_val) = result_value.as_integer() {
            Some(int_val.to_string())
        } else if let Some(float_val) = result_value.as_number() {
            Some(float_val.to_string())
        } else {
            None
        };

        let right_str = if let Some(str_id) = next_value.as_string_id() {
            vm.object_pool
                .get_string(str_id)
                .map(|s| s.as_str().to_string())
        } else if let Some(int_val) = next_value.as_integer() {
            Some(int_val.to_string())
        } else if let Some(float_val) = next_value.as_number() {
            Some(float_val.to_string())
        } else {
            None
        };

        if let (Some(l), Some(r)) = (left_str, right_str) {
            let concat_result = l + &r;
            result_value = vm.create_string_owned(concat_result);
        } else {
            // Try __concat metamethod - use pre-cached StringId
            let mm_key = LuaValue::string(vm.object_pool.tm_concat);
            let mut found_metamethod = false;

            if let Some(mt) = vm.table_get_metatable(&result_value) {
                if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key) {
                    if !metamethod.is_nil() {
                        if let Some(mm_result) =
                            vm.call_metamethod(&metamethod, &[result_value, next_value])?
                        {
                            result_value = mm_result;
                            found_metamethod = true;
                        }
                    }
                }
            }

            if !found_metamethod {
                if let Some(mt) = vm.table_get_metatable(&next_value) {
                    if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key) {
                        if !metamethod.is_nil() {
                            if let Some(mm_result) =
                                vm.call_metamethod(&metamethod, &[result_value, next_value])?
                            {
                                result_value = mm_result;
                                found_metamethod = true;
                            }
                        }
                    }
                }
            }

            if !found_metamethod {
                return Err(vm.error(format!(
                    "attempt to concatenate a {} value",
                    if result_value.is_string() || result_value.is_number() {
                        next_value.type_name()
                    } else {
                        result_value.type_name()
                    }
                )));
            }
        }
    }

    thread.register_stack[base_ptr + a] = result_value;

    // GC checkpoint - Lua checks GC after CONCAT
    vm.check_gc();
    Ok(())
}

/// SETLIST A B C k
/// R[A][C+i] := R[A+i], 1 <= i <= B
#[inline(always)]
pub fn exec_setlist(thread: &mut LuaThread, vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    let top = unsafe { (*frame_ptr).top } as usize;
    let table = thread.register_stack[base_ptr + a];

    let start_idx = c * 50; // 0-based for array indexing
    let count = if b == 0 { top - a - 1 } else { b };

    // Fast path: direct array manipulation using unchecked access
    if let Some(table_id) = table.as_table_id() {
        // SAFETY: table_id is valid because it came from as_table_id()
        let t = unsafe { vm.object_pool.get_table_mut_unchecked(table_id) };

        // Reserve space
        let needed = start_idx + count;
        if t.array.len() < needed {
            t.array.resize(needed, crate::LuaValue::nil());
        }

        // Copy all values using unchecked access
        for i in 0..count {
            unsafe {
                *t.array.get_unchecked_mut(start_idx + i) =
                    *thread.register_stack.get_unchecked(base_ptr + a + 1 + i);
            }
        }
        return;
    }

    // Slow path with metamethods
    for i in 0..count {
        let key = LuaValue::integer((start_idx + i + 1) as i64);
        let value = thread.register_stack[base_ptr + a + i + 1];
        let _ = vm.table_set_with_meta(table, key, value);
    }
}

/// TBC A
/// mark variable A as to-be-closed
/// This marks a variable to have its __close metamethod called when it goes out of scope
#[inline(always)]
pub fn exec_tbc(thread: &mut LuaThread, vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);
    let reg_idx = base_ptr + a;

    // Get the value to be marked as to-be-closed
    let value = thread.register_stack[reg_idx];

    // Add to to_be_closed stack (will be processed in LIFO order)
    // Store absolute register index for later closing
    vm.to_be_closed.push((reg_idx, value));
}

