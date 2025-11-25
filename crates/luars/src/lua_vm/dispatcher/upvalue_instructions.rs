/// Upvalue and closure operations
///
/// These instructions handle upvalues, closures, and variable captures.
use crate::{
    LuaValue,
    lua_vm::{Instruction, LuaResult, LuaVM},
};

/// GETUPVAL A B
/// R[A] := UpValue[B]
pub fn exec_getupval(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let Some(func_ref) = frame.get_lua_function() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    let Some(upvalue) = func_ref.borrow().upvalues.get(b).cloned() else {
        return Err(vm.error(format!("Invalid upvalue index: {}", b)));
    };

    let value = upvalue.get_value(&vm.frames, &vm.register_stack);
    vm.register_stack[base_ptr + a] = value;

    Ok(())
}

/// SETUPVAL A B
/// UpValue[B] := R[A]
pub fn exec_setupval(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let Some(func_ref) = frame.get_lua_function() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    let Some(upvalue) = func_ref.borrow().upvalues.get(b).cloned() else {
        return Err(vm.error(format!("Invalid upvalue index: {}", b)));
    };

    let value = vm.register_stack[base_ptr + a];

    upvalue.set_value(&mut vm.frames, &mut vm.register_stack, value);

    Ok(())
}

/// CLOSE A
/// close all upvalues >= R[A]
pub fn exec_close(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    let close_from = base_ptr + a;

    vm.close_upvalues_from(close_from);

    Ok(())
}

/// CLOSURE A Bx
/// R[A] := closure(KPROTO[Bx])
pub fn exec_closure(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let Some(func_ref) = frame.get_lua_function() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    let Some(proto) = func_ref.borrow().chunk.child_protos.get(bx).cloned() else {
        return Err(vm.error(format!("Invalid prototype index: {}", bx)));
    };

    // Get upvalue descriptors from the prototype
    let upvalue_descs = proto.upvalue_descs.clone();

    // Create upvalues for the new closure based on descriptors
    let mut upvalues = Vec::new();
    let mut new_open_upvalues = Vec::new();

    for desc in upvalue_descs.iter() {
        if desc.is_local {
            // Upvalue refers to a register in current function

            // Check if this upvalue is already open
            let existing_index = vm
                .open_upvalues
                .iter()
                .position(|uv| uv.points_to(frame.frame_id, desc.index as usize));

            if let Some(idx) = existing_index {
                upvalues.push(vm.open_upvalues[idx].clone());
            } else {
                // Create new open upvalue
                let new_uv =
                    crate::lua_vm::LuaUpvalue::new_open(frame.frame_id, desc.index as usize);
                upvalues.push(new_uv.clone());
                new_open_upvalues.push(new_uv);
            }
        } else {
            // Upvalue refers to an upvalue in the enclosing function
            if let Some(parent_uv) = func_ref.borrow().upvalues.get(desc.index as usize) {
                upvalues.push(parent_uv.clone());
            } else {
                return Err(vm.error(format!("Invalid upvalue index in parent: {}", desc.index)));
            }
        }
    }

    // Add all new upvalues to the open list
    vm.open_upvalues.extend(new_open_upvalues);

    let closure = vm.create_function(proto, upvalues);
    vm.register_stack[base_ptr + a] = closure;

    // GC checkpoint: closure now safely stored in register
    vm.check_gc();

    Ok(())
}

/// VARARG A C
/// R[A], R[A+1], ..., R[A+C-2] = vararg
pub fn exec_vararg(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    let vararg_start = frame.vararg_start;
    let vararg_count = frame.get_vararg_count();

    if c == 0 {
        // Variable number of results - copy all varargs
        // Update frame top to accommodate all varargs
        let new_top = a + vararg_count;
        vm.current_frame_mut().top = new_top.max(frame.top);

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
pub fn exec_concat(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // ULTRA-OPTIMIZED: Build result string directly without intermediate allocations
    // Estimate total capacity and format numbers inline with itoa
    let mut total_capacity = 0usize;
    let mut all_simple = true;

    // First pass: check types and estimate capacity
    for i in 0..=b {
        let value = vm.register_stack[base_ptr + a + i];

        if let Some(s) = value.as_lua_string() {
            total_capacity += s.as_str().len();
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
            let value = vm.register_stack[base_ptr + a + i];

            if let Some(s) = value.as_lua_string() {
                result.push_str(s.as_str());
            } else if let Some(int_val) = value.as_integer() {
                // OPTIMIZED: Direct formatting with itoa
                result.push_str(int_buffer.format(int_val));
            } else if let Some(float_val) = value.as_number() {
                // OPTIMIZED: Direct formatting with ryu
                result.push_str(float_buffer.format(float_val));
            }
        }

        let result_value = vm.create_string(&result);
        vm.register_stack[base_ptr + a] = result_value;

        // No GC check for fast path - rely on debt mechanism
        // Only large allocations trigger automatic GC
        return Ok(());
    }

    // Slow path: need to handle metamethods
    let mut result_value = vm.register_stack[base_ptr + a];

    for i in 1..=b {
        let next_value = vm.register_stack[base_ptr + a + i];

        // Try direct concatenation first
        let left_str = if let Some(s) = result_value.as_lua_string() {
            Some(s.as_str().to_string())
        } else if let Some(int_val) = result_value.as_integer() {
            Some(int_val.to_string())
        } else if let Some(float_val) = result_value.as_number() {
            Some(float_val.to_string())
        } else {
            None
        };

        let right_str = if let Some(s) = next_value.as_lua_string() {
            Some(s.as_str().to_string())
        } else if let Some(int_val) = next_value.as_integer() {
            Some(int_val.to_string())
        } else if let Some(float_val) = next_value.as_number() {
            Some(float_val.to_string())
        } else {
            None
        };

        if let (Some(l), Some(r)) = (left_str, right_str) {
            let concat_result = l + &r;
            result_value = vm.create_string(&concat_result);
        } else {
            // Try __concat metamethod
            let mm_key = vm.create_string("__concat");
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

    vm.register_stack[base_ptr + a] = result_value;

    // No GC check - rely on debt mechanism
    Ok(())
}

/// SETLIST A B C k
/// R[A][C+i] := R[A+i], 1 <= i <= B
pub fn exec_setlist(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;
    let _k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let table = vm.register_stack[base_ptr + a];

    // Calculate starting index
    let start_idx = c * 50 + 1; // LFIELDS_PER_FLUSH = 50 in Lua

    let count = if b == 0 {
        // Use all values to top of stack
        frame.top - a - 1
    } else {
        b
    };

    // OPTIMIZATION: SETLIST is typically used for table initialization
    // Batch all operations in a single borrow_mut() to minimize RefCell overhead
    // This is safe because:
    // 1. SETLIST is only emitted by compiler for table constructors
    // 2. Newly created tables don't have metatables yet
    // 3. Even if metatable exists, Lua doesn't call __newindex for array part

    // Fast path: Use cached pointer (avoids ObjectPool lookup)
    if let Some(lua_table) = table.as_lua_table() {
        // ULTRA-OPTIMIZED: Sequential insertion from start_idx
        let mut table_mut = lua_table.borrow_mut();

        // Reserve capacity upfront to avoid reallocations
        let expected_end_idx = (start_idx - 1 + count) as usize;
        let current_len = table_mut.array.len();
        if table_mut.array.capacity() < expected_end_idx && expected_end_idx > current_len {
            table_mut.array.reserve(expected_end_idx - current_len);
        }

        // Batch process all values with proper index handling
        for i in 0..count {
            let key_int = (start_idx + i) as i64;
            let value = vm.register_stack[base_ptr + a + i + 1];

            // Use optimized set_int which correctly handles nils and gaps
            let idx = (key_int - 1) as usize;

            if value.is_nil() {
                // Nil values: only store if filling a gap in existing array
                if idx < table_mut.array.len() {
                    table_mut.array[idx] = crate::LuaValue::nil();
                }
                // Don't extend array for trailing nils
            } else {
                // Non-nil value: ensure array is large enough
                if idx < table_mut.array.len() {
                    table_mut.array[idx] = value;
                } else if idx == table_mut.array.len() {
                    // Fast append
                    table_mut.array.push(value);
                } else {
                    // Gap: fill with nils then append value
                    while table_mut.array.len() < idx {
                        table_mut.array.push(crate::LuaValue::nil());
                    }
                    table_mut.array.push(value);
                }
            }
        }
        drop(table_mut); // Release borrow early

        // Write barrier for GC (outside borrow scope)
        // OPTIMIZATION: Only check for GC-relevant values
        if let Some(table_id) = table.as_table_id() {
            vm.gc
                .barrier_forward(crate::gc::GcObjectType::Table, table_id.0);

            // Optimized barrier: skip primitives
            for i in 0..count {
                let value = vm.register_stack[base_ptr + a + i + 1];
                if !value.is_nil() && !value.is_number() && !value.is_boolean() {
                    vm.gc.barrier_back(&value);
                }
            }
        }
    } else {
        // Slow path: fallback to table_set_with_meta
        for i in 0..count {
            let key = LuaValue::integer((start_idx + i) as i64);
            let value = vm.register_stack[base_ptr + a + i + 1];
            vm.table_set_with_meta(table, key, value)?;
        }
    }

    Ok(())
}

/// TBC A
/// mark variable A as to-be-closed
/// This marks a variable to have its __close metamethod called when it goes out of scope
pub fn exec_tbc(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    let reg_idx = base_ptr + a;

    // Get the value to be marked as to-be-closed
    let value = vm.register_stack[reg_idx];

    // Add to to_be_closed stack (will be processed in LIFO order)
    // Store absolute register index for later closing
    vm.to_be_closed.push((reg_idx, value));

    Ok(())
}
