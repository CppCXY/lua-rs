use crate::lua_value::LuaValueKind;
/// Control flow instructions
/// 
/// These instructions handle function calls, returns, jumps, and coroutine operations.

use crate::LuaValue;
use crate::lua_vm::{Instruction, LuaCallFrame, LuaError, LuaResult, LuaVM};
use super::DispatchAction;

/// RETURN A B C k
/// return R[A], ... ,R[A+B-2]
pub fn exec_return(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let _c = Instruction::get_c(instr) as usize;
    let _k = Instruction::get_k(instr);

    let frame = vm.frames.pop().ok_or_else(|| {
        LuaError::RuntimeError("RETURN with no frame on stack".to_string())
    })?;

    let base_ptr = frame.base_ptr;
    // CRITICAL: result_reg and num_results are stored in the CALLED frame, not the caller!
    // They tell us where to place return values in the CALLER's register space.
    let result_reg = frame.get_result_reg();
    let num_results = frame.get_num_results();

    // Collect return values
    vm.return_values.clear();

    if b == 0 {
        // Return all values from R[A] to top of stack
        let top = frame.top;
        for i in a..top {
            if base_ptr + i < vm.register_stack.len() {
                vm.return_values
                    .push(vm.register_stack[base_ptr + i]);
            }
        }
    } else {
        // Return b-1 values
        let count = b - 1;
        for i in 0..count {
            if base_ptr + a + i < vm.register_stack.len() {
                vm.return_values
                    .push(vm.register_stack[base_ptr + a + i]);
            }
        }
    }

    // If there are more frames, place return values in the caller's registers
    if !vm.frames.is_empty() {
        let caller_frame = vm.current_frame();
        let caller_base = caller_frame.base_ptr;
        
        // Copy return values to result registers
        let actual_returns = if num_results == usize::MAX {
            // Multiple return values expected - copy all
            for (i, value) in vm.return_values.iter().enumerate() {
                vm.register_stack[caller_base + result_reg + i] = *value;
            }
            vm.return_values.len()
        } else {
            // Fixed number of return values
            for i in 0..num_results {
                let value = vm.return_values.get(i).copied().unwrap_or(LuaValue::nil());
                vm.register_stack[caller_base + result_reg + i] = value;
            }
            num_results
        };
        
        // CRITICAL: Update caller's top to point after the return values
        // This is how Lua communicates variable return counts to the next instruction
        // (e.g., CALL with B=0 will use this top to determine argument count)
        vm.current_frame_mut().top = result_reg + actual_returns;
    }

    // Handle upvalue closing (k bit)
    // k=1 means close upvalues >= R[A]
    if _k {
        let close_from = base_ptr + a;
        vm.close_upvalues_from(close_from);
    }

    Ok(DispatchAction::Return)
}

// ============ Jump Instructions ============

/// JMP sJ
/// pc += sJ
pub fn exec_jmp(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let sj = Instruction::get_sj(instr);
    
    let frame = vm.current_frame_mut();
    // PC already incremented by dispatcher, so we add offset directly
    frame.pc = (frame.pc as i32 + sj) as usize;
    
    Ok(DispatchAction::Continue)
}

// ============ Test Instructions ============

/// TEST A k
/// if (not R[A] == k) then pc++
pub fn exec_test(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + a];
    
    // Lua truthiness: nil and false are falsy, everything else is truthy
    let is_truthy = !value.is_nil() && value.as_bool().unwrap_or(true);
    
    // If (not value) == k, skip next instruction
    if !is_truthy == k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// TESTSET A B k
/// if (not R[B] == k) then R[A] := R[B] else pc++
pub fn exec_testset(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];
    
    // Lua truthiness: not l_isfalse(v) means v is truthy
    let is_truthy = !value.is_nil() && value.as_bool().unwrap_or(true);
    
    // TESTSET: if ((not l_isfalse(R[B])) == k) then R[A] := R[B] else pc++
    // If (is_truthy == k), assign R[A] = R[B], otherwise skip next instruction
    if is_truthy == k {
        vm.register_stack[base_ptr + a] = value;
    } else {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

// ============ Comparison Instructions ============

/// EQ A B k
/// if ((R[A] == R[B]) ~= k) then pc++
pub fn exec_eq(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + a];
    let right = vm.register_stack[base_ptr + b];

    let is_equal = left == right;
    
    // If (left == right) != k, skip next instruction
    if is_equal != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// LT A B k
/// if ((R[A] < R[B]) ~= k) then pc++
pub fn exec_lt(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + a];
    let right = vm.register_stack[base_ptr + b];

    let is_less = if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
        l < r
    } else if let (Some(l), Some(r)) = (left.as_number(), right.as_number()) {
        l < r
    } else if left.is_string() && right.is_string() {
        // String comparison - would need VM to get strings
        // For now, just use default comparison
        left < right
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with {}",
            left.type_name(),
            right.type_name()
        )));
    };
    
    if is_less != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// LE A B k
/// if ((R[A] <= R[B]) ~= k) then pc++
pub fn exec_le(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + a];
    let right = vm.register_stack[base_ptr + b];

    let is_less_equal = if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
        l <= r
    } else if let (Some(l), Some(r)) = (left.as_number(), right.as_number()) {
        l <= r
    } else if left.is_string() && right.is_string() {
        left <= right
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with {}",
            left.type_name(),
            right.type_name()
        )));
    };
    
    if is_less_equal != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// EQK A B k
/// if ((R[A] == K[B]) ~= k) then pc++
pub fn exec_eqk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let constant = func.borrow().chunk.constants.get(b).copied().ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid constant index: {}", b))
    })?;

    let left = vm.register_stack[base_ptr + a];

    let is_equal = left == constant;
    
    if is_equal != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// EQI A sB k
/// if ((R[A] == sB) ~= k) then pc++
pub fn exec_eqi(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + a];

    let is_equal = if let Some(l) = left.as_integer() {
        l == sb as i64
    } else if let Some(l) = left.as_number() {
        l == sb as f64
    } else {
        false
    };
    
    if is_equal != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// LTI A sB k
/// if ((R[A] < sB) ~= k) then pc++
pub fn exec_lti(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + a];

    let is_less = if let Some(l) = left.as_integer() {
        l < sb as i64
    } else if let Some(l) = left.as_number() {
        l < sb as f64
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with number",
            left.type_name()
        )));
    };
    
    if is_less != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// LEI A sB k
/// if ((R[A] <= sB) ~= k) then pc++
pub fn exec_lei(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + a];

    let is_less_equal = if let Some(l) = left.as_integer() {
        l <= sb as i64
    } else if let Some(l) = left.as_number() {
        l <= sb as f64
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with number",
            left.type_name()
        )));
    };
    
    if is_less_equal != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// GTI A sB k
/// if ((R[A] > sB) ~= k) then pc++
pub fn exec_gti(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + a];

    let is_greater = if let Some(l) = left.as_integer() {
        l > sb as i64
    } else if let Some(l) = left.as_number() {
        l > sb as f64
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with number",
            left.type_name()
        )));
    };
    
    if is_greater != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// GEI A sB k
/// if ((R[A] >= sB) ~= k) then pc++
pub fn exec_gei(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + a];

    let is_greater_equal = if let Some(l) = left.as_integer() {
        l >= sb as i64
    } else if let Some(l) = left.as_number() {
        l >= sb as f64
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with number",
            left.type_name()
        )));
    };
    
    if is_greater_equal != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

// ============ Call Instructions ============

/// CALL A B C
/// R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
pub fn exec_call(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    use crate::lua_value::LuaValueKind;
    use crate::lua_vm::LuaCallFrame;
    
    // CALL A B C: R[A], ..., R[A+C-2] := R[A](R[A+1], ..., R[A+B-1])
    // A: function register, B: arg count + 1 (0 = use top), C: return count + 1 (0 = use top)
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.frames.last().unwrap();
    let base = frame.base_ptr;

    // Get function from R[A]
    let func = vm.register_stack[base + a];

    // Determine argument count
    // CRITICAL: In Lua, when B != 0, CALL sets its own top (doesn't use frame.top)
    // When B == 0, it uses the top set by the previous instruction (e.g., another CALL)
    let arg_count = if b == 0 {
        // Use all values from R[A+1] to current top
        // This top was set by the previous instruction (usually a CALL with C=0)
        let frame = vm.current_frame();
        if frame.top > a + 1 {
            frame.top - (a + 1)
        } else {
            0
        }
    } else {
        // Fixed argument count: B-1
        // We IGNORE frame.top here - B specifies the exact number of arguments
        // AND we update frame.top to reflect the call boundary
        // This matches Lua's: L->top.p = ra + b
        vm.current_frame_mut().top = a + b;
        b - 1
    };

    // Determine expected return count
    let return_count = if c == 0 {
        usize::MAX // Want all return values
    } else {
        c - 1
    };

    match func.kind() {
        LuaValueKind::CFunction => {
            
            // Call C function immediately
            let cfunc = func.as_cfunction().unwrap();
            
            // Create temporary frame for CFunction
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;
            
            // Set up call arguments in a new stack segment
            let call_base = vm.register_stack.len();
            vm.ensure_stack_capacity(call_base + arg_count + 1);
            
            // Copy function and arguments
            vm.register_stack[call_base] = func;
            for i in 0..arg_count {
                vm.register_stack[call_base + i + 1] = vm.register_stack[base + a + 1 + i];
            }
            
            let temp_frame = LuaCallFrame::new_c_function(
                frame_id,
                vm.current_frame().function_value,
                vm.current_frame().pc,
                call_base,
                arg_count + 1,
            );
            
            vm.frames.push(temp_frame);
            let result = match cfunc(vm) {
                Ok(r) => Ok(r),
                Err(LuaError::Yield(values)) => {
                    // CFunction yielded - pop the temporary frame before yielding
                    vm.frames.pop();
                    return Err(LuaError::Yield(values));
                }
                Err(e) => {
                    vm.frames.pop();
                    return Err(e);
                }
            };
            vm.frames.pop();
            let result = result?;
            
            // Store return values
            let values = result.all_values();
            let num_returns = if return_count == usize::MAX {
                values.len()
            } else {
                return_count
            };
            
            for i in 0..num_returns {
                let value = values.get(i).copied().unwrap_or(crate::LuaValue::nil());
                vm.register_stack[base + a + i] = value;
            }
            
            // CRITICAL: Update caller's top to indicate how many values were returned
            // This is essential for variable returns (C=0) so the next instruction knows
            // how many values are available (e.g., CALL with B=0)
            vm.current_frame_mut().top = a + num_returns;
            
            Ok(DispatchAction::Continue)
        },
        LuaValueKind::Function => {
            // Get max_stack_size from function before creating frame
            let func_id = func.as_function_id().unwrap();
            let max_stack_size = match vm.object_pool.get_function(func_id) {
                Some(func_ref) => {
                    let size = func_ref.borrow().chunk.max_stack_size;
                    // Ensure at least 1 register for function body
                    if size == 0 { 1 } else { size }
                },
                None => {
                    return Err(LuaError::RuntimeError("Invalid function reference".to_string()));
                }
            };

            // Create new frame
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;

            // Ensure source argument registers are accessible
            let max_src_reg = base + a + 1 + arg_count;
            if max_src_reg > vm.register_stack.len() {
                vm.ensure_stack_capacity(max_src_reg);
            }

            let new_base = vm.register_stack.len();
            // Ensure new frame can hold at least the arguments
            let actual_stack_size = max_stack_size.max(arg_count);
            
            // For vararg functions, we need extra space to store varargs beyond max_stack_size
            // Check if this is a vararg function by looking at the chunk
            let is_vararg = match vm.object_pool.get_function(func_id) {
                Some(func_ref) => func_ref.borrow().chunk.is_vararg,
                None => false,
            };
            
            let total_stack_size = if is_vararg && arg_count > 0 {
                // Allocate space for: max_stack_size + varargs
                actual_stack_size + arg_count
            } else {
                actual_stack_size
            };
            
            vm.ensure_stack_capacity(new_base + total_stack_size);

            // Initialize registers with nil
            for i in 0..total_stack_size {
                vm.register_stack[new_base + i] = crate::LuaValue::nil();
            }

            if is_vararg && arg_count > 0 {
                // For vararg functions, copy arguments BEYOND max_stack_size
                // so they won't be overwritten by local variables
                let vararg_base = new_base + actual_stack_size;
                for i in 0..arg_count {
                    vm.register_stack[vararg_base + i] = vm.register_stack[base + a + 1 + i];
                }
            } else {
                // Regular function: copy arguments to R[0], R[1], ...
                for i in 0..arg_count {
                    vm.register_stack[new_base + i] = vm.register_stack[base + a + 1 + i];
                }
            }

            // Create and push new frame
            // IMPORTANT: For vararg functions, top should reflect actual arg count, not max_stack_size
            // VARARGPREP will use this to determine the number of varargs
            let new_frame = LuaCallFrame::new_lua_function(
                frame_id,
                func,
                new_base,
                arg_count, // top = number of arguments passed
                a, // result_reg: where to store return values
                return_count,
            );
            vm.frames.push(new_frame);

            Ok(DispatchAction::Call)
        },
        _ => {
            Err(LuaError::RuntimeError(format!(
                "attempt to call a {} value",
                func.type_name()
            )))
        }
    }
}

/// TAILCALL A B C k
/// return R[A](R[A+1], ... ,R[A+B-1])
pub fn exec_tailcall(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    
    // TAILCALL A B C: return R[A](R[A+1], ..., R[A+B-1])
    // Reuse current frame (tail call optimization)
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.frames.last().unwrap();
    let base = frame.base_ptr;

    // Get function from R[A]
    let func = vm.register_stack[base + a];

    // Determine argument count
    let arg_count = if b == 0 {
        // Use all values from R[A+1] to top
        let frame = vm.current_frame();
        frame.top - (base + a + 1)
    } else {
        b - 1
    };

    // Copy arguments to temporary buffer
    let mut args = Vec::with_capacity(arg_count);
    for i in 0..arg_count {
        args.push(vm.register_stack[base + a + 1 + i]);
    }

    // Pop current frame (tail call optimization)
    let old_base = frame.base_ptr;
    let return_count = frame.get_num_results();
    vm.frames.pop();

    // Get max_stack_size from function
    let max_stack_size = match func.kind() {
        LuaValueKind::Function => {
            let func_id = func.as_function_id().unwrap();
            match vm.object_pool.get_function(func_id) {
                Some(func_ref) => func_ref.borrow().chunk.max_stack_size,
                None => {
                    return Err(LuaError::RuntimeError("Invalid function reference".to_string()));
                }
            }
        }
        LuaValueKind::CFunction => 256,
        _ => {
            return Err(LuaError::RuntimeError(format!(
                "attempt to call a {} value",
                func.type_name()
            )));
        }
    };

    // Create new frame at same location
    let frame_id = vm.next_frame_id;
    vm.next_frame_id += 1;

    vm.ensure_stack_capacity(old_base + max_stack_size);

    // Copy arguments to frame base
    for (i, arg) in args.iter().enumerate() {
        vm.register_stack[old_base + i] = *arg;
    }

    let new_frame = LuaCallFrame::new_lua_function(
        frame_id,
        func,
        old_base,
        max_stack_size,
        0,
        return_count,
    );
    vm.frames.push(new_frame);

    Ok(DispatchAction::Call)
}

/// RETURN0
/// return (no values)
pub fn exec_return0(vm: &mut LuaVM, _instr: u32) -> LuaResult<DispatchAction> {
    let frame = vm.frames.pop().ok_or_else(|| {
        LuaError::RuntimeError("RETURN0 with no frame on stack".to_string())
    })?;

    vm.return_values.clear();
    
    // Update caller's top to indicate 0 return values
    if !vm.frames.is_empty() {
        let result_reg = frame.get_result_reg();
        vm.current_frame_mut().top = result_reg; // No return values, so top = result_reg + 0
    }
    
    Ok(DispatchAction::Return)
}

/// RETURN1 A
/// return R[A]
pub fn exec_return1(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;

    let frame = vm.frames.pop().ok_or_else(|| {
        LuaError::RuntimeError("RETURN1 with no frame on stack".to_string())
    })?;

    let base_ptr = frame.base_ptr;
    let result_reg = frame.get_result_reg();

    vm.return_values.clear();
    if base_ptr + a < vm.register_stack.len() {
        vm.return_values.push(vm.register_stack[base_ptr + a]);
    }
    
    // Copy return value to caller's registers if needed
    if !vm.frames.is_empty() {
        let caller_base = vm.current_frame().base_ptr;

        if !vm.return_values.is_empty() {
            vm.register_stack[caller_base + result_reg] = vm.return_values[0];
        }
        
        // Update caller's top to indicate 1 return value
        vm.current_frame_mut().top = result_reg + 1;
    }

    Ok(DispatchAction::Return)
}
