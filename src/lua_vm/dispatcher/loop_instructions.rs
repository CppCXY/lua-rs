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
/// R[A]-=R[A+2]; pc+=Bx
pub fn exec_forprep(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let init = vm.register_stack[base_ptr + a];
    let step = vm.register_stack[base_ptr + a + 2];

    // Subtract step from init (preparation for first iteration)
    let result = if let (Some(i), Some(s)) = (init.as_integer(), step.as_integer()) {
        if let Some(diff) = i.checked_sub(s) {
            LuaValue::integer(diff)
        } else {
            LuaValue::number(i as f64 - s as f64)
        }
    } else {
        let init_f = init.as_number().ok_or_else(|| {
            LuaError::RuntimeError("'for' initial value must be a number".to_string())
        })?;
        let step_f = step
            .as_number()
            .ok_or_else(|| LuaError::RuntimeError("'for' step must be a number".to_string()))?;
        LuaValue::number(init_f - step_f)
    };

    vm.register_stack[base_ptr + a] = result;

    // Jump to loop start
    vm.current_frame_mut().pc += bx;

    Ok(DispatchAction::Continue)
}

/// FORLOOP A Bx
/// R[A]+=R[A+2];
/// if R[A] <?= R[A+1] then { pc-=Bx; R[A+3]=R[A] }
pub fn exec_forloop(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let idx = vm.register_stack[base_ptr + a];
    let limit = vm.register_stack[base_ptr + a + 1];
    let step = vm.register_stack[base_ptr + a + 2];

    // Add step to index
    let new_idx = if let (Some(i), Some(s)) = (idx.as_integer(), step.as_integer()) {
        if let Some(sum) = i.checked_add(s) {
            LuaValue::integer(sum)
        } else {
            LuaValue::number(i as f64 + s as f64)
        }
    } else {
        let idx_f = idx
            .as_number()
            .ok_or_else(|| LuaError::RuntimeError("'for' index must be a number".to_string()))?;
        let step_f = step
            .as_number()
            .ok_or_else(|| LuaError::RuntimeError("'for' step must be a number".to_string()))?;
        LuaValue::number(idx_f + step_f)
    };

    vm.register_stack[base_ptr + a] = new_idx;

    // Check loop condition
    let should_continue = if let (Some(i), Some(l), Some(s)) =
        (new_idx.as_integer(), limit.as_integer(), step.as_integer())
    {
        if s > 0 { i <= l } else { i >= l }
    } else {
        let idx_f = new_idx
            .as_number()
            .ok_or_else(|| LuaError::RuntimeError("'for' index must be a number".to_string()))?;
        let limit_f = limit
            .as_number()
            .ok_or_else(|| LuaError::RuntimeError("'for' limit must be a number".to_string()))?;
        let step_f = step
            .as_number()
            .ok_or_else(|| LuaError::RuntimeError("'for' step must be a number".to_string()))?;

        if step_f > 0.0 {
            idx_f <= limit_f
        } else {
            idx_f >= limit_f
        }
    };

    if should_continue {
        // Continue loop: set loop variable and jump back
        vm.register_stack[base_ptr + a + 3] = new_idx;
        vm.current_frame_mut().pc -= bx;
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
