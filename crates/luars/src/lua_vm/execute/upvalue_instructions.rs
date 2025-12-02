/// Upvalue and closure operations
///
/// These instructions handle upvalues, closures, and variable captures.
use crate::{
    LuaValue,
    lua_vm::{Instruction, LuaCallFrame, LuaResult, LuaVM},
};

/// GETUPVAL A B
/// R[A] := UpValue[B]
#[inline(always)]
pub fn exec_getupval(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, pc: &mut usize, base_ptr: &mut usize) {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let func_value = unsafe { (*frame_ptr).function_value };

    // OPTIMIZED: Use unchecked access since we know the function is valid
    let func_id = unsafe { func_value.as_function_id().unwrap_unchecked() };
    let func_ref = unsafe { vm.object_pool.get_function_unchecked(func_id) };
    let upvalue_id = unsafe { *func_ref.upvalues.get_unchecked(b) };

    // OPTIMIZED: Use unchecked read for hot path
    // SAFETY: upvalue_id is from a valid function closure
    let value = unsafe { vm.read_upvalue_unchecked(upvalue_id) };
    unsafe {
        *vm.register_stack.get_unchecked_mut(*base_ptr + a) = value;
    }
}

/// SETUPVAL A B
/// UpValue[B] := R[A]
#[inline(always)]
pub fn exec_setupval(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, pc: &mut usize, base_ptr: &mut usize) {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let func_value = unsafe { (*frame_ptr).function_value };

    // OPTIMIZED: Use unchecked access since we know the function is valid
    let func_id = unsafe { func_value.as_function_id().unwrap_unchecked() };
    let func_ref = unsafe { vm.object_pool.get_function_unchecked(func_id) };
    let upvalue_id = unsafe { *func_ref.upvalues.get_unchecked(b) };

    let value = unsafe { *vm.register_stack.get_unchecked(*base_ptr + a) };

    // Set upvalue value using the helper method
    vm.write_upvalue(upvalue_id, value);
}

/// CLOSE A
/// close all upvalues >= R[A]
#[inline(always)]
pub fn exec_close(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, pc: &mut usize, base_ptr: &mut usize) {
    let a = Instruction::get_a(instr) as usize;
    let close_from = *base_ptr + a;

    vm.close_upvalues_from(close_from);
}

/// CLOSURE A Bx
/// R[A] := closure(KPROTO[Bx])
/// OPTIMIZED: Fast path for closures without upvalues
#[inline(always)]
pub fn exec_closure(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, pc: &mut usize, base_ptr: &mut usize) -> LuaResult<()> {
    use crate::gc::UpvalueId;


    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let func_value = unsafe { (*frame_ptr).function_value };
    // Get current function using ID-based lookup
    let func_id = func_value
        .as_function_id()
        .ok_or_else(|| vm.error("Not a Lua function".to_string()))?;

    let func_ref = vm.object_pool.get_function(func_id);
    let (proto, parent_upvalues) = if let Some(f) = func_ref {
        let p = f.chunk.child_protos.get(bx).cloned();
        if let Some(proto) = p {
            (proto, f.upvalues.clone())
        } else {
            return Err(vm.error(format!("Invalid prototype index: {}", bx)));
        }
    } else {
        return Err(vm.error("Invalid function reference".to_string()));
    };

    // FAST PATH: No upvalues (most common for simple lambdas)
    if proto.upvalue_descs.is_empty() {
        let closure = vm.create_function(proto, Vec::new());
        unsafe {
            *vm.register_stack.get_unchecked_mut(*base_ptr + a) = closure;
        }
        vm.check_gc();
        return Ok(());
    }

    // Get upvalue descriptors from the prototype
    let upvalue_descs = proto.upvalue_descs.clone();

    // Create upvalues for the new closure based on descriptors
    let mut upvalue_ids: Vec<UpvalueId> = Vec::with_capacity(upvalue_descs.len());
    let mut new_open_upvalue_ids: Vec<UpvalueId> = Vec::new();

    for desc in upvalue_descs.iter() {
        if desc.is_local {
            // Upvalue refers to a register in current function
            // Calculate absolute stack index for this upvalue
            let stack_index = *base_ptr + desc.index as usize;

            // Check if this upvalue is already open
            let existing = vm.open_upvalues.iter().find(|uv_id| {
                vm.object_pool
                    .get_upvalue(**uv_id)
                    .map(|uv| uv.points_to_index(stack_index))
                    .unwrap_or(false)
            });

            if let Some(&existing_uv_id) = existing {
                upvalue_ids.push(existing_uv_id);
            } else {
                // Create new open upvalue using absolute stack index
                let new_uv_id = vm.object_pool.create_upvalue_open(stack_index);
                upvalue_ids.push(new_uv_id);
                new_open_upvalue_ids.push(new_uv_id);
            }
        } else {
            // Upvalue refers to an upvalue in the enclosing function
            if let Some(&parent_uv_id) = parent_upvalues.get(desc.index as usize) {
                upvalue_ids.push(parent_uv_id);
            } else {
                return Err(vm.error(format!("Invalid upvalue index in parent: {}", desc.index)));
            }
        }
    }

    // Add all new upvalues to the open list
    vm.open_upvalues.extend(new_open_upvalue_ids);

    let closure = vm.create_function(proto, upvalue_ids);
    unsafe {
        *vm.register_stack.get_unchecked_mut(*base_ptr + a) = closure;
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
pub fn exec_vararg(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, pc: &mut usize, base_ptr: &mut usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let (base_ptr, vararg_start, vararg_count, top) = unsafe {
        let frame = &*frame_ptr;
        (
            frame.base_ptr,
            frame.get_vararg_start(),
            frame.get_vararg_count(),
            frame.top,
        )
    };

    if c == 0 {
        // Variable number of results - copy all varargs
        // Update frame top to accommodate all varargs
        let new_top = a + vararg_count;
        unsafe {
            (*frame_ptr).top = new_top.max(top);
        }

        for i in 0..vararg_count {
            let value = if vararg_start + i < vm.register_stack.len() {
                vm.register_stack[vararg_start + i]
            } else {
                LuaValue::nil()
            };
            vm.register_stack[base_ptr + a + i] = value;
        }
    } else {
        // Fixed number of results (c-1 values)
        let count = c - 1;
        for i in 0..count {
            let value = if i < vararg_count && vararg_start + i < vm.register_stack.len() {
                vm.register_stack[vararg_start + i]
            } else {
                LuaValue::nil()
            };
            vm.register_stack[base_ptr + a + i] = value;
        }
    }

    Ok(())
}

/// CONCAT A B
/// R[A] := R[A].. ... ..R[A+B]
/// OPTIMIZED: Pre-allocation for string/number combinations
#[inline(always)]
pub fn exec_concat(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, pc: &mut usize, base_ptr: &mut usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    // ULTRA-OPTIMIZED: Build result string directly without intermediate allocations
    // Estimate total capacity and format numbers inline with itoa
    let mut total_capacity = 0usize;
    let mut all_simple = true;

    // First pass: check types and estimate capacity
    for i in 0..=b {
        let value = vm.register_stack[*base_ptr + a + i];

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
            let value = vm.register_stack[*base_ptr + a + i];

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
        vm.register_stack[*base_ptr + a] = result_value;

        // No GC check for fast path - rely on debt mechanism
        // Only large allocations trigger automatic GC
        return Ok(());
    }

    // Slow path: need to handle metamethods
    let mut result_value = vm.register_stack[*base_ptr + a];

    for i in 1..=b {
        let next_value = vm.register_stack[*base_ptr + a + i];

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

    vm.register_stack[*base_ptr + a] = result_value;

    // No GC check - rely on debt mechanism
    Ok(())
}

/// SETLIST A B C k
/// R[A][C+i] := R[A+i], 1 <= i <= B
#[inline(always)]
pub fn exec_setlist(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, pc: &mut usize, base_ptr: &mut usize) {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let top = unsafe { (*frame_ptr).top };
    let table = vm.register_stack[*base_ptr + a];

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
                    *vm.register_stack.get_unchecked(*base_ptr + a + 1 + i);
            }
        }
        return;
    }

    // Slow path with metamethods
    for i in 0..count {
        let key = LuaValue::integer((start_idx + i + 1) as i64);
        let value = vm.register_stack[*base_ptr + a + i + 1];
        let _ = vm.table_set_with_meta(table, key, value);
    }
}

/// TBC A
/// mark variable A as to-be-closed
/// This marks a variable to have its __close metamethod called when it goes out of scope
#[inline(always)]
pub fn exec_tbc(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, pc: &mut usize, base_ptr: &mut usize) {
    let a = Instruction::get_a(instr) as usize;
    let reg_idx = *base_ptr + a;

    // Get the value to be marked as to-be-closed
    let value = vm.register_stack[reg_idx];

    // Add to to_be_closed stack (will be processed in LIFO order)
    // Store absolute register index for later closing
    vm.to_be_closed.push((reg_idx, value));
}
