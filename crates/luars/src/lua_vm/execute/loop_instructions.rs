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
/// ULTRA-OPTIMIZED: Check idx type (set by FORPREP), use chgivalue pattern
#[inline(always)]
#[allow(dead_code)]
pub fn exec_forloop(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    unsafe {
        let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr + a);

        // Check idx type - FORPREP sets this to integer only if ALL of init/limit/step are integers
        let idx = *reg_base;

        if (idx.primary & TYPE_MASK) == TAG_INTEGER {
            // Integer loop - idx and counter are integers (set by FORPREP)
            let count = (*reg_base.add(1)).secondary; // counter as u64

            if count > 0 {
                let idx_i = idx.secondary as i64;
                let step_i = (*reg_base.add(2)).secondary as i64;
                let new_idx = idx_i.wrapping_add(step_i);

                // chgivalue pattern - only update secondary (value), primary (type) stays same
                (*reg_base.add(1)).secondary = count - 1; // counter--
                (*reg_base).secondary = new_idx as u64; // idx += step
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
/// 
/// Lua 5.4 for-in loop layout:
/// R[A]   = iter_func
/// R[A+1] = state
/// R[A+2] = control variable
/// R[A+3] = to-be-closed variable (not used here)
/// R[A+4] = first loop variable (first return value goes here)
/// R[A+5] = second loop variable, etc.
///
/// Returns true if a Lua function was called and frame changed (needs updatestate)
#[inline(always)]
pub fn exec_tforcall(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: &mut *mut LuaCallFrame,
    base_ptr: usize,
) -> LuaResult<bool> {
    let a = Instruction::get_a(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    // Get iterator function and state
    let func = unsafe { *vm.register_stack.get_unchecked(base_ptr + a) };
    let state = unsafe { *vm.register_stack.get_unchecked(base_ptr + a + 1) };
    let control = unsafe { *vm.register_stack.get_unchecked(base_ptr + a + 2) };

    // FAST PATH: Check if it's a CFunction (most common for ipairs/pairs)
    use crate::lua_value::TAG_CFUNCTION;
    if func.primary == TAG_CFUNCTION {
        let cfunc = unsafe { func.as_cfunction().unwrap_unchecked() };

        // Set up temporary call frame position (beyond result area)
        let call_base = base_ptr + a + 4 + c + 1;
        vm.ensure_stack_capacity(call_base + 3);
        
        unsafe {
            *vm.register_stack.get_unchecked_mut(call_base) = func;
            *vm.register_stack.get_unchecked_mut(call_base + 1) = state;
            *vm.register_stack.get_unchecked_mut(call_base + 2) = control;
        }

        // Create minimal temporary frame for the call
        let temp_frame = LuaCallFrame::new_c_function(call_base, 3);
        vm.push_frame(temp_frame);
        let result = cfunc(vm)?;
        vm.pop_frame_discard();

        // OPTIMIZED: Direct inline access without Vec allocation
        // ipairs/pairs typically return 2 values (index, value) or nil
        let result_base = base_ptr + a + 4;
        
        if result.overflow.is_some() {
            // Rare case: more than 2 values
            let values = result.overflow.unwrap();
            let count = values.len().min(c);
            for i in 0..count {
                unsafe { *vm.register_stack.get_unchecked_mut(result_base + i) = values[i]; }
            }
            for i in count..c {
                unsafe { *vm.register_stack.get_unchecked_mut(result_base + i) = LuaValue::nil(); }
            }
        } else {
            // Common case: 0-2 inline values
            let inline_count = result.inline_count as usize;
            unsafe {
                if c >= 1 {
                    *vm.register_stack.get_unchecked_mut(result_base) = 
                        if inline_count >= 1 { result.inline[0] } else { LuaValue::nil() };
                }
                if c >= 2 {
                    *vm.register_stack.get_unchecked_mut(result_base + 1) = 
                        if inline_count >= 2 { result.inline[1] } else { LuaValue::nil() };
                }
                // Fill any remaining slots with nil
                for i in 2..c {
                    *vm.register_stack.get_unchecked_mut(result_base + i) = LuaValue::nil();
                }
            }
        }
        return Ok(false);
    }

    // Lua function path
    match func.kind() {
        LuaValueKind::Function => {
            // For Lua functions, set up for a normal call
            // We need to place function and arguments, then results go to R[A+4]
            
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

            // Set up arguments at base_ptr + a + 4 (first arg = state)
            // Arguments: state, control
            let call_base = base_ptr + a + 4;
            vm.ensure_stack_capacity(call_base + max_stack_size);
            
            // Place arguments (overwriting result slots temporarily is OK)
            vm.register_stack[call_base] = state;
            vm.register_stack[call_base + 1] = control;

            // Initialize registers beyond arguments
            for i in 2..max_stack_size {
                vm.register_stack[call_base + i] = LuaValue::nil();
            }

            // Create new frame
            // Result destination is a + 4 (relative to caller's base)
            let nresults = c as i16;
            let new_frame = LuaCallFrame::new_lua_function(
                func_id,
                code_ptr,
                constants_ptr,
                call_base,
                2,        // top = 2 (we have 2 arguments)
                a + 4,    // result goes to R[A+4] relative to caller's base
                nresults, // expecting c results
            );

            vm.push_frame(new_frame);
            // Update frame_ptr to point to new frame
            *frame_ptr = vm.current_frame_ptr();
            Ok(true) // Frame changed, need updatestate
        }
        _ => Err(vm.error("attempt to call a non-function value in for loop".to_string())),
    }
}

/// TFORLOOP A Bx
/// if R[A+4] ~= nil then { R[A+2]=R[A+4]; pc -= Bx }
/// 
/// Lua 5.4 for-in loop layout:
/// R[A]   = iter_func
/// R[A+1] = state
/// R[A+2] = control variable (updated here when continuing)
/// R[A+3] = to-be-closed variable
/// R[A+4] = first loop variable (checked here)
#[inline(always)]
pub fn exec_tforloop(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    // Check first loop variable at R[A+4]
    let first_var = vm.register_stack[base_ptr + a + 4];

    if !first_var.is_nil() {
        // Continue loop: update control variable R[A+2] with first return value
        vm.register_stack[base_ptr + a + 2] = first_var;
        *pc -= bx;
    }
}
