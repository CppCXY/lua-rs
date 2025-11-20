/// Arithmetic and logical instructions
///
/// These instructions handle arithmetic operations, bitwise operations, and comparisons.

use super::DispatchAction;
use crate::{
    LuaValue,
    lua_vm::{Instruction, LuaError, LuaResult, LuaVM},
};

/// ADD: R[A] = R[B] + R[C]
pub fn exec_add(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let result = if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
        // Integer addition with overflow check
        if let Some(sum) = l.checked_add(r) {
            LuaValue::integer(sum)
        } else {
            // Overflow: convert to float
            LuaValue::number(l as f64 + r as f64)
        }
    } else {
        // Float addition
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to add {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        let r_float = right.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to add {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        LuaValue::number(l_float + r_float)
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
}

/// SUB: R[A] = R[B] - R[C]
pub fn exec_sub(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let result = if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
        if let Some(diff) = l.checked_sub(r) {
            LuaValue::integer(diff)
        } else {
            LuaValue::number(l as f64 - r as f64)
        }
    } else {
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to subtract {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        let r_float = right.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to subtract {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        LuaValue::number(l_float - r_float)
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
}

/// MUL: R[A] = R[B] * R[C]
pub fn exec_mul(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];
    let right = vm.register_stack[base_ptr + c];

    let result = if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
        if let Some(prod) = l.checked_mul(r) {
            LuaValue::integer(prod)
        } else {
            LuaValue::number(l as f64 * r as f64)
        }
    } else {
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to multiply {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        let r_float = right.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to multiply {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        LuaValue::number(l_float * r_float)
    };

    vm.register_stack[base_ptr + a] = result;
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
    let l_float = left.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to divide {} with {}",
            left.type_name(),
            right.type_name()
        ))
    })?;
    let r_float = right.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to divide {} with {}",
            left.type_name(),
            right.type_name()
        ))
    })?;

    let result = LuaValue::number(l_float / r_float);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to divide {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        let r_float = right.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to divide {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        LuaValue::number((l_float / r_float).floor())
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to perform modulo on {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        let r_float = right.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to perform modulo on {} with {}",
                left.type_name(),
                right.type_name()
            ))
        })?;
        // Lua uses floored division modulo: a % b = a - floor(a/b) * b
        let result = l_float - (l_float / r_float).floor() * r_float;
        LuaValue::number(result)
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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
    let l_float = left.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to exponentiate {} with {}",
            left.type_name(),
            right.type_name()
        ))
    })?;
    let r_float = right.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to exponentiate {} with {}",
            left.type_name(),
            right.type_name()
        ))
    })?;

    let result = LuaValue::number(l_float.powf(r_float));
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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
        return Err(LuaError::RuntimeError(format!(
            "attempt to negate {}",
            value.type_name()
        )));
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
}

// ============ Arithmetic Immediate Instructions ============

/// ADDI: R[A] = R[B] + sC
pub fn exec_addi(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let sc = Instruction::get_sc(instr); // Signed immediate value

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let left = vm.register_stack[base_ptr + b];

    let result = if let Some(l) = left.as_integer() {
        if let Some(sum) = l.checked_add(sc as i64) {
            LuaValue::integer(sum)
        } else {
            LuaValue::number(l as f64 + sc as f64)
        }
    } else {
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!("attempt to add {} with number", left.type_name()))
        })?;
        LuaValue::number(l_float + sc as f64)
    };

    vm.register_stack[base_ptr + a] = result;
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

    let result = if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if let Some(sum) = l.checked_add(r) {
            LuaValue::integer(sum)
        } else {
            LuaValue::number(l as f64 + r as f64)
        }
    } else {
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to add {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        let r_float = constant.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to add {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        LuaValue::number(l_float + r_float)
    };

    vm.register_stack[base_ptr + a] = result;
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

    let result = if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if let Some(diff) = l.checked_sub(r) {
            LuaValue::integer(diff)
        } else {
            LuaValue::number(l as f64 - r as f64)
        }
    } else {
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to subtract {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        let r_float = constant.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to subtract {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        LuaValue::number(l_float - r_float)
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
}

/// MULK: R[A] = R[B] * K[C]
pub fn exec_mulk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
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

    let result = if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if let Some(prod) = l.checked_mul(r) {
            LuaValue::integer(prod)
        } else {
            LuaValue::number(l as f64 * r as f64)
        }
    } else {
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to multiply {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        let r_float = constant.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to multiply {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        LuaValue::number(l_float * r_float)
    };

    vm.register_stack[base_ptr + a] = result;
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

    let result = if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if r == 0 {
            return Err(LuaError::RuntimeError(
                "attempt to perform 'n%0'".to_string(),
            ));
        }
        LuaValue::integer(l.rem_euclid(r))
    } else {
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to perform modulo on {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        let r_float = constant.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to perform modulo on {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        let result = l_float - (l_float / r_float).floor() * r_float;
        LuaValue::number(result)
    };

    vm.register_stack[base_ptr + a] = result;
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

    let l_float = left.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to exponentiate {} with {}",
            left.type_name(),
            constant.type_name()
        ))
    })?;
    let r_float = constant.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to exponentiate {} with {}",
            left.type_name(),
            constant.type_name()
        ))
    })?;

    let result = LuaValue::number(l_float.powf(r_float));
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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

    let l_float = left.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to divide {} with {}",
            left.type_name(),
            constant.type_name()
        ))
    })?;
    let r_float = constant.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to divide {} with {}",
            left.type_name(),
            constant.type_name()
        ))
    })?;

    let result = LuaValue::number(l_float / r_float);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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

    let result = if let (Some(l), Some(r)) = (left.as_integer(), constant.as_integer()) {
        if r == 0 {
            return Err(LuaError::RuntimeError(
                "attempt to divide by zero".to_string(),
            ));
        }
        LuaValue::integer(l.div_euclid(r))
    } else {
        let l_float = left.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to divide {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        let r_float = constant.as_number().ok_or_else(|| {
            LuaError::RuntimeError(format!(
                "attempt to divide {} with {}",
                left.type_name(),
                constant.type_name()
            ))
        })?;
        LuaValue::number((l_float / r_float).floor())
    };

    vm.register_stack[base_ptr + a] = result;
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

    let result = LuaValue::integer(l_int & r_int);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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

    let result = LuaValue::integer(l_int | r_int);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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

    let result = LuaValue::integer(l_int ^ r_int);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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
    Ok(DispatchAction::Continue)
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
        LuaValue::integer(l_int >> (r_int & 63))
    } else {
        LuaValue::integer(l_int << ((-r_int) & 63))
    };

    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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

    let l_int = left.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            left.type_name()
        ))
    })?;
    let r_int = constant.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            constant.type_name()
        ))
    })?;

    let result = LuaValue::integer(l_int & r_int);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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

    let l_int = left.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            left.type_name()
        ))
    })?;
    let r_int = constant.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            constant.type_name()
        ))
    })?;

    let result = LuaValue::integer(l_int | r_int);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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

    let l_int = left.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            left.type_name()
        ))
    })?;
    let r_int = constant.as_integer().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "attempt to perform bitwise operation on {}",
            constant.type_name()
        ))
    })?;

    let result = LuaValue::integer(l_int ^ r_int);
    vm.register_stack[base_ptr + a] = result;
    Ok(DispatchAction::Continue)
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
pub fn exec_len(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];

    let len = if let Some(table_id) = value.as_table_id() {
        let table = vm
            .object_pool
            .get_table(table_id)
            .ok_or_else(|| LuaError::RuntimeError("invalid table".to_string()))?;
        table.borrow().len() as i64
    } else if value.is_string() {
        if let Some(s) = vm.get_string(&value) {
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
