/// Arithmetic and logical instructions
///
/// These instructions handle arithmetic operations, bitwise operations, and comparisons.
use crate::{
    LuaValue,
    lua_value::{TAG_FLOAT, TAG_INTEGER, TYPE_MASK},
    lua_vm::{Instruction, LuaCallFrame, LuaResult, LuaVM},
};

/// ADD: R[A] = R[B] + R[C]
/// ULTRA-OPTIMIZED: Uses pre-fetched frame_ptr to avoid Vec lookups
#[inline(always)]
pub fn exec_add(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        // OPTIMIZATION: Combined type check
        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        // Fast path: Both integers (single branch!)
        if combined_tags == TAG_INTEGER {
            let result =
                LuaValue::integer((left.secondary as i64).wrapping_add(right.secondary as i64));
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
            (*frame_ptr).pc += 1;
            return Ok(());
        }

        // Slow path: Check individual types
        let left_tag = left.primary & TYPE_MASK;
        let right_tag = right.primary & TYPE_MASK;

        let result = if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
            LuaValue::number(f64::from_bits(left.secondary) + f64::from_bits(right.secondary))
        } else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
            LuaValue::number((left.secondary as i64) as f64 + f64::from_bits(right.secondary))
        } else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
            LuaValue::number(f64::from_bits(left.secondary) + (right.secondary as i64) as f64)
        } else {
            return Ok(());
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
        (*frame_ptr).pc += 1;
        Ok(())
    }
}

/// SUB: R[A] = R[B] - R[C]
/// ULTRA-OPTIMIZED: Uses pre-fetched frame_ptr
#[inline(always)]
pub fn exec_sub(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        if combined_tags == TAG_INTEGER {
            let result =
                LuaValue::integer((left.secondary as i64).wrapping_sub(right.secondary as i64));
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
            (*frame_ptr).pc += 1;
            return Ok(());
        }

        let left_tag = left.primary & TYPE_MASK;
        let right_tag = right.primary & TYPE_MASK;

        let result = if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
            LuaValue::number(f64::from_bits(left.secondary) - f64::from_bits(right.secondary))
        } else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
            LuaValue::number((left.secondary as i64) as f64 - f64::from_bits(right.secondary))
        } else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
            LuaValue::number(f64::from_bits(left.secondary) - (right.secondary as i64) as f64)
        } else {
            return Ok(());
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
        (*frame_ptr).pc += 1;
        Ok(())
    }
}

/// MUL: R[A] = R[B] * R[C]
/// ULTRA-OPTIMIZED: Uses pre-fetched frame_ptr
#[inline(always)]
pub fn exec_mul(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        if combined_tags == TAG_INTEGER {
            let result =
                LuaValue::integer((left.secondary as i64).wrapping_mul(right.secondary as i64));
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
            (*frame_ptr).pc += 1;
            return Ok(());
        }

        let left_tag = left.primary & TYPE_MASK;
        let right_tag = right.primary & TYPE_MASK;

        let result = if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
            LuaValue::number(f64::from_bits(left.secondary) * f64::from_bits(right.secondary))
        } else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
            LuaValue::number((left.secondary as i64) as f64 * f64::from_bits(right.secondary))
        } else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
            LuaValue::number(f64::from_bits(left.secondary) * (right.secondary as i64) as f64)
        } else {
            return Ok(());
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
        (*frame_ptr).pc += 1;
        Ok(())
    }
}

/// DIV: R[A] = R[B] / R[C]
/// Division always returns float in Lua
#[inline(always)]
pub fn exec_div(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let left_tag = left.primary & TYPE_MASK;
        let right_tag = right.primary & TYPE_MASK;

        let l_float = if left_tag == TAG_INTEGER {
            (left.secondary as i64) as f64
        } else if left_tag == TAG_FLOAT {
            f64::from_bits(left.secondary)
        } else {
            return Ok(());
        };

        let r_float = if right_tag == TAG_INTEGER {
            (right.secondary as i64) as f64
        } else if right_tag == TAG_FLOAT {
            f64::from_bits(right.secondary)
        } else {
            return Ok(());
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(l_float / r_float);
        (*frame_ptr).pc += 1;
        Ok(())
    }
}

/// IDIV: R[A] = R[B] // R[C] (floor division)
#[inline(always)]
pub fn exec_idiv(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        let result = if combined_tags == TAG_INTEGER {
            let r = right.secondary as i64;
            if r == 0 {
                return Err(vm.error("attempt to divide by zero".to_string()));
            }
            let l = left.secondary as i64;
            LuaValue::integer(l.div_euclid(r))
        } else {
            let left_tag = left.primary & TYPE_MASK;
            let right_tag = right.primary & TYPE_MASK;

            let l_float = if left_tag == TAG_INTEGER {
                (left.secondary as i64) as f64
            } else if left_tag == TAG_FLOAT {
                f64::from_bits(left.secondary)
            } else {
                return Ok(());
            };

            let r_float = if right_tag == TAG_INTEGER {
                (right.secondary as i64) as f64
            } else if right_tag == TAG_FLOAT {
                f64::from_bits(right.secondary)
            } else {
                return Ok(());
            };

            LuaValue::number((l_float / r_float).floor())
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
        (*frame_ptr).pc += 1;
        Ok(())
    }
}

/// MOD: R[A] = R[B] % R[C]
#[inline(always)]
pub fn exec_mod(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        let result = if combined_tags == TAG_INTEGER {
            let r = right.secondary as i64;
            if r == 0 {
                return Err(vm.error("attempt to perform 'n%0'".to_string()));
            }
            let l = left.secondary as i64;
            LuaValue::integer(l.rem_euclid(r))
        } else {
            let left_tag = left.primary & TYPE_MASK;
            let right_tag = right.primary & TYPE_MASK;

            let l_float = if left_tag == TAG_INTEGER {
                (left.secondary as i64) as f64
            } else if left_tag == TAG_FLOAT {
                f64::from_bits(left.secondary)
            } else {
                return Ok(());
            };

            let r_float = if right_tag == TAG_INTEGER {
                (right.secondary as i64) as f64
            } else if right_tag == TAG_FLOAT {
                f64::from_bits(right.secondary)
            } else {
                return Ok(());
            };

            let result = l_float - (l_float / r_float).floor() * r_float;
            LuaValue::number(result)
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
        (*frame_ptr).pc += 1;
        Ok(())
    }
}

/// POW: R[A] = R[B] ^ R[C]
#[inline(always)]
pub fn exec_pow(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let right = *vm.register_stack.as_ptr().add(base_ptr + c);

        let l_float = match left.as_number() {
            Some(n) => n,
            None => return Ok(()),
        };
        let r_float = match right.as_number() {
            Some(n) => n,
            None => return Ok(()),
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(l_float.powf(r_float));
        (*frame_ptr).pc += 1;
        Ok(())
    }
}

/// UNM: R[A] = -R[B] (unary minus)
pub fn exec_unm(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];

    let result = if let Some(i) = value.as_integer() {
        if let Some(neg) = i.checked_neg() {
            LuaValue::integer(neg)
        } else {
            LuaValue::number(-(i as f64))
        }
    } else if let Some(f) = value.as_number() {
        LuaValue::number(-f)
    } else {
        // Try metamethod
        let mm_key = vm.create_string("__unm");
        if let Some(mt) = vm.table_get_metatable(&value) {
            if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key) {
                if !metamethod.is_nil() {
                    let result = vm
                        .call_metamethod(&metamethod, &[value])?
                        .unwrap_or(LuaValue::nil());
                    vm.register_stack[base_ptr + a] = result;
                    return Ok(());
                }
            }
        }
        return Err(vm.error(format!(
            "attempt to perform arithmetic on {}",
            value.type_name()
        )));
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(())
}

// ============ Arithmetic Immediate Instructions ============

/// ADDI: R[A] = R[B] + sC
/// OPTIMIZATION: After successful integer add, check if next instruction is JMP and execute inline
#[inline(always)]
pub fn exec_addi(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let sc = Instruction::get_sc(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        if left.primary == TAG_INTEGER {
            let l = left.secondary as i64;
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) =
                LuaValue::integer(l.wrapping_add(sc as i64));
            (*frame_ptr).pc += 1; // Skip MMBINI

            // OPTIMIZATION: Check if next instruction is backward JMP (loop)
            let next_instr = (*frame_ptr).code_ptr.add((*frame_ptr).pc).read();
            if (next_instr & 0x7F) == 56 {
                // JMP opcode
                let sj = ((next_instr >> 7) & 0x1FFFFFF) as i32 - 16777215;
                if sj < 0 {
                    // Backward jump = loop
                    (*frame_ptr).pc = ((*frame_ptr).pc as i32 + 1 + sj) as usize;
                }
            }
            return Ok(());
        }

        if left.primary == TAG_FLOAT {
            let l = f64::from_bits(left.secondary);
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::float(l + sc as f64);
            (*frame_ptr).pc += 1;
            return Ok(());
        }

        Ok(())
    }
}

/// ADDK: R[A] = R[B] + K[C]
pub fn exec_addk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try integer operation
    if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        // Integer operation with wraparound, skip fallback
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.wrapping_add(r));
        vm.current_frame_mut().pc += 1;
        return Ok(()); // Skip MMBIN fallback
    }

    // Try float operation
    if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
        // Float operation succeeded, skip fallback
        vm.register_stack[base_ptr + a] = LuaValue::number(l + r);
        vm.current_frame_mut().pc += 1;
        return Ok(()); // Skip MMBIN fallback
    }

    // Not numbers, fallthrough to MMBIN
    Ok(())
}

/// SUBK: R[A] = R[B] - K[C]
pub fn exec_subk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try integer operation
    if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.wrapping_sub(r));
        vm.current_frame_mut().pc += 1;
        return Ok(()); // Skip MMBIN fallback
    }

    // Try float operation
    if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
        vm.register_stack[base_ptr + a] = LuaValue::number(l - r);
        vm.current_frame_mut().pc += 1;
        return Ok(()); // Skip MMBIN fallback
    }
    Ok(())
}

/// MULK: R[A] = R[B] * K[C]
#[inline(always)]
pub fn exec_mulk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };
    let left = vm.register_stack[base_ptr + b];

    // Fast path: Direct type tag check (avoid method call overhead)

    // Check if both are integers
    if left.primary == TAG_INTEGER && constant.primary == TAG_INTEGER {
        let l = left.secondary as i64;
        let r = constant.secondary as i64;
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.wrapping_mul(r));
        vm.current_frame_mut().pc += 1;
        return Ok(());
    }

    // Check if at least one is float (covers float*float, float*int, int*float)
    if left.primary == TAG_FLOAT || constant.primary == TAG_FLOAT {
        // Convert to float
        let l = if left.primary == TAG_FLOAT {
            f64::from_bits(left.secondary)
        } else if left.primary == TAG_INTEGER {
            left.secondary as i64 as f64
        } else {
            return Ok(()); // Not a number
        };

        let r = if constant.primary == TAG_FLOAT {
            f64::from_bits(constant.secondary)
        } else if constant.primary == TAG_INTEGER {
            constant.secondary as i64 as f64
        } else {
            return Ok(()); // Not a number
        };

        vm.register_stack[base_ptr + a] = LuaValue::float(l * r);
        vm.current_frame_mut().pc += 1;
        return Ok(());
    }

    Ok(())
}

/// MODK: R[A] = R[B] % K[C]
pub fn exec_modk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try integer operation
    if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if r == 0 {
            return Err(vm.error("attempt to perform 'n%0'".to_string()));
        }
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.rem_euclid(r));
        vm.current_frame_mut().pc += 1;
        return Ok(()); // Skip MMBIN fallback
    }

    // Try float operation
    if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
        let result = l - (l / r).floor() * r;
        vm.register_stack[base_ptr + a] = LuaValue::number(result);
        vm.current_frame_mut().pc += 1;
        return Ok(()); // Skip MMBIN fallback
    }
    Ok(())
}

/// POWK: R[A] = R[B] ^ K[C]
pub fn exec_powk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try float operation
    let l_float = match left.as_number() {
        Some(n) => n,
        None => return Ok(()), // Let MMBINK handle
    };
    let r_float = match constant.as_number() {
        Some(n) => n,
        None => return Ok(()), // Let MMBINK handle
    };

    let result = LuaValue::number(l_float.powf(r_float));
    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(()) // Skip MMBIN fallback
}

/// DIVK: R[A] = R[B] / K[C]
pub fn exec_divk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try to perform the operation; if not possible, let MMBINK handle it
    let l_float = match left.as_number() {
        Some(n) => n,
        None => return Ok(()), // Let MMBINK handle
    };
    let r_float = match constant.as_number() {
        Some(n) => n,
        None => return Ok(()), // Let MMBINK handle
    };

    let result = LuaValue::number(l_float / r_float);
    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(())
}

/// IDIVK: R[A] = R[B] // K[C]
pub fn exec_idivk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try integer operation
    if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if r == 0 {
            return Err(vm.error("attempt to divide by zero".to_string()));
        }
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.div_euclid(r));
        vm.current_frame_mut().pc += 1;
        return Ok(()); // Skip MMBIN fallback
    }

    // Try float operation
    if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
        vm.register_stack[base_ptr + a] = LuaValue::number((l / r).floor());
        vm.current_frame_mut().pc += 1;
        return Ok(()); // Skip MMBIN fallback
    }
    Ok(())
}

// ============ Bitwise Operations ============

/// BAND: R[A] = R[B] & R[C]
pub fn exec_band(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBIN handle
    };
    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBIN handle
    };

    let result = LuaValue::integer(l_int & r_int);
    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(())
}

/// BOR: R[A] = R[B] | R[C]
pub fn exec_bor(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBIN handle
    };
    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBIN handle
    };

    let result = LuaValue::integer(l_int | r_int);
    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(())
}

/// BXOR: R[A] = R[B] ~ R[C]
pub fn exec_bxor(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBIN handle
    };
    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBIN handle
    };

    let result = LuaValue::integer(l_int ^ r_int);
    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(())
}

/// SHL: R[A] = R[B] << R[C]
pub fn exec_shl(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let Some(l_int) = left.as_integer() else {
        return Ok(());
    };
    let Some(r_int) = right.as_integer() else {
        return Ok(());
    };

    let result = if r_int >= 0 {
        LuaValue::integer(l_int << (r_int & 63))
    } else {
        LuaValue::integer(l_int >> ((-r_int) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(())
}

/// SHR: R[A] = R[B] >> R[C]
pub fn exec_shr(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBIN handle
    };
    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBIN handle
    };

    let result = if r_int >= 0 {
        LuaValue::integer(l_int >> (r_int & 63))
    } else {
        LuaValue::integer(l_int << ((-r_int) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(())
}

/// BANDK: R[A] = R[B] & K[C]
pub fn exec_bandk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try to get integer from left operand (may convert from float)
    let l_int = if let Some(i) = left.as_integer() {
        i
    } else if let Some(f) = left.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(());
        }
    } else {
        return Ok(());
    };

    // Try to get integer from constant
    let r_int = if let Some(i) = constant.as_integer() {
        i
    } else if let Some(f) = constant.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(());
        }
    } else {
        return Ok(());
    };

    vm.register_stack[base_ptr + a] = LuaValue::integer(l_int & r_int);
    vm.current_frame_mut().pc += 1;
    Ok(()) // Skip MMBIN fallback
}

/// BORK: R[A] = R[B] | K[C]
pub fn exec_bork(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try to get integer from left operand (may convert from float)
    let l_int = if let Some(i) = left.as_integer() {
        i
    } else if let Some(f) = left.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(());
        }
    } else {
        return Ok(());
    };

    // Try to get integer from constant
    let r_int = if let Some(i) = constant.as_integer() {
        i
    } else if let Some(f) = constant.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(());
        }
    } else {
        return Ok(());
    };

    vm.register_stack[base_ptr + a] = LuaValue::integer(l_int | r_int);
    vm.current_frame_mut().pc += 1;
    Ok(()) // Skip MMBIN fallback
}

/// BXORK: R[A] = R[B] ~ K[C]
pub fn exec_bxork(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk using new API
    let Some(constant) = vm.get_frame_constant(frame, c) else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let left = vm.register_stack[base_ptr + b];

    // Try to get integer from left operand (may convert from float)
    let l_int = if let Some(i) = left.as_integer() {
        i
    } else if let Some(f) = left.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(());
        }
    } else {
        return Ok(());
    };

    // Try to get integer from constant
    let r_int = if let Some(i) = constant.as_integer() {
        i
    } else if let Some(f) = constant.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(());
        }
    } else {
        return Ok(());
    };

    vm.register_stack[base_ptr + a] = LuaValue::integer(l_int ^ r_int);
    vm.current_frame_mut().pc += 1;
    Ok(()) // Skip MMBIN fallback
}

/// SHRI: R[A] = R[B] >> sC
pub fn exec_shri(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let sc = Instruction::get_sc(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBINI handle metamethod
    };

    let result = if sc >= 0 {
        LuaValue::integer(l_int >> (sc & 63))
    } else {
        LuaValue::integer(l_int << ((-sc) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(())
}

/// SHLI: R[A] = sC << R[B]
pub fn exec_shli(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let sc = Instruction::get_sc(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let right = vm.register_stack[base_ptr + b];

    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(()), // Let MMBINI handle metamethod
    };

    let result = if r_int >= 0 {
        LuaValue::integer((sc as i64) << (r_int & 63))
    } else {
        LuaValue::integer((sc as i64) >> ((-r_int) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    vm.current_frame_mut().pc += 1;
    Ok(())
}

/// BNOT: R[A] = ~R[B]
pub fn exec_bnot(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];

    let result = if let Some(int_val) = value.as_integer() {
        LuaValue::integer(!int_val)
    } else {
        // Try metamethod
        let mm_key = vm.create_string("__bnot");
        if let Some(mt) = vm.table_get_metatable(&value) {
            if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key) {
                if !metamethod.is_nil() {
                    let result = vm
                        .call_metamethod(&metamethod, &[value])?
                        .unwrap_or(LuaValue::nil());
                    vm.register_stack[base_ptr + a] = result;
                    return Ok(());
                }
            }
        }
        return Err(vm.error(format!(
            "attempt to perform bitwise operation on {}",
            value.type_name()
        )));
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(())
}

/// NOT: R[A] = not R[B]
pub fn exec_not(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];
    // In Lua, only nil and false are falsy; everything else is truthy
    // NOT returns true if value is nil or false, otherwise returns false
    let is_falsy = value.is_nil() || matches!(value.as_bool(), Some(false));
    let result = LuaValue::boolean(is_falsy);

    vm.register_stack[base_ptr + a] = result;
    Ok(())
}

/// LEN: R[A] = #R[B]
#[inline(always)]
pub fn exec_len(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];

    // Check for __len metamethod first (for tables)
    if value.is_table() {
        let mm_key = vm.create_string("__len");
        if let Some(mt) = vm.table_get_metatable(&value) {
            if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key) {
                if !metamethod.is_nil() {
                    let result = vm
                        .call_metamethod(&metamethod, &[value])?
                        .unwrap_or(LuaValue::nil());
                    vm.register_stack[base_ptr + a] = result;
                    return Ok(());
                }
            }
        }
    }

    // Use ObjectPool for table/string length
    let len = if let Some(table_id) = value.as_table_id() {
        if let Some(table) = vm.object_pool.get_table(table_id) {
            table.len() as i64
        } else {
            0
        }
    } else if let Some(string_id) = value.as_string_id() {
        if let Some(s) = vm.object_pool.get_string(string_id) {
            s.as_str().len() as i64
        } else {
            0
        }
    } else {
        return Err(vm.error(format!("attempt to get length of {}", value.type_name())));
    };

    let result = LuaValue::integer(len);
    vm.register_stack[base_ptr + a] = result;
    Ok(())
}

/// Get metamethod name for binary operation
fn get_binop_metamethod(tm: u8) -> &'static str {
    // TMS enum from ltm.h:
    // TM_INDEX=0, TM_NEWINDEX=1, TM_GC=2, TM_MODE=3, TM_LEN=4, TM_EQ=5,
    // TM_ADD=6, TM_SUB=7, TM_MUL=8, TM_MOD=9, TM_POW=10, TM_DIV=11,
    // TM_IDIV=12, TM_BAND=13, TM_BOR=14, TM_BXOR=15, TM_SHL=16, TM_SHR=17,
    // TM_UNM=18, TM_BNOT=19, TM_LT=20, TM_LE=21, TM_CONCAT=22, TM_CALL=23, TM_CLOSE=24
    match tm {
        6 => "__add",
        7 => "__sub",
        8 => "__mul",
        9 => "__mod",
        10 => "__pow",
        11 => "__div",
        12 => "__idiv",
        13 => "__band",
        14 => "__bor",
        15 => "__bxor",
        16 => "__shl",
        17 => "__shr",
        22 => "__concat",
        5 => "__eq",
        20 => "__lt",
        21 => "__le",
        18 => "__unm",
        19 => "__bnot",
        4 => "__len",
        _ => "__unknown",
    }
}

/// MmBin: Metamethod binary operation (register, register)
pub fn exec_mmbin(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize; // operand 1
    let b = Instruction::get_b(instr) as usize; // operand 2
    let c = Instruction::get_c(instr) as usize; // C is the TagMethod index
    // k bit is unused for MMBIN

    // Get the previous instruction to find the destination register
    let frame = vm.current_frame();
    let prev_pc = frame.pc - 1; // Previous instruction was the failed arithmetic op
    let base_ptr = frame.base_ptr;

    if prev_pc == 0 {
        return Err(vm.error("MMBIN: no previous instruction".to_string()));
    }

    let Some(prev_instr) = vm.get_frame_instruction(frame, prev_pc - 1) else {
        return Err(vm.error("MMBIN: failed to get previous instruction".to_string()));
    };
    let dest_reg = Instruction::get_a(prev_instr) as usize; // Destination register from ADD/SUB/etc

    // A and B are the operand registers
    let ra = vm.register_stack[base_ptr + a];
    let rb = vm.register_stack[base_ptr + b];

    // C is the metamethod index (TagMethod enum value)
    let metamethod_name = get_binop_metamethod(c as u8);
    let mm_key = vm.create_string(metamethod_name);

    // Try to get metamethod from first operand's metatable
    let metamethod = if let Some(mt) = vm.table_get_metatable(&ra) {
        vm.table_get_with_meta(&mt, &mm_key)
            .unwrap_or(LuaValue::nil())
    } else if let Some(mt) = vm.table_get_metatable(&rb) {
        vm.table_get_with_meta(&mt, &mm_key)
            .unwrap_or(LuaValue::nil())
    } else {
        LuaValue::nil()
    };

    if metamethod.is_nil() {
        return Err(vm.error(format!(
            "attempt to perform arithmetic on {} and {}",
            ra.type_name(),
            rb.type_name()
        )));
    }

    // Call metamethod
    let args = vec![ra, rb];
    let result = vm
        .call_metamethod(&metamethod, &args)?
        .unwrap_or(LuaValue::nil());
    // Store result in the destination register from the previous instruction
    vm.register_stack[base_ptr + dest_reg] = result;

    Ok(())
}

/// MmBinI: Metamethod binary operation (register, immediate)
pub fn exec_mmbini(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr); // Signed immediate value (as in ADDI)
    let c = Instruction::get_c(instr);
    let k = Instruction::get_k(instr);

    // Get the previous instruction to find the destination register
    let frame = vm.current_frame();
    let prev_pc = frame.pc - 1; // Previous instruction was the failed arithmetic op
    let base_ptr = frame.base_ptr;

    if prev_pc == 0 {
        return Err(vm.error("MMBINI: no previous instruction".to_string()));
    }

    let Some(prev_instr) = vm.get_frame_instruction(frame, prev_pc - 1) else {
        return Err(vm.error("MMBINI: failed to get previous instruction".to_string()));
    };
    let dest_reg = Instruction::get_a(prev_instr) as usize; // Destination register from ADDI

    // From compiler: create_abck(OpCode::MmBinI, left_reg, imm, TagMethod, false)
    // So: A = left_reg (operand), B = imm (immediate value encoded), C = tagmethod

    let rb = vm.register_stack[base_ptr + a]; // A contains the operand register content
    let rc = LuaValue::integer(sb as i64); // B field contains the immediate value

    let metamethod_name = get_binop_metamethod(c as u8);
    let mm_key = vm.create_string(metamethod_name);

    // Try to get metamethod from operand's metatable
    let metamethod = if let Some(mt) = vm.table_get_metatable(&rb) {
        vm.table_get_with_meta(&mt, &mm_key)
            .unwrap_or(LuaValue::nil())
    } else {
        LuaValue::nil()
    };

    if metamethod.is_nil() {
        return Err(vm.error(format!(
            "attempt to perform arithmetic on {} and {}",
            rb.type_name(),
            rc.type_name()
        )));
    }

    // Call metamethod
    let args = if k { vec![rc, rb] } else { vec![rb, rc] };
    let result = vm
        .call_metamethod(&metamethod, &args)?
        .unwrap_or(LuaValue::nil());
    vm.register_stack[base_ptr + dest_reg] = result;

    Ok(())
}

/// MmBinK: Metamethod binary operation (register, constant)
pub fn exec_mmbink(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize; // C is the TagMethod
    let k = Instruction::get_k(instr); // k is the flip flag

    // Get the previous instruction to find the destination register
    let frame = vm.current_frame();
    let prev_pc = frame.pc - 1; // Previous instruction was the failed arithmetic op
    let base_ptr = frame.base_ptr;

    if prev_pc == 0 {
        return Err(vm.error("MMBINK: no previous instruction".to_string()));
    }

    let Some(prev_instr) = vm.get_frame_instruction(frame, prev_pc - 1) else {
        return Err(vm.error("MMBINK: failed to get previous instruction".to_string()));
    };
    let dest_reg = Instruction::get_a(prev_instr) as usize; // Destination register from ADDK/SUBK/etc

    let ra = vm.register_stack[base_ptr + a];
    let Some(kb) = vm.get_frame_constant(frame, b) else {
        return Err(vm.error(format!("Constant index out of bounds: {}", b)));
    };

    // C is the TagMethod, not a constant index
    let metamethod_name = get_binop_metamethod(c as u8);
    let mm_key = vm.create_string(metamethod_name);

    // k is flip flag: if k==false then (ra, kb), if k==true then (kb, ra)
    let (left, right) = if !k { (ra, kb) } else { (kb, ra) };

    // Try to get metamethod from operands' metatables
    let metamethod = if let Some(mt) = vm.table_get_metatable(&left) {
        vm.table_get_with_meta(&mt, &mm_key)
            .unwrap_or(LuaValue::nil())
    } else if let Some(mt) = vm.table_get_metatable(&right) {
        vm.table_get_with_meta(&mt, &mm_key)
            .unwrap_or(LuaValue::nil())
    } else {
        LuaValue::nil()
    };

    if metamethod.is_nil() {
        return Err(vm.error(format!(
            "attempt to perform arithmetic on {} and {}",
            left.type_name(),
            right.type_name()
        )));
    }

    // Call metamethod
    let args = vec![left, right];
    let result = vm
        .call_metamethod(&metamethod, &args)?
        .unwrap_or(LuaValue::nil());
    vm.register_stack[base_ptr + dest_reg] = result;

    Ok(())
}
