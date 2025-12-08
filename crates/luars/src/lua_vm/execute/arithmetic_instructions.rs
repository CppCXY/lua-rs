/// Arithmetic and logical instructions
///
/// These instructions handle arithmetic operations, bitwise operations, and comparisons.
use crate::{
    LuaValue, get_a, get_b, get_c, get_k, get_sb, get_sc,
    lua_value::{TAG_FLOAT, TAG_INTEGER, TYPE_MASK},
    lua_vm::{LuaCallFrame, LuaResult, LuaVM},
};

/// ADD: R[A] = R[B] + R[C]
/// OPTIMIZED: Matches Lua C's setivalue behavior - always write both fields
#[inline(always)]
pub fn exec_add(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        // Combined type check - if result is TAG_INTEGER, both are integers
        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        if combined_tags == TAG_INTEGER {
            // Fast path: Both integers - like Lua's setivalue, write both fields
            let result = (left.secondary as i64).wrapping_add(right.secondary as i64);
            *reg_base.add(a) = LuaValue {
                primary: TAG_INTEGER,
                secondary: result as u64,
            };
            *pc += 1; // Skip MMBIN
            return;
        }

        let left_tag = left.primary & TYPE_MASK;
        let right_tag = right.primary & TYPE_MASK;

        let result = if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
            LuaValue::number(f64::from_bits(left.secondary) + f64::from_bits(right.secondary))
        } else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
            LuaValue::number((left.secondary as i64) as f64 + f64::from_bits(right.secondary))
        } else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
            LuaValue::number(f64::from_bits(left.secondary) + (right.secondary as i64) as f64)
        } else {
            return;
        };

        *reg_base.add(a) = result;
        *pc += 1;
    }
}

/// SUB: R[A] = R[B] - R[C]
/// OPTIMIZED: Matches Lua C's setivalue behavior
#[inline(always)]
pub fn exec_sub(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        if combined_tags == TAG_INTEGER {
            let result = (left.secondary as i64).wrapping_sub(right.secondary as i64);
            *reg_base.add(a) = LuaValue {
                primary: TAG_INTEGER,
                secondary: result as u64,
            };
            *pc += 1;
            return;
        }

        // Slow path: mixed or float types
        let left_tag = left.primary & TYPE_MASK;
        let right_tag = right.primary & TYPE_MASK;

        let result = if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
            LuaValue::number(f64::from_bits(left.secondary) - f64::from_bits(right.secondary))
        } else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
            LuaValue::number((left.secondary as i64) as f64 - f64::from_bits(right.secondary))
        } else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
            LuaValue::number(f64::from_bits(left.secondary) - (right.secondary as i64) as f64)
        } else {
            return;
        };

        *reg_base.add(a) = result;
        *pc += 1;
    }
}

/// MUL: R[A] = R[B] * R[C]
/// OPTIMIZED: Matches Lua C's setivalue behavior
#[inline(always)]
pub fn exec_mul(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        if combined_tags == TAG_INTEGER {
            let result = (left.secondary as i64).wrapping_mul(right.secondary as i64);
            *reg_base.add(a) = LuaValue {
                primary: TAG_INTEGER,
                secondary: result as u64,
            };
            *pc += 1;
            return;
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
            return;
        };

        *reg_base.add(a) = result;
        *pc += 1;
    }
}

/// DIV: R[A] = R[B] / R[C]
/// OPTIMIZED: If both are integers and division is exact, return integer
pub fn exec_div(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        // Fast path: both integers - check if division is exact
        if combined_tags == TAG_INTEGER {
            let l = left.secondary as i64;
            let r = right.secondary as i64;
            if r != 0 && l % r == 0 {
                // Exact division - return integer
                *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l / r);
                *pc += 1;
                return;
            }
        }

        // Slow path: convert to float
        let left_tag = left.primary & TYPE_MASK;
        let right_tag = right.primary & TYPE_MASK;

        let l_float = if left_tag == TAG_INTEGER {
            (left.secondary as i64) as f64
        } else if left_tag == TAG_FLOAT {
            f64::from_bits(left.secondary)
        } else {
            return;
        };

        let r_float = if right_tag == TAG_INTEGER {
            (right.secondary as i64) as f64
        } else if right_tag == TAG_FLOAT {
            f64::from_bits(right.secondary)
        } else {
            return;
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(l_float / r_float);
        *pc += 1;
    }
}

/// IDIV: R[A] = R[B] // R[C] (floor division)
pub fn exec_idiv(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        let result = if combined_tags == TAG_INTEGER {
            let r = right.secondary as i64;
            if r == 0 {
                return; // Division by zero - let MMBIN handle
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
                return;
            };

            let r_float = if right_tag == TAG_INTEGER {
                (right.secondary as i64) as f64
            } else if right_tag == TAG_FLOAT {
                f64::from_bits(right.secondary)
            } else {
                return;
            };

            LuaValue::number((l_float / r_float).floor())
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
        *pc += 1;
    }
}

/// MOD: R[A] = R[B] % R[C]
/// OPTIMIZED: Returns integer when both operands are integers
pub fn exec_mod(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        let left = *reg_base.add(b);
        let right = *reg_base.add(c);

        let combined_tags = (left.primary | right.primary) & TYPE_MASK;

        let result = if combined_tags == TAG_INTEGER {
            let r = right.secondary as i64;
            if r == 0 {
                return; // Mod by zero - let MMBIN handle
            }
            let l = left.secondary as i64;
            // Integer modulo always returns integer
            LuaValue::integer(l.rem_euclid(r))
        } else {
            let left_tag = left.primary & TYPE_MASK;
            let right_tag = right.primary & TYPE_MASK;

            let l_float = if left_tag == TAG_INTEGER {
                (left.secondary as i64) as f64
            } else if left_tag == TAG_FLOAT {
                f64::from_bits(left.secondary)
            } else {
                return;
            };

            let r_float = if right_tag == TAG_INTEGER {
                (right.secondary as i64) as f64
            } else if right_tag == TAG_FLOAT {
                f64::from_bits(right.secondary)
            } else {
                return;
            };

            let float_result = l_float - (l_float / r_float).floor() * r_float;
            // Check if result is an integer
            if float_result.fract() == 0.0
                && float_result.is_finite()
                && float_result >= i64::MIN as f64
                && float_result <= i64::MAX as f64
            {
                LuaValue::integer(float_result as i64)
            } else {
                LuaValue::number(float_result)
            }
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
        *pc += 1;
    }
}

/// POW: R[A] = R[B] ^ R[C]
/// OPTIMIZED: Fast path for small integer exponents
#[inline(always)]
pub fn exec_pow(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let right = *vm.register_stack.as_ptr().add(base_ptr + c);

        let l_float = match left.as_number() {
            Some(n) => n,
            None => return,
        };

        // Fast path: integer exponent (common case like x^2, x^3)
        if let Some(exp) = right.as_integer_strict() {
            let result = fast_int_pow(l_float, exp);
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(result);
            *pc += 1;
            return;
        }

        let r_float = match right.as_number() {
            Some(n) => n,
            None => return,
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(l_float.powf(r_float));
        *pc += 1;
    }
}

/// UNM: R[A] = -R[B] (unary minus)
pub fn exec_unm(vm: &mut LuaVM, instr: u32, base_ptr: usize) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);

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
        // Try metamethod - use pre-cached __unm StringId
        let mm_key = LuaValue::string(vm.object_pool.tm_unm);
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
/// OPTIMIZED: Minimal branches, inline integer path
#[inline(always)]
pub fn exec_addi(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let sc = get_sc!(instr);

    unsafe {
        let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr);
        let left = *reg_base.add(b);

        if left.primary == TAG_INTEGER {
            let result = (left.secondary as i64).wrapping_add(sc as i64);
            // Write both fields atomically (matches Lua's setivalue)
            let dest = reg_base.add(a);
            (*dest).primary = TAG_INTEGER;
            (*dest).secondary = result as u64;
            *pc += 1; // Skip MMBINI
            return;
        }

        if left.primary == TAG_FLOAT {
            let l = f64::from_bits(left.secondary);
            *reg_base.add(a) = LuaValue::float(l + sc as f64);
            *pc += 1;
            return;
        }
    }
}

/// ADDK: R[A] = R[B] + K[C]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
pub fn exec_addk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        // FAST PATH: Direct constant access via cached pointer
        let constant = *(*frame_ptr).constants_ptr.add(c);

        // Integer + Integer fast path
        if left.primary == TAG_INTEGER && constant.primary == TAG_INTEGER {
            let result = (left.secondary as i64).wrapping_add(constant.secondary as i64);
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue {
                primary: TAG_INTEGER,
                secondary: result as u64,
            };
            *pc += 1;
            return;
        }

        // Float + Float fast path
        if left.primary == TAG_FLOAT && constant.primary == TAG_FLOAT {
            let result = f64::from_bits(left.secondary) + f64::from_bits(constant.secondary);
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue {
                primary: TAG_FLOAT,
                secondary: result.to_bits(),
            };
            *pc += 1;
            return;
        }

        // Mixed types
        if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(l + r);
            *pc += 1;
        }
    }
}

/// SUBK: R[A] = R[B] - K[C]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
pub fn exec_subk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        // FAST PATH: Direct constant access via cached pointer
        let constant = *(*frame_ptr).constants_ptr.add(c);

        // Integer - Integer fast path
        if left.primary == TAG_INTEGER && constant.primary == TAG_INTEGER {
            let result = (left.secondary as i64).wrapping_sub(constant.secondary as i64);
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue {
                primary: TAG_INTEGER,
                secondary: result as u64,
            };
            *pc += 1;
            return;
        }

        // Float - Float fast path
        if left.primary == TAG_FLOAT && constant.primary == TAG_FLOAT {
            let result = f64::from_bits(left.secondary) - f64::from_bits(constant.secondary);
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue {
                primary: TAG_FLOAT,
                secondary: result.to_bits(),
            };
            *pc += 1;
            return;
        }

        // Mixed types
        if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(l - r);
            *pc += 1;
        }
    }
}

/// MULK: R[A] = R[B] * K[C]
/// OPTIMIZED: Direct field writes, minimal branching
#[inline(always)]
pub fn exec_mulk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        // FAST PATH: Direct constant access via cached pointer
        let constant = *(*frame_ptr).constants_ptr.add(c);

        // Integer * Integer fast path FIRST (most common in benchmarks with integer loops)
        if left.primary == TAG_INTEGER && constant.primary == TAG_INTEGER {
            let result = (left.secondary as i64).wrapping_mul(constant.secondary as i64);
            let dest = vm.register_stack.as_mut_ptr().add(base_ptr + a);
            (*dest).primary = TAG_INTEGER;
            (*dest).secondary = result as u64;
            *pc += 1;
            return;
        }

        // Float * Float fast path
        if left.primary == TAG_FLOAT && constant.primary == TAG_FLOAT {
            let result = f64::from_bits(left.secondary) * f64::from_bits(constant.secondary);
            let dest = vm.register_stack.as_mut_ptr().add(base_ptr + a);
            (*dest).primary = TAG_FLOAT;
            (*dest).secondary = result.to_bits();
            *pc += 1;
            return;
        }

        // Mixed types: Integer * Float or Float * Integer
        if left.primary == TAG_INTEGER && constant.primary == TAG_FLOAT {
            let result = (left.secondary as i64) as f64 * f64::from_bits(constant.secondary);
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::float(result);
            *pc += 1;
            return;
        }

        if left.primary == TAG_FLOAT && constant.primary == TAG_INTEGER {
            let result = f64::from_bits(left.secondary) * (constant.secondary as i64) as f64;
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::float(result);
            *pc += 1;
        }
    }
}

/// MODK: R[A] = R[B] % K[C]
/// OPTIMIZED: Returns integer when result is integer
#[inline(always)]
pub fn exec_modk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let constant = *(*frame_ptr).constants_ptr.add(c);

        // Integer % Integer fast path - always returns integer
        if left.primary == TAG_INTEGER && constant.primary == TAG_INTEGER {
            let r = constant.secondary as i64;
            if r == 0 {
                return;
            }
            let l = left.secondary as i64;
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue {
                primary: TAG_INTEGER,
                secondary: l.rem_euclid(r) as u64,
            };
            *pc += 1;
            return;
        }

        // Float % Float - check if result is integer
        if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
            let float_result = l - (l / r).floor() * r;
            if float_result.fract() == 0.0
                && float_result.is_finite()
                && float_result >= i64::MIN as f64
                && float_result <= i64::MAX as f64
            {
                *vm.register_stack.as_mut_ptr().add(base_ptr + a) =
                    LuaValue::integer(float_result as i64);
            } else {
                *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(float_result);
            }
            *pc += 1;
        }
    }
}

/// Fast integer power using binary exponentiation
/// Only for small positive exponents (0-63) to avoid overflow checks
#[inline(always)]
fn fast_int_pow(base: f64, exp: i64) -> f64 {
    match exp {
        0 => 1.0,
        1 => base,
        2 => base * base,
        3 => base * base * base,
        4 => {
            let b2 = base * base;
            b2 * b2
        }
        _ if exp > 0 && exp <= 63 => {
            // Binary exponentiation for larger exponents
            let mut result = 1.0;
            let mut b = base;
            let mut e = exp as u32;
            while e > 0 {
                if e & 1 == 1 {
                    result *= b;
                }
                b *= b;
                e >>= 1;
            }
            result
        }
        _ => base.powf(exp as f64), // Fallback for negative or very large
    }
}

/// POWK: R[A] = R[B] ^ K[C]
/// OPTIMIZED: Fast path for small integer exponents (^2, ^3, etc.)
#[inline(always)]
pub fn exec_powk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let constant = *(*frame_ptr).constants_ptr.add(c);

        let l_float = match left.as_number() {
            Some(n) => n,
            None => return,
        };

        // Fast path: integer exponent (common case like x^2, x^3)
        if let Some(exp) = constant.as_integer_strict() {
            let result = fast_int_pow(l_float, exp);
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(result);
            *pc += 1;
            return;
        }

        let r_float = match constant.as_number() {
            Some(n) => n,
            None => return,
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(l_float.powf(r_float));
        *pc += 1;
    }
}

/// DIVK: R[A] = R[B] / K[C]
/// OPTIMIZED: Returns integer when division is exact
#[inline(always)]
pub fn exec_divk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let constant = *(*frame_ptr).constants_ptr.add(c);

        // Fast path: both integers - check if division is exact
        if left.primary == TAG_INTEGER && constant.primary == TAG_INTEGER {
            let l = left.secondary as i64;
            let r = constant.secondary as i64;
            if r != 0 && l % r == 0 {
                *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l / r);
                *pc += 1;
                return;
            }
        }

        let l_float = match left.as_number() {
            Some(n) => n,
            None => return,
        };
        let r_float = match constant.as_number() {
            Some(n) => n,
            None => return,
        };

        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(l_float / r_float);
        *pc += 1;
    }
}

/// IDIVK: R[A] = R[B] // K[C]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
pub fn exec_idivk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        // FAST PATH: Direct constant access via cached pointer
        let constant = *(*frame_ptr).constants_ptr.add(c);

        // Integer // Integer fast path
        if left.primary == TAG_INTEGER && constant.primary == TAG_INTEGER {
            let r = constant.secondary as i64;
            if r == 0 {
                return;
            }
            let l = left.secondary as i64;
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue {
                primary: TAG_INTEGER,
                secondary: l.div_euclid(r) as u64,
            };
            *pc += 1;
            return;
        }

        // Float // Float
        if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number((l / r).floor());
            *pc += 1;
        }
    }
}

// ============ Bitwise Operations ============

/// BAND: R[A] = R[B] & R[C]
/// OPTIMIZED: Fast path for integer-integer, fallback for float conversion
#[inline(always)]
pub fn exec_band(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let right = *vm.register_stack.as_ptr().add(base_ptr + c);

        // Fast path: both are integers (most common case)
        if let (Some(l), Some(r)) = (left.as_integer_strict(), right.as_integer_strict()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l & r);
            *pc += 1;
            return;
        }
        // Slow path: try float conversion
        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l & r);
            *pc += 1;
        }
    }
}

/// BOR: R[A] = R[B] | R[C]
/// OPTIMIZED: Fast path for integer-integer
#[inline(always)]
pub fn exec_bor(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let right = *vm.register_stack.as_ptr().add(base_ptr + c);

        // Fast path: both are integers
        if let (Some(l), Some(r)) = (left.as_integer_strict(), right.as_integer_strict()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l | r);
            *pc += 1;
            return;
        }
        // Slow path: try float conversion
        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l | r);
            *pc += 1;
        }
    }
}

/// BXOR: R[A] = R[B] ~ R[C]
/// OPTIMIZED: Fast path for integer-integer
#[inline(always)]
pub fn exec_bxor(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let right = *vm.register_stack.as_ptr().add(base_ptr + c);

        // Fast path: both are integers
        if let (Some(l), Some(r)) = (left.as_integer_strict(), right.as_integer_strict()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l ^ r);
            *pc += 1;
            return;
        }
        // Slow path: try float conversion
        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l ^ r);
            *pc += 1;
        }
    }
}

/// SHL: R[A] = R[B] << R[C]
/// OPTIMIZED: Fast path + branchless shift
#[inline(always)]
pub fn exec_shl(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let right = *vm.register_stack.as_ptr().add(base_ptr + c);

        // Fast path: both are integers
        if let (Some(l), Some(r)) = (left.as_integer_strict(), right.as_integer_strict()) {
            // Lua shift semantics: negative r means shift right
            let result = if r >= 0 {
                l.wrapping_shl((r & 63) as u32)
            } else {
                (l as u64).wrapping_shr(((-r) & 63) as u32) as i64
            };
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(result);
            *pc += 1;
            return;
        }
        // Slow path
        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            let result = if r >= 0 {
                l.wrapping_shl((r & 63) as u32)
            } else {
                (l as u64).wrapping_shr(((-r) & 63) as u32) as i64
            };
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(result);
            *pc += 1;
        }
    }
}

/// SHR: R[A] = R[B] >> R[C]
/// OPTIMIZED: Fast path + logical shift (unsigned)
#[inline(always)]
pub fn exec_shr(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);
        let right = *vm.register_stack.as_ptr().add(base_ptr + c);

        // Fast path: both are integers
        if let (Some(l), Some(r)) = (left.as_integer_strict(), right.as_integer_strict()) {
            // Lua uses logical (unsigned) right shift
            let result = if r >= 0 {
                (l as u64).wrapping_shr((r & 63) as u32) as i64
            } else {
                l.wrapping_shl(((-r) & 63) as u32)
            };
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(result);
            *pc += 1;
            return;
        }
        // Slow path
        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            let result = if r >= 0 {
                (l as u64).wrapping_shr((r & 63) as u32) as i64
            } else {
                l.wrapping_shl(((-r) & 63) as u32)
            };
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(result);
            *pc += 1;
        }
    }
}

/// BANDK: R[A] = R[B] & K[C]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
pub fn exec_bandk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        // FAST PATH: Direct constant access via cached pointer
        let constant = *(*frame_ptr).constants_ptr.add(c);

        let l_int = left.as_integer().or_else(|| {
            left.as_number().and_then(|f| {
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    Some(f as i64)
                } else {
                    None
                }
            })
        });
        let r_int = constant.as_integer().or_else(|| {
            constant.as_number().and_then(|f| {
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    Some(f as i64)
                } else {
                    None
                }
            })
        });

        if let (Some(l), Some(r)) = (l_int, r_int) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l & r);
            *pc += 1;
        }
    }
}

/// BORK: R[A] = R[B] | K[C]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
pub fn exec_bork(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        // FAST PATH: Direct constant access via cached pointer
        let constant = *(*frame_ptr).constants_ptr.add(c);

        let l_int = left.as_integer().or_else(|| {
            left.as_number().and_then(|f| {
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    Some(f as i64)
                } else {
                    None
                }
            })
        });
        let r_int = constant.as_integer().or_else(|| {
            constant.as_number().and_then(|f| {
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    Some(f as i64)
                } else {
                    None
                }
            })
        });

        if let (Some(l), Some(r)) = (l_int, r_int) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l | r);
            *pc += 1;
        }
    }
}

/// BXORK: R[A] = R[B] ~ K[C]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
pub fn exec_bxork(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        // FAST PATH: Direct constant access via cached pointer
        let constant = *(*frame_ptr).constants_ptr.add(c);

        let l_int = left.as_integer().or_else(|| {
            left.as_number().and_then(|f| {
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    Some(f as i64)
                } else {
                    None
                }
            })
        });
        let r_int = constant.as_integer().or_else(|| {
            constant.as_number().and_then(|f| {
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    Some(f as i64)
                } else {
                    None
                }
            })
        });

        if let (Some(l), Some(r)) = (l_int, r_int) {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(l ^ r);
            *pc += 1;
        }
    }
}

/// SHRI: R[A] = R[B] >> sC
#[inline(always)]
pub fn exec_shri(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let sc = get_sc!(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + b);

        if let Some(l) = left.as_integer() {
            let result = if sc >= 0 {
                LuaValue::integer(l >> (sc & 63))
            } else {
                LuaValue::integer(l << ((-sc) & 63))
            };
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
            *pc += 1;
        }
    }
}

/// SHLI: R[A] = sC << R[B]
#[inline(always)]
pub fn exec_shli(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let sc = get_sc!(instr);

    unsafe {
        let right = *vm.register_stack.as_ptr().add(base_ptr + b);

        if let Some(r) = right.as_integer() {
            let result = if r >= 0 {
                LuaValue::integer((sc as i64) << (r & 63))
            } else {
                LuaValue::integer((sc as i64) >> ((-r) & 63))
            };
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
            *pc += 1;
        }
    }
}

/// BNOT: R[A] = ~R[B]
#[inline(always)]
pub fn exec_bnot(vm: &mut LuaVM, instr: u32, base_ptr: usize) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);

    let value = vm.register_stack[base_ptr + b];

    if let Some(int_val) = value.as_integer() {
        vm.register_stack[base_ptr + a] = LuaValue::integer(!int_val);
        return Ok(());
    }

    // Try metamethod for non-integer values - use pre-cached __bnot StringId
    let mm_key = LuaValue::string(vm.object_pool.tm_bnot);
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

    Err(vm.error(format!(
        "attempt to perform bitwise operation on {}",
        value.type_name()
    )))
}

#[allow(dead_code)]
/// NOT: R[A] = not R[B]
#[inline(always)]
pub fn exec_not(vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);

    unsafe {
        let value = *vm.register_stack.as_ptr().add(base_ptr + b);

        // In Lua, only nil and false are falsy
        use crate::lua_value::{TAG_NIL, VALUE_FALSE};
        let is_falsy = value.primary == TAG_NIL || value.primary == VALUE_FALSE;
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::boolean(is_falsy);
    }
}

/// LEN: R[A] = #R[B]
/// OPTIMIZED: Fast path for tables without __len metamethod
#[inline(always)]
pub fn exec_len(vm: &mut LuaVM, instr: u32, base_ptr: usize) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);

    let value = vm.register_stack[base_ptr + b];

    // Fast path: string length - no metamethod
    if let Some(string_id) = value.as_string_id() {
        if let Some(s) = vm.object_pool.get_string(string_id) {
            vm.register_stack[base_ptr + a] = LuaValue::integer(s.as_str().len() as i64);
            return Ok(());
        }
    }

    // Table length with metamethod support
    if let Some(table_id) = value.as_table_id() {
        use crate::lua_value::tm_flags;

        // Single table access to get both length and metatable info
        let table = match vm.object_pool.get_table(table_id) {
            Some(t) => t,
            None => return Err(vm.error("invalid table")),
        };

        // FAST PATH: No metatable
        let mt_val = match table.get_metatable() {
            None => {
                let len = table.len() as i64;
                vm.register_stack[base_ptr + a] = LuaValue::integer(len);
                return Ok(());
            }
            Some(mt) => mt,
        };

        // Has metatable - check for __len
        let mt_id = match mt_val.as_table_id() {
            Some(id) => id,
            None => {
                let len = table.len() as i64;
                vm.register_stack[base_ptr + a] = LuaValue::integer(len);
                return Ok(());
            }
        };

        // Get both fasttm flag and __len value in one lookup
        let (len, len_mm) = {
            let mt_table = match vm.object_pool.get_table(mt_id) {
                Some(t) => t,
                None => {
                    let len = table.len() as i64;
                    return Ok({
                        vm.register_stack[base_ptr + a] = LuaValue::integer(len);
                    });
                }
            };

            // FAST PATH: fasttm check - __len is known to be absent
            if mt_table.tm_absent(tm_flags::TM_LEN) {
                (table.len() as i64, None)
            } else {
                let mm_key = LuaValue::string(vm.object_pool.tm_len);
                match mt_table.raw_get(&mm_key) {
                    Some(mm) if !mm.is_nil() => (0, Some(mm)),
                    _ => (table.len() as i64, None),
                }
            }
        };

        if let Some(metamethod) = len_mm {
            // Call __len metamethod
            let result = vm
                .call_metamethod(&metamethod, &[value])?
                .unwrap_or(LuaValue::nil());
            vm.register_stack[base_ptr + a] = result;
        } else {
            // Cache __len absence for future lookups
            if let Some(mt_table) = vm.object_pool.get_table_mut(mt_id) {
                mt_table.set_tm_absent(tm_flags::TM_LEN);
            }
            vm.register_stack[base_ptr + a] = LuaValue::integer(len);
        }
        return Ok(());
    }

    Err(vm.error(format!("attempt to get length of {}", value.type_name())))
}

/// MmBin: Metamethod binary operation (register, register)
/// OPTIMIZED: Uses passed code_ptr instead of dereferencing frame_ptr
#[inline(always)]
pub fn exec_mmbin(
    vm: &mut LuaVM,
    instr: u32,
    code_ptr: *const u32,
    pc: &mut usize,
    base_ptr: usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    unsafe {
        let prev_pc = *pc - 1;

        if prev_pc == 0 {
            return Ok(());
        }

        // Get previous instruction to find destination register
        let prev_instr = code_ptr.add(prev_pc - 1).read();
        let dest_reg = get_a!(prev_instr);

        let ra = *vm.register_stack.as_ptr().add(base_ptr + a);
        let rb = *vm.register_stack.as_ptr().add(base_ptr + b);

        // Use pre-cached metamethod StringId
        let mm_key = LuaValue::string(vm.object_pool.get_binop_tm(c as u8));

        let metamethod = if let Some(mt) = vm.table_get_metatable(&ra) {
            vm.table_get_with_meta(&mt, &mm_key)
                .unwrap_or(LuaValue::nil())
        } else if let Some(mt) = vm.table_get_metatable(&rb) {
            vm.table_get_with_meta(&mt, &mm_key)
                .unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        };

        if !metamethod.is_nil() {
            if let Some(result) = vm.call_metamethod(&metamethod, &[ra, rb])? {
                *vm.register_stack.as_mut_ptr().add(base_ptr + dest_reg) = result;
            }
        }
        // If no metamethod, leave the instruction result as-is (error will be caught elsewhere)
    }
    Ok(())
}

/// MmBinI: Metamethod binary operation (register, immediate)
/// OPTIMIZED: Uses passed code_ptr instead of dereferencing frame_ptr
#[inline(always)]
pub fn exec_mmbini(
    vm: &mut LuaVM,
    instr: u32,
    code_ptr: *const u32,
    pc: &mut usize,
    base_ptr: usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let sb = get_sb!(instr);
    let c = get_c!(instr);
    let k = get_k!(instr);

    unsafe {
        let prev_pc = *pc - 1;

        if prev_pc == 0 {
            return Ok(());
        }

        let prev_instr = code_ptr.add(prev_pc - 1).read();
        let dest_reg = get_a!(prev_instr);

        let rb = *vm.register_stack.as_ptr().add(base_ptr + a);
        let rc = LuaValue::integer(sb as i64);

        // Use pre-cached metamethod StringId
        let mm_key = LuaValue::string(vm.object_pool.get_binop_tm(c as u8));

        let metamethod = if let Some(mt) = vm.table_get_metatable(&rb) {
            vm.table_get_with_meta(&mt, &mm_key)
                .unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        };

        if !metamethod.is_nil() {
            let args = if k { vec![rc, rb] } else { vec![rb, rc] };
            if let Some(result) = vm.call_metamethod(&metamethod, &args)? {
                *vm.register_stack.as_mut_ptr().add(base_ptr + dest_reg) = result;
            }
        }
    }
    Ok(())
}

/// MmBinK: Metamethod binary operation (register, constant)
/// OPTIMIZED: Uses passed code_ptr and constants_ptr instead of dereferencing frame_ptr
#[inline(always)]
pub fn exec_mmbink(
    vm: &mut LuaVM,
    instr: u32,
    code_ptr: *const u32,
    constants_ptr: *const LuaValue,
    pc: &mut usize,
    base_ptr: usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);
    let k = get_k!(instr);

    unsafe {
        let prev_pc = *pc - 1;

        if prev_pc == 0 {
            return Ok(());
        }

        let prev_instr = code_ptr.add(prev_pc - 1).read();
        let dest_reg = get_a!(prev_instr);

        let ra = *vm.register_stack.as_ptr().add(base_ptr + a);

        // Get constant via passed constants_ptr
        let kb = *constants_ptr.add(b);

        // Use pre-cached metamethod StringId
        let mm_key = LuaValue::string(vm.object_pool.get_binop_tm(c as u8));

        let (left, right) = if !k { (ra, kb) } else { (kb, ra) };

        let metamethod = if let Some(mt) = vm.table_get_metatable(&left) {
            vm.table_get_with_meta(&mt, &mm_key)
                .unwrap_or(LuaValue::nil())
        } else if let Some(mt) = vm.table_get_metatable(&right) {
            vm.table_get_with_meta(&mt, &mm_key)
                .unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        };

        if !metamethod.is_nil() {
            if let Some(result) = vm.call_metamethod(&metamethod, &[left, right])? {
                *vm.register_stack.as_mut_ptr().add(base_ptr + dest_reg) = result;
            }
        }
    }
    Ok(())
}
