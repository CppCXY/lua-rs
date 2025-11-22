use super::DispatchAction;
/// Loop instructions
///
/// These instructions handle for loops (numeric and generic iterators).
use crate::{
    LuaValue,
    lua_value::LuaValueKind,
    lua_vm::{Instruction, LuaCallFrame, LuaError, LuaResult, LuaVM},
};

/// FORPREP A Bx
/// Prepare numeric for loop: R[A]-=R[A+2]; R[A+3]=R[A]; if (skip) pc+=Bx+1
pub fn exec_forprep(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let init = vm.register_stack[base_ptr + a];
    let limit = vm.register_stack[base_ptr + a + 1];
    let step = vm.register_stack[base_ptr + a + 2];

    // Check for integer loop
    if let (Some(init_i), Some(limit_i), Some(step_i)) = 
        (init.as_integer(), limit.as_integer(), step.as_integer())
    {
        if step_i == 0 {
            return Err(LuaError::RuntimeError("'for' step is zero".to_string()));
        }

        // Set control variable (R[A+3] = init)
        vm.register_stack[base_ptr + a + 3] = LuaValue::integer(init_i);

        // Calculate loop count (Lua 5.4 uses counter for integer loops)
        let count = if step_i > 0 {
            // Ascending: count = (limit - init) / step
            if limit_i < init_i {
                0 // skip loop
            } else {
                let diff = (limit_i as i128) - (init_i as i128);
                (diff / (step_i as i128)) as u64
            }
        } else {
            // Descending: count = (init - limit) / (-(step+1)+1)
            if init_i < limit_i {
                0 // skip loop
            } else {
                let diff = (init_i as i128) - (limit_i as i128);
                let divisor = -((step_i + 1) as i128) + 1;
                (diff / divisor) as u64
            }
        };

        if count == 0 {
            // Skip the entire loop body and FORLOOP
            vm.current_frame_mut().pc = vm.current_frame().pc + bx;
        } else {
            // Store count in R[A+1] (replacing limit)
            vm.register_stack[base_ptr + a + 1] = LuaValue::integer(count as i64);
            // R[A] keeps init value (will be updated by FORLOOP)
            // Don't modify R[A] here!
        }
    } else {
        // Float loop
        let init_f = init.as_number().ok_or_else(|| {
            LuaError::RuntimeError("'for' initial value must be a number".to_string())
        })?;
        let limit_f = limit.as_number().ok_or_else(|| {
            LuaError::RuntimeError("'for' limit must be a number".to_string())
        })?;
        let step_f = step.as_number().ok_or_else(|| {
            LuaError::RuntimeError("'for' step must be a number".to_string())
        })?;

        if step_f == 0.0 {
            return Err(LuaError::RuntimeError("'for' step is zero".to_string()));
        }

        // Set control variable
        vm.register_stack[base_ptr + a + 3] = LuaValue::number(init_f);

        // Check if we should skip
        let should_skip = if step_f > 0.0 {
            init_f > limit_f
        } else {
            init_f < limit_f
        };

        if should_skip {
            vm.current_frame_mut().pc = vm.current_frame().pc + bx;
        } else {
            // Prepare internal index
            vm.register_stack[base_ptr + a] = LuaValue::number(init_f - step_f);
        }
    }

    Ok(DispatchAction::Continue)
}

/// FORLOOP A Bx
/// R[A]+=R[A+2];
/// if R[A] <?= R[A+1] then { pc-=Bx; R[A+3]=R[A] }
pub fn exec_forloop(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access (hot path)
    // Safety: FORPREP guarantees these registers exist and are initialized
    let (idx, counter_or_limit, step) = unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr + a);
        (*reg_base, *reg_base.add(1), *reg_base.add(2))
    };

    // Check if this is an integer loop (step is integer)
    if let Some(step_i) = step.as_integer() {
        // Integer loop: R[A+1] is a counter
        // OPTIMIZATION: Use match instead of ok_or_else to avoid closure
        let count = match counter_or_limit.as_integer() {
            Some(c) => c,
            None => return Err(LuaError::RuntimeError("'for' counter must be a number".to_string())),
        };

        if count > 0 {
            // Update internal index
            let idx_i = match idx.as_integer() {
                Some(i) => i,
                None => return Err(LuaError::RuntimeError("'for' index must be a number".to_string())),
            };
            let new_idx = idx_i.wrapping_add(step_i);
            
            // OPTIMIZATION: Use unsafe for unchecked writes (hot path)
            // Safety: Same registers we just read from, still valid
            unsafe {
                let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr + a);
                *reg_base = LuaValue::integer(new_idx);
                *reg_base.add(1) = LuaValue::integer(count - 1);
                *reg_base.add(3) = LuaValue::integer(new_idx);
            }

            // OPTIMIZATION: Direct PC manipulation
            let pc = vm.current_frame().pc;
            vm.current_frame_mut().pc = pc - bx;
        }
        // If count <= 0, exit loop (don't jump)
    } else {
        // Float loop: R[A+1] is limit, use traditional comparison
        let limit_f = match counter_or_limit.as_number() {
            Some(l) => l,
            None => return Err(LuaError::RuntimeError("'for' limit must be a number".to_string())),
        };
        let idx_f = match idx.as_number() {
            Some(i) => i,
            None => return Err(LuaError::RuntimeError("'for' index must be a number".to_string())),
        };
        let step_f = match step.as_number() {
            Some(s) => s,
            None => return Err(LuaError::RuntimeError("'for' step must be a number".to_string())),
        };

        // Add step to index
        let new_idx_f = idx_f + step_f;

        // Check condition
        let should_continue = if step_f > 0.0 {
            new_idx_f <= limit_f
        } else {
            new_idx_f >= limit_f
        };

        if should_continue {
            // OPTIMIZATION: Unsafe writes for float path too
            unsafe {
                let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr + a);
                *reg_base = LuaValue::number(new_idx_f);
                *reg_base.add(3) = LuaValue::number(new_idx_f);
            }
            
            let pc = vm.current_frame().pc;
            vm.current_frame_mut().pc = pc - bx;
        }
    }

    Ok(DispatchAction::Continue)
}

/// TFORPREP A Bx
/// create upvalue for R[A + 3]; pc+=Bx
/// In Lua 5.4, this creates a to-be-closed variable for the state
pub fn exec_tforprep(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    // In Lua 5.4, R[A+3] is the to-be-closed variable for the state
    // For now, we just copy the state value to ensure it's preserved
    let state = vm.register_stack[base_ptr + a + 1];
    vm.register_stack[base_ptr + a + 3] = state;

    // Jump to loop start
    vm.current_frame_mut().pc += bx;

    Ok(DispatchAction::Continue)
}

/// TFORCALL A C
/// R[A+4], ... ,R[A+3+C] := R[A](R[A+1], R[A+2]);
pub fn exec_tforcall(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get iterator function and state
    let func = vm.register_stack[base_ptr + a];
    let state = vm.register_stack[base_ptr + a + 1];
    let control = vm.register_stack[base_ptr + a + 2];

    // Call func(state, control)
    // This is similar to CALL instruction but with fixed arguments
    match func.kind() {
        LuaValueKind::CFunction => {
            let cfunc = func.as_cfunction().ok_or_else(|| {
                LuaError::RuntimeError("Invalid CFunction".to_string())
            })?;
            
            // Create temporary frame for the call
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;
            
            // Set up call stack: func, state, control
            let call_base = base_ptr + a + 3;
            vm.register_stack[call_base] = func;
            vm.register_stack[call_base + 1] = state;
            vm.register_stack[call_base + 2] = control;
            
            let temp_frame = LuaCallFrame::new_c_function(
                frame_id,
                vm.current_frame().function_value,
                vm.current_frame().pc,
                call_base,
                3, // func + 2 args
            );
            
            vm.frames.push(temp_frame);
            let result = cfunc(vm)?;
            vm.frames.pop();
            
            // Store results starting at R[A+3]
            let values = result.all_values();
            for (i, value) in values.iter().enumerate().take(c + 1) {
                vm.register_stack[base_ptr + a + 3 + i] = *value;
            }
            // Fill remaining with nil
            for i in values.len()..=c {
                vm.register_stack[base_ptr + a + 3 + i] = LuaValue::nil();
            }
        },
        LuaValueKind::Function => {
            // For Lua functions, we need to use CALL instruction logic
            // Set up registers for the call
            vm.register_stack[base_ptr + a + 3] = func;
            vm.register_stack[base_ptr + a + 4] = state;
            vm.register_stack[base_ptr + a + 5] = control;
            
            // Create the call frame
            let func_id = func.as_function_id().ok_or_else(|| {
                LuaError::RuntimeError("Invalid function".to_string())
            })?;
            
            let max_stack_size = {
                let func_ref = vm.object_pool.get_function(func_id)
                    .ok_or_else(|| LuaError::RuntimeError("Invalid function".to_string()))?;
                func_ref.borrow().chunk.max_stack_size
            };
            
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;
            
            let call_base = vm.register_stack.len();
            vm.ensure_stack_capacity(call_base + max_stack_size);
            
            // Initialize registers
            for i in 0..max_stack_size {
                vm.register_stack[call_base + i] = LuaValue::nil();
            }
            
            // Copy arguments
            vm.register_stack[call_base] = state;
            vm.register_stack[call_base + 1] = control;
            
            let new_frame = LuaCallFrame::new_lua_function(
                frame_id,
                func,
                call_base,
                max_stack_size,
                a + 3, // result goes to R[A+3]
                c + 1, // expecting c+1 results
            );
            
            vm.frames.push(new_frame);
            // Execution will continue in the new frame
        },
        _ => {
            return Err(LuaError::RuntimeError(
                "attempt to call a non-function value in for loop".to_string()
            ));
        }
    }

    Ok(DispatchAction::Continue)
}

/// TFORLOOP A Bx
/// if R[A+1] ~= nil then { R[A]=R[A+1]; pc -= Bx }
pub fn exec_tforloop(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + a + 1];

    if !value.is_nil() {
        // Continue loop
        vm.register_stack[base_ptr + a] = value;
        vm.current_frame_mut().pc -= bx;
    }

    Ok(DispatchAction::Continue)
}
