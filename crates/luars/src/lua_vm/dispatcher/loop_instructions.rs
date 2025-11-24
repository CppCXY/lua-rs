/// Loop instructions
///
/// These instructions handle for loops (numeric and generic iterators).
use crate::{
    LuaValue,
    lua_value::{LuaValueKind, TAG_FLOAT, TAG_INTEGER, TYPE_MASK},
    lua_vm::{Instruction, LuaCallFrame, LuaError, LuaResult, LuaVM},
};

/// FORPREP A Bx
/// Prepare numeric for loop: R[A]-=R[A+2]; R[A+3]=R[A]; if (skip) pc+=Bx+1
#[inline(always)]
pub fn exec_forprep(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
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
        let limit_f = limit
            .as_number()
            .ok_or_else(|| LuaError::RuntimeError("'for' limit must be a number".to_string()))?;
        let step_f = step
            .as_number()
            .ok_or_else(|| LuaError::RuntimeError("'for' step must be a number".to_string()))?;

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

    Ok(())
}

/// FORLOOP A Bx
/// R[A]+=R[A+2];
/// if R[A] <?= R[A+1] then { pc-=Bx; R[A+3]=R[A] }
///
/// ULTRA-OPTIMIZED V2: Cache frame pointer + direct bit-mask type checking
/// - Eliminate Vec::len() call by using last_mut() directly
/// - Single type check for all 3 values (branchless fast path)
/// - Zero function calls in hot path
#[inline(always)]
pub fn exec_forloop(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    // OPTIMIZATION: Single unsafe block + direct bit-mask type checking
    // Use last_mut() to avoid len() call
    unsafe {
        let frame_ptr = vm.frames.last_mut().unwrap_unchecked() as *mut LuaCallFrame;
        let base_ptr = (*frame_ptr).base_ptr;
        let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr + a);

        // Load all 3 values
        let idx = *reg_base;
        let counter_or_limit = *reg_base.add(1);
        let step = *reg_base.add(2);

        // OPTIMIZATION: Combined type check - single OR operation to detect if all are integers
        // If all 3 are TAG_INTEGER, then (a|b|c) & TYPE_MASK == TAG_INTEGER
        let combined_tags = (idx.primary | counter_or_limit.primary | step.primary) & TYPE_MASK;

        // Fast path: All integers (single branch!)
        if combined_tags == TAG_INTEGER {
            let count = counter_or_limit.secondary as i64;

            if count > 0 {
                let idx_i = idx.secondary as i64;
                let step_i = step.secondary as i64;
                let new_idx = idx_i.wrapping_add(step_i);

                // Update registers
                *reg_base = LuaValue::integer(new_idx);
                *reg_base.add(1) = LuaValue::integer(count - 1);
                *reg_base.add(3) = LuaValue::integer(new_idx);

                (*frame_ptr).pc -= bx;
            }
        }
        // Slow path: at least one non-integer
        else {
            let step_tag = step.primary & TYPE_MASK;
            let counter_tag = counter_or_limit.primary & TYPE_MASK;
            let idx_tag = idx.primary & TYPE_MASK;

            // Check if all are numbers (integer or float)
            if (step_tag == TAG_FLOAT || step_tag == TAG_INTEGER)
                && (counter_tag == TAG_FLOAT || counter_tag == TAG_INTEGER)
                && (idx_tag == TAG_FLOAT || idx_tag == TAG_INTEGER)
            {
                // Convert to float
                let idx_f = if idx_tag == TAG_FLOAT {
                    f64::from_bits(idx.secondary)
                } else {
                    idx.secondary as i64 as f64
                };

                let limit_f = if counter_tag == TAG_FLOAT {
                    f64::from_bits(counter_or_limit.secondary)
                } else {
                    counter_or_limit.secondary as i64 as f64
                };

                let step_f = if step_tag == TAG_FLOAT {
                    f64::from_bits(step.secondary)
                } else {
                    step.secondary as i64 as f64
                };

                let new_idx_f = idx_f + step_f;
                let should_continue = if step_f > 0.0 {
                    new_idx_f <= limit_f
                } else {
                    new_idx_f >= limit_f
                };

                if should_continue {
                    *reg_base = LuaValue::number(new_idx_f);
                    *reg_base.add(3) = LuaValue::number(new_idx_f);
                    (*frame_ptr).pc -= bx;
                }
            } else {
                return Err(LuaError::RuntimeError(
                    "'for' values must be numbers".to_string(),
                ));
            }
        }
    }

    Ok(())
}

/// TFORPREP A Bx
/// create upvalue for R[A + 3]; pc+=Bx
/// In Lua 5.4, this creates a to-be-closed variable for the state
pub fn exec_tforprep(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
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

    Ok(())
}

/// TFORCALL A C
/// R[A+4], ... ,R[A+3+C] := R[A](R[A+1], R[A+2]);
pub fn exec_tforcall(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
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
            let cfunc = func
                .as_cfunction()
                .ok_or_else(|| LuaError::RuntimeError("Invalid CFunction".to_string()))?;

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
        }
        LuaValueKind::Function => {
            // For Lua functions, we need to use CALL instruction logic
            // Set up registers for the call
            vm.register_stack[base_ptr + a + 3] = func;
            vm.register_stack[base_ptr + a + 4] = state;
            vm.register_stack[base_ptr + a + 5] = control;

            // OPTIMIZATION: Use direct pointer access instead of hash lookup
            let func_ptr = func
                .as_function_ptr()
                .ok_or_else(|| LuaError::RuntimeError("Invalid function pointer".to_string()))?;

            let max_stack_size = unsafe { (*func_ptr).borrow().chunk.max_stack_size };

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

            // Get code pointer from function
            let func_ptr = func
                .as_function_ptr()
                .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
            let func_obj = unsafe { &*func_ptr };
            let code_ptr = func_obj.borrow().chunk.code.as_ptr();

            let new_frame = LuaCallFrame::new_lua_function(
                frame_id,
                func,
                code_ptr,
                call_base,
                max_stack_size,
                a + 3, // result goes to R[A+3]
                c + 1, // expecting c+1 results
            );

            vm.frames.push(new_frame);
            // Execution will continue in the new frame
        }
        _ => {
            return Err(LuaError::RuntimeError(
                "attempt to call a non-function value in for loop".to_string(),
            ));
        }
    }

    Ok(())
}

/// TFORLOOP A Bx
/// if R[A+1] ~= nil then { R[A]=R[A+1]; pc -= Bx }
pub fn exec_tforloop(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
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

    Ok(())
}
