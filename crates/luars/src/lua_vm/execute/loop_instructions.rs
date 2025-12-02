/// Loop instructions
///
/// These instructions handle for loops (numeric and generic iterators).
use crate::{
    LuaValue,
    lua_value::{LuaValueKind, TAG_FLOAT, TAG_INTEGER, TYPE_MASK},
    lua_vm::{Instruction, LuaCallFrame, LuaResult, LuaVM},
};

// Re-export LuaCallFrame for use with frame_ptr

/// FORPREP A Bx
/// Prepare numeric for loop: R[A]-=R[A+2]; R[A+3]=R[A]; if (skip) pc+=Bx+1
/// OPTIMIZED: Uses frame_ptr directly, no i128, unsafe register access
#[inline(always)]
pub fn exec_forprep(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    unsafe {
        let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr + a);

        let init = *reg_base;
        let limit = *reg_base.add(1);
        let step = *reg_base.add(2);

        // Check for integer loop using type tags
        let init_tag = init.primary & TYPE_MASK;
        let limit_tag = limit.primary & TYPE_MASK;
        let step_tag = step.primary & TYPE_MASK;

        if init_tag == TAG_INTEGER && limit_tag == TAG_INTEGER && step_tag == TAG_INTEGER {
            let init_i = init.secondary as i64;
            let limit_i = limit.secondary as i64;
            let step_i = step.secondary as i64;

            if step_i == 0 {
                return Err(vm.error("'for' step is zero".to_string()));
            }

            // Set control variable (R[A+3] = init)
            *reg_base.add(3) = LuaValue::integer(init_i);

            // Calculate loop count using i64 arithmetic (avoid i128!)
            // Lua 5.4 style: count = floor((limit - init) / step) + 1 if will execute
            let count: u64 = if step_i > 0 {
                // Ascending loop
                if init_i > limit_i {
                    0 // Won't execute at all
                } else {
                    // (limit - init) / step + 1, using unsigned division
                    let diff = (limit_i - init_i) as u64;
                    diff / (step_i as u64) + 1
                }
            } else {
                // Descending loop (step < 0)
                if init_i < limit_i {
                    0 // Won't execute at all
                } else {
                    // (init - limit) / (-step) + 1
                    let diff = (init_i - limit_i) as u64;
                    let neg_step = (-step_i) as u64;
                    diff / neg_step + 1
                }
            };

            if count == 0 {
                // Skip the entire loop body and FORLOOP
                *pc += bx;
            } else {
                // Store count-1 in R[A+1] (we'll execute count times, counter starts at count-1)
                // because we already set R[A+3] = init for the first iteration
                *reg_base.add(1) = LuaValue::integer((count - 1) as i64);
                // Set R[A] = init (internal index starts at init)
                *reg_base = LuaValue::integer(init_i);
            }
        } else {
            // Float loop - convert to f64
            let init_f = if init_tag == TAG_INTEGER {
                init.secondary as i64 as f64
            } else if init_tag == TAG_FLOAT {
                f64::from_bits(init.secondary)
            } else {
                return Err(vm.error("'for' initial value must be a number".to_string()));
            };

            let limit_f = if limit_tag == TAG_INTEGER {
                limit.secondary as i64 as f64
            } else if limit_tag == TAG_FLOAT {
                f64::from_bits(limit.secondary)
            } else {
                return Err(vm.error("'for' limit must be a number".to_string()));
            };

            let step_f = if step_tag == TAG_INTEGER {
                step.secondary as i64 as f64
            } else if step_tag == TAG_FLOAT {
                f64::from_bits(step.secondary)
            } else {
                return Err(vm.error("'for' step must be a number".to_string()));
            };

            if step_f == 0.0 {
                return Err(vm.error("'for' step is zero".to_string()));
            }

            // Set control variable
            *reg_base.add(3) = LuaValue::number(init_f);

            // Check if we should skip
            let should_skip = if step_f > 0.0 {
                init_f > limit_f
            } else {
                init_f < limit_f
            };

            if should_skip {
                *pc += bx;
            } else {
                // Prepare internal index
                *reg_base = LuaValue::number(init_f - step_f);
            }
        }
    }

    Ok(())
}

/// FORLOOP A Bx
/// R[A]+=R[A+2];
/// if R[A] <?= R[A+1] then { pc-=Bx; R[A+3]=R[A] }
///
/// ULTRA-OPTIMIZED: Only check step type (like Lua C), use chgivalue pattern
#[inline(always)]
#[allow(dead_code)]
pub fn exec_forloop(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    unsafe {
        let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr + a);

        // Only check step type - like Lua C's ttisinteger(s2v(ra + 2))
        let step = *reg_base.add(2);
        
        if step.primary == TAG_INTEGER {
            // Integer loop - step is integer, so idx and counter must be too (set by FORPREP)
            let count = (*reg_base.add(1)).secondary; // counter as u64
            
            if count > 0 {
                let idx = (*reg_base).secondary as i64;
                let step_i = step.secondary as i64;
                let new_idx = idx.wrapping_add(step_i);
                
                // chgivalue pattern - only update secondary (value), primary (type) stays same
                (*reg_base.add(1)).secondary = count - 1; // counter--
                (*reg_base).secondary = new_idx as u64;   // idx += step
                (*reg_base.add(3)).secondary = new_idx as u64; // control = idx
                
                *pc -= bx;
            }
            // count == 0: loop ended, fall through
            return Ok(());
        }

        // Float loop - slower path
        exec_forloop_float(vm, reg_base, bx, pc)
    }
}

/// Float loop - separate cold function
#[cold]
#[inline(never)]
pub fn exec_forloop_float(
    vm: &mut LuaVM,
    reg_base: *mut LuaValue,
    bx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    unsafe {
        let idx = *reg_base;
        let limit = *reg_base.add(1);
        let step = *reg_base.add(2);
        
        let idx_tag = idx.primary & TYPE_MASK;
        let limit_tag = limit.primary & TYPE_MASK;
        let step_tag = step.primary & TYPE_MASK;

        let idx_f = if idx_tag == TAG_FLOAT {
            f64::from_bits(idx.secondary)
        } else if idx_tag == TAG_INTEGER {
            idx.secondary as i64 as f64
        } else {
            return Err(vm.error("'for' index must be a number".to_string()));
        };

        let limit_f = if limit_tag == TAG_FLOAT {
            f64::from_bits(limit.secondary)
        } else if limit_tag == TAG_INTEGER {
            limit.secondary as i64 as f64
        } else {
            return Err(vm.error("'for' limit must be a number".to_string()));
        };

        let step_f = if step_tag == TAG_FLOAT {
            f64::from_bits(step.secondary)
        } else if step_tag == TAG_INTEGER {
            step.secondary as i64 as f64
        } else {
            return Err(vm.error("'for' step must be a number".to_string()));
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
            *pc -= bx;
        }
    }

    Ok(())
}

/// TFORPREP A Bx
/// create upvalue for R[A + 3]; pc+=Bx
/// In Lua 5.4, this creates a to-be-closed variable for the state
#[inline(always)]
pub fn exec_tforprep(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    // In Lua 5.4, R[A+3] is the to-be-closed variable for the state
    // For now, we just copy the state value to ensure it's preserved
    let state = vm.register_stack[base_ptr + a + 1];
    vm.register_stack[base_ptr + a + 3] = state;

    // Jump to loop start
    *pc += bx;
}

/// TFORCALL A C
/// R[A+4], ... ,R[A+3+C] := R[A](R[A+1], R[A+2]);
/// Returns true if a Lua function was called and frame changed (needs updatestate)
#[inline(always)]
pub fn exec_tforcall(vm: &mut LuaVM, instr: u32, frame_ptr: &mut *mut LuaCallFrame, base_ptr: usize) -> LuaResult<bool> {
    let a = Instruction::get_a(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    // Get iterator function and state
    let func = vm.register_stack[base_ptr + a];
    let state = vm.register_stack[base_ptr + a + 1];
    let control = vm.register_stack[base_ptr + a + 2];

    // Call func(state, control)
    // This is similar to CALL instruction but with fixed arguments
    match func.kind() {
        LuaValueKind::CFunction => {
            let Some(cfunc) = func.as_cfunction() else {
                return Err(vm.error("Invalid CFunction".to_string()));
            };

            // Set up call stack: func, state, control
            let call_base = base_ptr + a + 3;
            vm.register_stack[call_base] = func;
            vm.register_stack[call_base + 1] = state;
            vm.register_stack[call_base + 2] = control;

            // Create temporary frame for the call
            let temp_frame = LuaCallFrame::new_c_function(
                call_base, 3, // func + 2 args (top)
            );

            vm.push_frame(temp_frame);
            let result = cfunc(vm)?;
            vm.pop_frame_discard();

            // Store results starting at R[A+3]
            let values = result.all_values();
            for (i, value) in values.iter().enumerate().take(c + 1) {
                vm.register_stack[base_ptr + a + 3 + i] = *value;
            }
            // Fill remaining with nil
            for i in values.len()..=c {
                vm.register_stack[base_ptr + a + 3 + i] = LuaValue::nil();
            }
            Ok(false) // No frame change for C functions
        }
        LuaValueKind::Function => {
            // For Lua functions, we need to use CALL instruction logic
            // Set up registers for the call
            vm.register_stack[base_ptr + a + 3] = func;
            vm.register_stack[base_ptr + a + 4] = state;
            vm.register_stack[base_ptr + a + 5] = control;

            // Use new ID-based API to get function
            let Some(func_id) = func.as_function_id() else {
                return Err(vm.error("Not a Lua function".to_string()));
            };
            let Some(func_ref) = vm.object_pool.get_function(func_id) else {
                return Err(vm.error("Invalid function ID".to_string()));
            };

            let max_stack_size = func_ref.chunk.max_stack_size;
            let code_ptr = func_ref.chunk.code.as_ptr();
            let constants_ptr = func_ref.chunk.constants.as_ptr();

            // CRITICAL FIX: Use proper call base relative to current frame
            // Arguments are already at base_ptr + a + 4 and base_ptr + a + 5 (state, control)
            // New frame base should be at base_ptr + a + 4 where we placed the first arg
            let call_base = base_ptr + a + 4;
            vm.ensure_stack_capacity(call_base + max_stack_size);

            // Initialize registers beyond arguments
            for i in 2..max_stack_size {
                vm.register_stack[call_base + i] = LuaValue::nil();
            }

            // Arguments are already in place (state at +0, control at +1)
            // No need to copy them again

            // Create new frame with correct nresults type
            let nresults = (c + 1) as i16;
            let new_frame = LuaCallFrame::new_lua_function(
                func,
                code_ptr,
                constants_ptr,
                call_base,
                2,        // top = 2 (we have 2 arguments)
                a + 3,    // result goes to R[A+3]
                nresults, // expecting c+1 results
            );

            vm.push_frame(new_frame);
            // Update frame_ptr to point to new frame
            *frame_ptr = vm.current_frame_ptr();
            Ok(true) // Frame changed, need updatestate
        }
        _ => {
            Err(vm.error("attempt to call a non-function value in for loop".to_string()))
        }
    }
}

/// TFORLOOP A Bx
/// if R[A+1] ~= nil then { R[A]=R[A+1]; pc -= Bx }
#[inline(always)]
pub fn exec_tforloop(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let value = vm.register_stack[base_ptr + a + 1];

    if !value.is_nil() {
        // Continue loop
        vm.register_stack[base_ptr + a] = value;
        *pc -= bx;
    }
}
