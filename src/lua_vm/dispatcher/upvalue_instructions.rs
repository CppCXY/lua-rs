/// Upvalue and closure operations
/// 
/// These instructions handle upvalues, closures, and variable captures.

use crate::lua_vm::{LuaVM, LuaResult, LuaError, Instruction};
use super::DispatchAction;

/// GETUPVAL A B
/// R[A] := UpValue[B]
pub fn exec_getupval(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();

    let upvalue = func_ref.upvalues.get(b).ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid upvalue index: {}", b))
    })?;

    let value = upvalue.get_value(&vm.frames, &vm.register_stack);
    vm.register_stack[base_ptr + a] = value;

    Ok(DispatchAction::Continue)
}

/// SETUPVAL A B
/// UpValue[B] := R[A]
pub fn exec_setupval(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    
    let upvalue = func_ref.upvalues.get(b).ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid upvalue index: {}", b))
    })?;

    let value = vm.register_stack[base_ptr + a];
    
    upvalue.set_value(&mut vm.frames, &mut vm.register_stack, value);

    Ok(DispatchAction::Continue)
}

/// CLOSE A
/// close all upvalues >= R[A]
pub fn exec_close(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    let close_from = base_ptr + a;
    
    vm.close_upvalues_from(close_from);
    
    Ok(DispatchAction::Continue)
}

/// CLOSURE A Bx
/// R[A] := closure(KPROTO[Bx])
pub fn exec_closure(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();

    let proto = func_ref.chunk.child_protos.get(bx).ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid prototype index: {}", bx))
    })?.clone();

    // Get upvalue descriptors from the prototype
    let upvalue_descs = proto.upvalue_descs.clone();
    drop(func_ref);
    
    // Create upvalues for the new closure based on descriptors
    let mut upvalues = Vec::new();
    let mut new_open_upvalues = Vec::new();
    
    for (_i, desc) in upvalue_descs.iter().enumerate() {
        if desc.is_local {
            // Upvalue refers to a register in current function
            
            // Check if this upvalue is already open
            let existing_index = vm.open_upvalues.iter()
                .position(|uv| uv.points_to(frame.frame_id, desc.index as usize));
            
            if let Some(idx) = existing_index {
                upvalues.push(vm.open_upvalues[idx].clone());
            } else {
                // Create new open upvalue
                let new_uv = crate::lua_vm::LuaUpvalue::new_open(
                    frame.frame_id,
                    desc.index as usize
                );
                upvalues.push(new_uv.clone());
                new_open_upvalues.push(new_uv);
            }
        } else {
            // Upvalue refers to an upvalue in the enclosing function
            let parent_func = unsafe { &*func_ptr };
            let parent_upvalues = &parent_func.borrow().upvalues;
            
            if let Some(parent_uv) = parent_upvalues.get(desc.index as usize) {
                upvalues.push(parent_uv.clone());
            } else {
                return Err(LuaError::RuntimeError(
                    format!("Invalid upvalue index in parent: {}", desc.index)
                ));
            }
        }
    }
    
    // Add all new upvalues to the open list
    vm.open_upvalues.extend(new_open_upvalues);

    let closure = vm.create_function(proto, upvalues);
    vm.register_stack[base_ptr + a] = closure;

    Ok(DispatchAction::Continue)
}

/// VARARG A C
/// R[A], R[A+1], ..., R[A+C-2] = vararg
pub fn exec_vararg(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
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
                crate::LuaValue::nil()
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
                crate::LuaValue::nil()
            };
            vm.register_stack[base_ptr + a + i] = value;
        }
    }

    Ok(DispatchAction::Continue)
}

/// CONCAT A B
/// R[A] := R[A].. ... ..R[A+B]
pub fn exec_concat(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    use crate::lua_value::LuaValue;
    
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Helper function to convert value to string
    fn to_concat_string(value: LuaValue) -> Option<String> {
        if let Some(s) = value.as_lua_string() {
            return Some(s.as_str().to_string());
        } else if let Some(i) = value.as_integer() {
            return Some(i.to_string());
        } else if let Some(f) = value.as_number() {
            return Some(f.to_string());
        }
        None
    }

    // Concatenate values from R[A] to R[A+B]
    let mut result_value = vm.register_stack[base_ptr + a];
    
    for i in 1..=b {
        let next_value = vm.register_stack[base_ptr + a + i];
        
        // Try direct concatenation first
        let left_str = to_concat_string(result_value);
        let right_str = to_concat_string(next_value);
        
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
                        if let Some(mm_result) = vm.call_metamethod(&metamethod, &[result_value, next_value])? {
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
                            if let Some(mm_result) = vm.call_metamethod(&metamethod, &[result_value, next_value])? {
                                result_value = mm_result;
                                found_metamethod = true;
                            }
                        }
                    }
                }
            }
            
            if !found_metamethod {
                return Err(LuaError::RuntimeError(format!(
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

    Ok(DispatchAction::Continue)
}

/// SETLIST A B C k
/// R[A][C+i] := R[A+i], 1 <= i <= B
pub fn exec_setlist(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
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

    for i in 0..count {
        let key = crate::LuaValue::integer((start_idx + i) as i64);
        let value = vm.register_stack[base_ptr + a + i + 1];
        vm.table_set_with_meta(table, key, value)?;
    }

    Ok(DispatchAction::Continue)
}

/// TBC A
/// mark variable A as to-be-closed
/// This marks a variable to have its __close metamethod called when it goes out of scope
pub fn exec_tbc(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    // In a full implementation, we would:
    // 1. Mark the variable at R[A] as to-be-closed
    // 2. When the variable goes out of scope (block end, return, etc.),
    //    call its __close metamethod if it exists
    // 
    // For now, we just note the variable exists. The __close metamethod
    // should be called by:
    // - RETURN instruction (with k bit set)
    // - End of block (JMP with upvalue closing)
    // - Error unwinding
    
    // Check if the value has a __close metamethod (optional validation)
    let value = vm.register_stack[base_ptr + a];
    if !value.is_nil() {
        // We could check for __close metamethod here, but Lua allows
        // any value to be marked as to-be-closed
    }
    
    Ok(DispatchAction::Continue)
}
