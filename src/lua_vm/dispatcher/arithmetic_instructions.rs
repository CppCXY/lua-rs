/// Arithmetic and logical instructions
///
/// These instructions handle arithmetic operations, bitwise operations, and comparisons.

use super::DispatchAction;
use crate::{
    LuaValue,
    lua_vm::{Instruction, LuaError, LuaResult, LuaVM},
    lua_value::{TAG_INTEGER, TAG_FLOAT, TYPE_MASK},
};

/// ADD: R[A] = R[B] + R[C]
#[inline(always)]
pub fn exec_add(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access
    let (left, right) = unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        (*reg_base.add(b), *reg_base.add(c))
    };

    let left_tag = left.primary & TYPE_MASK;
    let right_tag = right.primary & TYPE_MASK;
    
    // Fast path 1: both integers
    let result = if left_tag == TAG_INTEGER && right_tag == TAG_INTEGER {
        LuaValue::integer((left.secondary as i64).wrapping_add(right.secondary as i64))
    }
    // Fast path 2: both floats
    else if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
        LuaValue::number(f64::from_bits(left.secondary) + f64::from_bits(right.secondary))
    }
    // Mixed: integer + float
    else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
        LuaValue::number((left.secondary as i64) as f64 + f64::from_bits(right.secondary))
    }
    // Mixed: float + integer
    else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
        LuaValue::number(f64::from_bits(left.secondary) + (right.secondary as i64) as f64)
    }
    else {
        return add_error(left, right);
    };

    unsafe {
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
    }
    Ok(DispatchAction::Skip(1))
}

#[cold]
#[inline(never)]
fn add_error(_left: LuaValue, _right: LuaValue) -> LuaResult<DispatchAction> {
    // Don't throw error - let MMBIN handle metamethod
    Ok(DispatchAction::Continue)
}

/// SUB: R[A] = R[B] - R[C]
#[inline(always)]
pub fn exec_sub(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let left_tag = left.primary & TYPE_MASK;
    let right_tag = right.primary & TYPE_MASK;
    
    // Fast path 1: both integers
    let result = if left_tag == TAG_INTEGER && right_tag == TAG_INTEGER {
        LuaValue::integer((left.secondary as i64).wrapping_sub(right.secondary as i64))
    }
    // Fast path 2: both floats
    else if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
        LuaValue::number(f64::from_bits(left.secondary) - f64::from_bits(right.secondary))
    }
    // Mixed: integer - float
    else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
        LuaValue::number((left.secondary as i64) as f64 - f64::from_bits(right.secondary))
    }
    // Mixed: float - integer
    else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
        LuaValue::number(f64::from_bits(left.secondary) - (right.secondary as i64) as f64)
    }
    else {
        return sub_error(left, right);
    };

    unsafe {
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = result;
    }
    Ok(DispatchAction::Skip(1))
}

#[cold]
#[inline(never)]
fn sub_error(_left: LuaValue, _right: LuaValue) -> LuaResult<DispatchAction> {
    // Don't throw error - let MMBIN handle metamethod
    Ok(DispatchAction::Continue)
}

/// MUL: R[A] = R[B] * R[C]
#[inline(always)]
pub fn exec_mul(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let left_tag = left.primary & TYPE_MASK;
    let right_tag = right.primary & TYPE_MASK;
    
    // Fast path 1: both are integers (most common, predict taken)
    let result = if left_tag == TAG_INTEGER && right_tag == TAG_INTEGER {
        LuaValue::integer((left.secondary as i64).wrapping_mul(right.secondary as i64))
    }
    // Fast path 2: both are floats
    else if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
        LuaValue::number(f64::from_bits(left.secondary) * f64::from_bits(right.secondary))
    }
    // Mixed: integer * float
    else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
        LuaValue::number((left.secondary as i64) as f64 * f64::from_bits(right.secondary))
    }
    // Mixed: float * integer
    else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
        LuaValue::number(f64::from_bits(left.secondary) * (right.secondary as i64) as f64)
    }
    else {
        return mul_error(left, right);
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Skip(1))
}

#[cold]
#[inline(never)]
fn mul_error(_left: LuaValue, _right: LuaValue) -> LuaResult<DispatchAction> {
    // Don't throw error - let MMBIN handle metamethod
    Ok(DispatchAction::Continue)
}

/// DIV: R[A] = R[B] / R[C]
pub fn exec_div(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    // Division always returns float in Lua
    let l_float = match left.as_number() {
        Some(n) => n,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };
    let r_float = match right.as_number() {
        Some(n) => n,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };

    let result = LuaValue::number(l_float / r_float);
    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// IDIV: R[A] = R[B] // R[C] (floor division)
pub fn exec_idiv(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let result = if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
        if r == 0 {
            return Err(LuaError::RuntimeError(
                "attempt to divide by zero".to_string(),
            ));
        }
        // Integer floor division
        let quot = l.div_euclid(r);
        LuaValue::integer(quot)
    } else {
        let l_float = match left.as_number() {
            Some(n) => n,
            None => return Ok(DispatchAction::Continue), // Let MMBIN handle
        };
        let r_float = match right.as_number() {
            Some(n) => n,
            None => return Ok(DispatchAction::Continue), // Let MMBIN handle
        };
        LuaValue::number((l_float / r_float).floor())
    };

    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// MOD: R[A] = R[B] % R[C]
pub fn exec_mod(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let result = if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
        if r == 0 {
            return Err(LuaError::RuntimeError(
                "attempt to perform 'n%0'".to_string(),
            ));
        }
        LuaValue::integer(l.rem_euclid(r))
    } else {
        let l_float = match left.as_number() {
            Some(n) => n,
            None => return Ok(DispatchAction::Continue), // Let MMBIN handle
        };
        let r_float = match right.as_number() {
            Some(n) => n,
            None => return Ok(DispatchAction::Continue), // Let MMBIN handle
        };
        // Lua uses floored division modulo: a % b = a - floor(a/b) * b
        let result = l_float - (l_float / r_float).floor() * r_float;
        LuaValue::number(result)
    };

    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// POW: R[A] = R[B] ^ R[C]
pub fn exec_pow(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    // Power always uses float
    let l_float = match left.as_number() {
        Some(n) => n,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };
    let r_float = match right.as_number() {
        Some(n) => n,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };

    let result = LuaValue::number(l_float.powf(r_float));
    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// UNM: R[A] = -R[B] (unary minus)
pub fn exec_unm(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
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
        // Let MMBIN handle metamethod
        return Ok(DispatchAction::Continue);
    };

    vm.register_stack[base_ptr + a] = result;
    // Skip MMBIN if successful
    Ok(DispatchAction::Skip(1))
}

// ============ Arithmetic Immediate Instructions ============

/// ADDI: R[A] = R[B] + sC
#[inline(always)]
pub fn exec_addi(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let sc = Instruction::get_sc(instr); // Signed immediate value

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];

    if left.primary == TAG_INTEGER {
        // Integer operation with wraparound
        let l = left.secondary as i64;
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.wrapping_add(sc as i64));
        // Skip the following MMBINI instruction (PC already incremented by main loop)
        vm.current_frame_mut().pc += 1;
        return Ok(DispatchAction::Continue);
    }
    
    if left.primary == TAG_FLOAT {
        // Float operation
        let l = f64::from_bits(left.secondary);
        vm.register_stack[base_ptr + a] = LuaValue::float(l + sc as f64);
        // Skip the following MMBINI instruction (PC already incremented by main loop)
        vm.current_frame_mut().pc += 1;
        return Ok(DispatchAction::Continue);
    }
    
    // Not a number, fallthrough to MMBINI without setting result
    // MMBINI will be next instruction and handle metamethod
    Ok(DispatchAction::Continue)
}

/// ADDK: R[A] = R[B] + K[C]
pub fn exec_addk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant from chunk
    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try integer operation
    if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        // Integer operation with wraparound, skip fallback
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.wrapping_add(r));
        return Ok(DispatchAction::Skip(1));  // Skip MMBIN fallback
    }
    
    // Try float operation
    if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
        // Float operation succeeded, skip fallback
        vm.register_stack[base_ptr + a] = LuaValue::number(l + r);
        return Ok(DispatchAction::Skip(1));  // Skip MMBIN fallback
    }
    
    // Not numbers, fallthrough to MMBIN
    Ok(DispatchAction::Continue)
}

/// SUBK: R[A] = R[B] - K[C]
pub fn exec_subk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try integer operation
    if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.wrapping_sub(r));
        return Ok(DispatchAction::Skip(1));  // Skip MMBIN fallback
    }
    
    // Try float operation
    if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
        vm.register_stack[base_ptr + a] = LuaValue::number(l - r);
        return Ok(DispatchAction::Skip(1));  // Skip MMBIN fallback
    }
    Ok(DispatchAction::Continue)
}

/// MULK: R[A] = R[B] * K[C]
#[inline(always)]
pub fn exec_mulk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    // Get constant once
    let func_ptr = match frame.get_function_ptr() {
        Some(ptr) => ptr,
        None => return Err(LuaError::RuntimeError("Not a Lua function".to_string())),
    };
    let func = unsafe { &*func_ptr };
    let constants = &func.borrow().chunk.constants;
    let constant = match constants.get(c) {
        Some(val) => *val,
        None => return Err(LuaError::RuntimeError(format!("Invalid constant index: {}", c))),
    };

    let left = vm.register_stack[base_ptr + b];

    // Fast path: Direct type tag check (avoid method call overhead)
    
    // Check if both are integers
    if left.primary == TAG_INTEGER && constant.primary == TAG_INTEGER {
        let l = left.secondary as i64;
        let r = constant.secondary as i64;
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.wrapping_mul(r));
        return Ok(DispatchAction::Skip(1));
    }
    
    // Check if at least one is float (covers float*float, float*int, int*float)
    if left.primary == TAG_FLOAT || constant.primary == TAG_FLOAT {
        // Convert to float
        let l = if left.primary == TAG_FLOAT {
            f64::from_bits(left.secondary)
        } else if left.primary == TAG_INTEGER {
            left.secondary as i64 as f64
        } else {
            return Ok(DispatchAction::Continue); // Not a number
        };
        
        let r = if constant.primary == TAG_FLOAT {
            f64::from_bits(constant.secondary)
        } else if constant.primary == TAG_INTEGER {
            constant.secondary as i64 as f64
        } else {
            return Ok(DispatchAction::Continue); // Not a number
        };
        
        vm.register_stack[base_ptr + a] = LuaValue::float(l * r);
        return Ok(DispatchAction::Skip(1));
    }

    Ok(DispatchAction::Continue)
}

/// MODK: R[A] = R[B] % K[C]
pub fn exec_modk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try integer operation
    if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if r == 0 {
            return Err(LuaError::RuntimeError(
                "attempt to perform 'n%0'".to_string(),
            ));
        }
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.rem_euclid(r));
        return Ok(DispatchAction::Skip(1));  // Skip MMBIN fallback
    }
    
    // Try float operation
    if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
        let result = l - (l / r).floor() * r;
        vm.register_stack[base_ptr + a] = LuaValue::number(result);
        return Ok(DispatchAction::Skip(1));  // Skip MMBIN fallback
    }
    Ok(DispatchAction::Continue)
}

/// POWK: R[A] = R[B] ^ K[C]
pub fn exec_powk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try float operation
    let l_float = match left.as_number() {
        Some(n) => n,
        None => return Ok(DispatchAction::Continue), // Let MMBINK handle
    };
    let r_float = match constant.as_number() {
        Some(n) => n,
        None => return Ok(DispatchAction::Continue), // Let MMBINK handle
    };

    let result = LuaValue::number(l_float.powf(r_float));
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Skip(1))  // Skip MMBIN fallback
}

/// DIVK: R[A] = R[B] / K[C]
pub fn exec_divk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try to perform the operation; if not possible, let MMBINK handle it
    let l_float = match left.as_number() {
        Some(n) => n,
        None => return Ok(DispatchAction::Continue), // Let MMBINK handle
    };
    let r_float = match constant.as_number() {
        Some(n) => n,
        None => return Ok(DispatchAction::Continue), // Let MMBINK handle
    };

    let result = LuaValue::number(l_float / r_float);
    vm.register_stack[base_ptr + a] = result;
    // Skip next instruction (MMBINK fallback) if operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// IDIVK: R[A] = R[B] // K[C]
pub fn exec_idivk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try integer operation
    if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if r == 0 {
            return Err(LuaError::RuntimeError(
                "attempt to divide by zero".to_string(),
            ));
        }
        vm.register_stack[base_ptr + a] = LuaValue::integer(l.div_euclid(r));
        return Ok(DispatchAction::Skip(1));  // Skip MMBIN fallback
    }
    
    // Try float operation
    if let (Some(l), Some(r)) = (left.as_number(), constant.as_number()) {
        vm.register_stack[base_ptr + a] = LuaValue::number((l / r).floor());
        return Ok(DispatchAction::Skip(1));  // Skip MMBIN fallback
    }
    Ok(DispatchAction::Continue)
}

// ============ Bitwise Operations ============

/// BAND: R[A] = R[B] & R[C]
pub fn exec_band(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };
    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };

    let result = LuaValue::integer(l_int & r_int);
    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// BOR: R[A] = R[B] | R[C]
pub fn exec_bor(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };
    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };

    let result = LuaValue::integer(l_int | r_int);
    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// BXOR: R[A] = R[B] ~ R[C]
pub fn exec_bxor(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };
    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };

    let result = LuaValue::integer(l_int ^ r_int);
    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// SHL: R[A] = R[B] << R[C]
pub fn exec_shl(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = left.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            left.type_name()
        ))
    })?;
    let r_int = right.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            right.type_name()
        ))
    })?;

    let result = if r_int >= 0 {
        LuaValue::integer(l_int << (r_int & 63))
    } else {
        LuaValue::integer(l_int >> ((-r_int) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// SHR: R[A] = R[B] >> R[C]
pub fn exec_shr(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let l_int = match left.as_integer() {
        Some(i) => i,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };
    let r_int = match right.as_integer() {
        Some(i) => i,
        None => return Ok(DispatchAction::Continue), // Let MMBIN handle
    };

    let result = if r_int >= 0 {
        LuaValue::integer(l_int >> (r_int & 63))
    } else {
        LuaValue::integer(l_int << ((-r_int) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    // Skip the following MMBIN instruction since the operation succeeded
    Ok(DispatchAction::Skip(1))
}

/// BANDK: R[A] = R[B] & K[C]
pub fn exec_bandk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try to get integer from left operand (may convert from float)
    let l_int = if let Some(i) = left.as_integer() {
        i
    } else if let Some(f) = left.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(DispatchAction::Continue);
        }
    } else {
        return Ok(DispatchAction::Continue);
    };
    
    // Try to get integer from constant
    let r_int = if let Some(i) = constant.as_integer() {
        i
    } else if let Some(f) = constant.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(DispatchAction::Continue);
        }
    } else {
        return Ok(DispatchAction::Continue);
    };

    vm.register_stack[base_ptr + a] = LuaValue::integer(l_int & r_int);
    Ok(DispatchAction::Skip(1))  // Skip MMBIN fallback
}

/// BORK: R[A] = R[B] | K[C]
pub fn exec_bork(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try to get integer from left operand (may convert from float)
    let l_int = if let Some(i) = left.as_integer() {
        i
    } else if let Some(f) = left.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(DispatchAction::Continue);
        }
    } else {
        return Ok(DispatchAction::Continue);
    };
    
    // Try to get integer from constant
    let r_int = if let Some(i) = constant.as_integer() {
        i
    } else if let Some(f) = constant.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(DispatchAction::Continue);
        }
    } else {
        return Ok(DispatchAction::Continue);
    };

    vm.register_stack[base_ptr + a] = LuaValue::integer(l_int | r_int);
    Ok(DispatchAction::Skip(1))  // Skip MMBIN fallback
}

/// BXORK: R[A] = R[B] ~ K[C]
pub fn exec_bxork(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(c)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", c)))?;

    let left = vm.register_stack[base_ptr + b];

    // Try to get integer from left operand (may convert from float)
    let l_int = if let Some(i) = left.as_integer() {
        i
    } else if let Some(f) = left.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(DispatchAction::Continue);
        }
    } else {
        return Ok(DispatchAction::Continue);
    };
    
    // Try to get integer from constant
    let r_int = if let Some(i) = constant.as_integer() {
        i
    } else if let Some(f) = constant.as_number() {
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            f as i64
        } else {
            return Ok(DispatchAction::Continue);
        }
    } else {
        return Ok(DispatchAction::Continue);
    };

    vm.register_stack[base_ptr + a] = LuaValue::integer(l_int ^ r_int);
    Ok(DispatchAction::Skip(1))  // Skip MMBIN fallback
}

/// SHRI: R[A] = R[B] >> sC
pub fn exec_shri(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let sc = Instruction::get_sc(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];

    let l_int = left.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            left.type_name()
        ))
    })?;

    let result = if sc >= 0 {
        LuaValue::integer(l_int >> (sc & 63))
    } else {
        LuaValue::integer(l_int << ((-sc) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
}

/// SHLI: R[A] = sC << R[B]
pub fn exec_shli(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let sc = Instruction::get_sc(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let right = vm.register_stack[base_ptr + b];

    let r_int = right.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            right.type_name()
        ))
    })?;

    let result = if r_int >= 0 {
        LuaValue::integer((sc as i64) << (r_int & 63))
    } else {
        LuaValue::integer((sc as i64) >> ((-r_int) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
}

/// BNOT: R[A] = ~R[B]
pub fn exec_bnot(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];

    let int_val = value.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            value.type_name()
        ))
    })?;

    let result = LuaValue::integer(!int_val);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
}

/// NOT: R[A] = not R[B]
pub fn exec_not(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];
    let result = LuaValue::boolean(!(value.is_nil() || (value.as_bool().unwrap_or(true))));

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
}

/// LEN: R[A] = #R[B]
#[inline(always)]
pub fn exec_len(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];

    // CRITICAL OPTIMIZATION: Use direct pointer instead of HashMap lookup!
    let len = if let Some(table_ptr) = value.as_table_ptr() {
        unsafe { (*table_ptr).borrow().len() as i64 }
    } else if value.is_string() {
        if let Some(s) = value.as_lua_string() {
            s.as_str().len() as i64
        } else {
            0
        }
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to get length of {}",
            value.type_name()
        )));
    };

    let result = LuaValue::integer(len);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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
pub fn exec_mmbin(vm: &mut LuaVM, instr: u32) -> Result<DispatchAction, LuaError> {
    let a = Instruction::get_a(instr) as usize;  // operand 1
    let b = Instruction::get_b(instr) as usize;  // operand 2
    let c = Instruction::get_c(instr) as usize;  // C is the TagMethod index
    // k bit is unused for MMBIN
    
    // Get the previous instruction to find the destination register
    let frame = vm.current_frame();
    let func_ptr = frame.get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    let chunk = &func_ref.chunk;
    let prev_pc = frame.pc - 1;  // Previous instruction was the failed arithmetic op
    
    if prev_pc == 0 {
        return Err(LuaError::RuntimeError("MMBIN: no previous instruction".to_string()));
    }
    
    let prev_instr = chunk.code[prev_pc - 1];
    let dest_reg = Instruction::get_a(prev_instr) as usize;  // Destination register from ADD/SUB/etc
    
    let base_ptr = frame.base_ptr;
    // A and B are the operand registers
    let ra = vm.register_stack[base_ptr + a];
    let rb = vm.register_stack[base_ptr + b];
    
    // C is the metamethod index (TagMethod enum value)
    let metamethod_name = get_binop_metamethod(c as u8);
    let mm_key = vm.create_string(metamethod_name);
    
    // Try to get metamethod from first operand's metatable
    let metamethod = if let Some(mt) = vm.table_get_metatable(&ra) {
        vm.table_get_with_meta(&mt, &mm_key).unwrap_or(LuaValue::nil())
    } else if let Some(mt) = vm.table_get_metatable(&rb) {
        vm.table_get_with_meta(&mt, &mm_key).unwrap_or(LuaValue::nil())
    } else {
        LuaValue::nil()
    };
    
    if metamethod.is_nil() {
        return Err(LuaError::RuntimeError(format!(
            "attempt to perform arithmetic on {} and {}",
            ra.type_name(),
            rb.type_name()
        )));
    }
    
    // Call metamethod
    let args = vec![ra, rb];
    let result = vm.call_metamethod(&metamethod, &args)?
        .unwrap_or(LuaValue::nil());
    // Store result in the destination register from the previous instruction
    vm.register_stack[base_ptr + dest_reg] = result;
    
    Ok(DispatchAction::Continue)
}

/// MmBinI: Metamethod binary operation (register, immediate)
pub fn exec_mmbini(vm: &mut LuaVM, instr: u32) -> Result<DispatchAction, LuaError> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);  // Signed immediate value (as in ADDI)
    let c = Instruction::get_c(instr);
    let k = Instruction::get_k(instr);
    
    // Get the previous instruction to find the destination register
    let frame = vm.current_frame();
    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    let chunk = &func_ref.chunk;
    let prev_pc = frame.pc - 1;  // Previous instruction was the failed arithmetic op
    
    if prev_pc == 0 {
        return Err(LuaError::RuntimeError("MMBINI: no previous instruction".to_string()));
    }
    
    let prev_instr = chunk.code[prev_pc - 1];
    let dest_reg = Instruction::get_a(prev_instr) as usize;  // Destination register from ADDI
    
    let base_ptr = frame.base_ptr;
    
    // From compiler: create_abck(OpCode::MmBinI, left_reg, imm, TagMethod, false)
    // So: A = left_reg (operand), B = imm (immediate value encoded), C = tagmethod
    
    let rb = vm.register_stack[base_ptr + a];  // A contains the operand register content
    let rc = LuaValue::integer(sb as i64);  // B field contains the immediate value
    
    let metamethod_name = get_binop_metamethod(c as u8);
    let mm_key = vm.create_string(metamethod_name);
    
    // Try to get metamethod from operand's metatable
    let metamethod = if let Some(mt) = vm.table_get_metatable(&rb) {
        vm.table_get_with_meta(&mt, &mm_key).unwrap_or(LuaValue::nil())
    } else {
        LuaValue::nil()
    };
    
    if metamethod.is_nil() {
        return Err(LuaError::RuntimeError(format!(
            "attempt to perform arithmetic on {} and {}",
            rb.type_name(),
            rc.type_name()
        )));
    }
    
    // Call metamethod
    let args = if k { vec![rc, rb] } else { vec![rb, rc] };
    let result = vm.call_metamethod(&metamethod, &args)?
        .unwrap_or(LuaValue::nil());
    vm.register_stack[base_ptr + dest_reg] = result;
    
    Ok(DispatchAction::Continue)
}

/// MmBinK: Metamethod binary operation (register, constant)
pub fn exec_mmbink(vm: &mut LuaVM, instr: u32) -> Result<DispatchAction, LuaError> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;  // C is the TagMethod
    let k = Instruction::get_k(instr);            // k is the flip flag
    
    // Get the previous instruction to find the destination register
    let frame = vm.current_frame();
    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    let chunk = &func_ref.chunk;
    let prev_pc = frame.pc - 1;  // Previous instruction was the failed arithmetic op
    
    if prev_pc == 0 {
        return Err(LuaError::RuntimeError("MMBINK: no previous instruction".to_string()));
    }
    
    let prev_instr = chunk.code[prev_pc - 1];
    let dest_reg = Instruction::get_a(prev_instr) as usize;  // Destination register from ADDK/SUBK/etc
    
    let base_ptr = frame.base_ptr;
    
    let ra = vm.register_stack[base_ptr + a];
    let kb = chunk.constants.get(b).copied().ok_or_else(|| {
        LuaError::RuntimeError(format!("Constant index out of bounds: {}", b))
    })?;
    
    // C is the TagMethod, not a constant index
    let metamethod_name = get_binop_metamethod(c as u8);
    let mm_key = vm.create_string(metamethod_name);
    
    // k is flip flag: if k==false then (ra, kb), if k==true then (kb, ra)
    let (left, right) = if !k {
        (ra, kb)
    } else {
        (kb, ra)
    };
    
    // Try to get metamethod from operands' metatables
    let metamethod = if let Some(mt) = vm.table_get_metatable(&left) {
        vm.table_get_with_meta(&mt, &mm_key).unwrap_or(LuaValue::nil())
    } else if let Some(mt) = vm.table_get_metatable(&right) {
        vm.table_get_with_meta(&mt, &mm_key).unwrap_or(LuaValue::nil())
    } else {
        LuaValue::nil()
    };
    
    if metamethod.is_nil() {
        return Err(LuaError::RuntimeError(format!(
            "attempt to perform arithmetic on {} and {}",
            left.type_name(),
            right.type_name()
        )));
    }
    
    // Call metamethod
    let args = vec![left, right];
    let result = vm.call_metamethod(&metamethod, &args)?
        .unwrap_or(LuaValue::nil());
    vm.register_stack[base_ptr + dest_reg] = result;
    
    Ok(DispatchAction::Continue)
}
