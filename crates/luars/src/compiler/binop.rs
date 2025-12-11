//! Binary operation compilation helpers (OLD - kept for potential optimizations)
//!
//! These functions are currently unused after refactoring to use official Lua's
//! infix/posfix strategy in binop_infix.rs. They are kept as reference for
//! future constant folding and immediate optimizations.

#![allow(dead_code)]

use super::exp2reg::exp_to_any_reg;
use super::expdesc::*;
use super::helpers::*;
use super::{Compiler, TagMethod};
use crate::lua_value::LuaValue;
use crate::lua_vm::{Instruction, OpCode};
use emmylua_parser::BinaryOperator;

/// Emit arithmetic binary operation (add, sub, mul, div, idiv, mod, pow)
pub fn emit_arith_op(
    c: &mut Compiler,
    op: BinaryOperator,
    left_reg: u32,
    right_desc: &mut ExpDesc,
    can_reuse: bool,
) -> Result<u32, String> {
    let result_reg = if can_reuse { left_reg } else { alloc_register(c) };
    
    // Check if right is a constant
    let is_const = matches!(right_desc.kind, ExpKind::VKInt | ExpKind::VKFlt | ExpKind::VK);
    
    // Check for small integer immediate
    let imm_val = if let ExpKind::VKInt = right_desc.kind {
        let v = right_desc.ival;
        if v >= -256 && v <= 255 { Some(v) } else { None }
    } else {
        None
    };

    // Get opcode info
    let (op_rr, op_rk, tm) = match op {
        BinaryOperator::OpAdd => (OpCode::Add, Some(OpCode::AddK), TagMethod::Add),
        BinaryOperator::OpSub => (OpCode::Sub, Some(OpCode::SubK), TagMethod::Sub),
        BinaryOperator::OpMul => (OpCode::Mul, Some(OpCode::MulK), TagMethod::Mul),
        BinaryOperator::OpDiv => (OpCode::Div, Some(OpCode::DivK), TagMethod::Div),
        BinaryOperator::OpIDiv => (OpCode::IDiv, Some(OpCode::IDivK), TagMethod::IDiv),
        BinaryOperator::OpMod => (OpCode::Mod, Some(OpCode::ModK), TagMethod::Mod),
        BinaryOperator::OpPow => (OpCode::Pow, Some(OpCode::PowK), TagMethod::Pow),
        _ => return Err(format!("Not an arithmetic operator: {:?}", op)),
    };

    // Try immediate form for ADD/SUB
    if let Some(imm) = imm_val {
        if matches!(op, BinaryOperator::OpAdd) {
            let enc_imm = ((imm + 127) & 0xff) as u32;
            let mm_imm = ((imm + 128) & 0xff) as u32;
            emit(c, Instruction::encode_abc(OpCode::AddI, result_reg, left_reg, enc_imm));
            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, mm_imm, tm.as_u32(), false));
            return Ok(result_reg);
        } else if matches!(op, BinaryOperator::OpSub) {
            // SUB uses ADDI with negated immediate
            let neg_imm = ((-imm + 127) & 0xff) as u32;
            let neg_mm = ((-imm + 128) & 0xff) as u32;
            emit(c, Instruction::encode_abc(OpCode::AddI, result_reg, left_reg, neg_imm));
            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, neg_mm, TagMethod::Sub.as_u32(), false));
            return Ok(result_reg);
        }
    }

    // Try constant form
    if is_const {
        if let Some(op_k) = op_rk {
            let const_idx = ensure_constant_idx(c, right_desc);
            emit(c, Instruction::encode_abc(op_k, result_reg, left_reg, const_idx));
            emit(c, Instruction::create_abck(OpCode::MmBinK, left_reg, const_idx, tm.as_u32(), false));
            return Ok(result_reg);
        }
    }

    // Fall back to register form
    let right_reg = exp_to_any_reg(c, right_desc);
    emit(c, Instruction::encode_abc(op_rr, result_reg, left_reg, right_reg));
    emit(c, Instruction::create_abck(OpCode::MmBin, left_reg, right_reg, tm.as_u32(), false));
    
    Ok(result_reg)
}

/// Emit bitwise binary operation (band, bor, bxor, shl, shr)
pub fn emit_bitwise_op(
    c: &mut Compiler,
    op: BinaryOperator,
    left_reg: u32,
    right_desc: &mut ExpDesc,
    can_reuse: bool,
) -> Result<u32, String> {
    let result_reg = if can_reuse { left_reg } else { alloc_register(c) };
    
    // Check for constant
    let is_const = matches!(right_desc.kind, ExpKind::VKInt | ExpKind::VK);
    
    // Check for shift immediate
    let shift_imm = if let ExpKind::VKInt = right_desc.kind {
        let v = right_desc.ival;
        if v >= -128 && v <= 127 { Some(v) } else { None }
    } else {
        None
    };

    let (op_rr, op_rk) = match op {
        BinaryOperator::OpBAnd => (OpCode::BAnd, Some(OpCode::BAndK)),
        BinaryOperator::OpBOr => (OpCode::BOr, Some(OpCode::BOrK)),
        BinaryOperator::OpBXor => (OpCode::BXor, Some(OpCode::BXorK)),
        BinaryOperator::OpShl => (OpCode::Shl, None),
        BinaryOperator::OpShr => (OpCode::Shr, None),
        _ => return Err(format!("Not a bitwise operator: {:?}", op)),
    };

    // Try shift immediate
    if let Some(imm) = shift_imm {
        if matches!(op, BinaryOperator::OpShr) {
            let enc = ((imm + 128) & 0xff) as u32;
            emit(c, Instruction::encode_abc(OpCode::ShrI, result_reg, left_reg, enc));
            return Ok(result_reg);
        } else if matches!(op, BinaryOperator::OpShl) {
            let enc = ((imm + 128) & 0xff) as u32;
            emit(c, Instruction::encode_abc(OpCode::ShlI, result_reg, left_reg, enc));
            return Ok(result_reg);
        }
    }

    // Try constant form for band/bor/bxor
    if is_const {
        if let Some(op_k) = op_rk {
            let const_idx = ensure_constant_idx(c, right_desc);
            emit(c, Instruction::encode_abc(op_k, result_reg, left_reg, const_idx));
            return Ok(result_reg);
        }
    }

    // Fall back to register form
    let right_reg = exp_to_any_reg(c, right_desc);
    emit(c, Instruction::encode_abc(op_rr, result_reg, left_reg, right_reg));
    
    Ok(result_reg)
}

/// Emit comparison operation
pub fn emit_cmp_op(
    c: &mut Compiler,
    op: BinaryOperator,
    left_reg: u32,
    right_reg: u32,
    can_reuse: bool,
) -> u32 {
    let result_reg = if can_reuse { left_reg } else { alloc_register(c) };
    
    let (opcode, a, b) = match op {
        BinaryOperator::OpEq | BinaryOperator::OpNe => (OpCode::Eq, left_reg, right_reg),
        BinaryOperator::OpLt => (OpCode::Lt, left_reg, right_reg),
        BinaryOperator::OpLe => (OpCode::Le, left_reg, right_reg),
        BinaryOperator::OpGt => (OpCode::Lt, right_reg, left_reg), // Swap
        BinaryOperator::OpGe => (OpCode::Le, right_reg, left_reg), // Swap
        _ => unreachable!(),
    };
    
    emit(c, Instruction::encode_abc(opcode, result_reg, a, b));
    result_reg
}

/// Helper to get constant index from ExpDesc
fn ensure_constant_idx(c: &mut Compiler, e: &ExpDesc) -> u32 {
    match e.kind {
        ExpKind::VKInt => add_constant_dedup(c, LuaValue::integer(e.ival)),
        ExpKind::VKFlt => add_constant_dedup(c, LuaValue::number(e.nval)),
        ExpKind::VK => e.info,
        _ => 0,
    }
}

//======================================================================================
// Register-based helpers for compile_binary_expr_to
//======================================================================================

/// Emit arithmetic operation with immediate constant (for compile_binary_expr_to)
/// Returns Some(result_reg) if immediate/constant form was used, None otherwise
pub fn emit_arith_imm(
    c: &mut Compiler,
    op: BinaryOperator,
    left_reg: u32,
    int_val: i64,
    result_reg: u32,
) -> Option<u32> {
    let imm = ((int_val + 127) & 0xff) as u32;
    let imm_mmbini = ((int_val + 128) & 0xff) as u32;

    match op {
        BinaryOperator::OpAdd => {
            emit(c, Instruction::encode_abc(OpCode::AddI, result_reg, left_reg, imm));
            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, imm_mmbini, TagMethod::Add.as_u32(), false));
            Some(result_reg)
        }
        BinaryOperator::OpSub => {
            let neg_imm = ((-int_val + 127) & 0xff) as u32;
            emit(c, Instruction::encode_abc(OpCode::AddI, result_reg, left_reg, neg_imm));
            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, imm_mmbini, TagMethod::Sub.as_u32(), false));
            Some(result_reg)
        }
        _ => None,
    }
}

/// Emit arithmetic operation with constant from K table (for compile_binary_expr_to)
pub fn emit_arith_k(
    c: &mut Compiler,
    op: BinaryOperator,
    left_reg: u32,
    const_idx: u32,
    result_reg: u32,
) -> Option<u32> {
    let (op_k, tm) = match op {
        BinaryOperator::OpAdd => (OpCode::AddK, TagMethod::Add),
        BinaryOperator::OpSub => (OpCode::SubK, TagMethod::Sub),
        BinaryOperator::OpMul => (OpCode::MulK, TagMethod::Mul),
        BinaryOperator::OpDiv => (OpCode::DivK, TagMethod::Div),
        BinaryOperator::OpIDiv => (OpCode::IDivK, TagMethod::IDiv),
        BinaryOperator::OpMod => (OpCode::ModK, TagMethod::Mod),
        BinaryOperator::OpPow => (OpCode::PowK, TagMethod::Pow),
        _ => return None,
    };

    emit(c, Instruction::encode_abc(op_k, result_reg, left_reg, const_idx));
    emit(c, Instruction::create_abck(OpCode::MmBinK, left_reg, const_idx, tm.as_u32(), false));
    Some(result_reg)
}

/// Emit shift operation with immediate (for compile_binary_expr_to)
pub fn emit_shift_imm(
    c: &mut Compiler,
    op: BinaryOperator,
    left_reg: u32,
    int_val: i64,
    result_reg: u32,
) -> Option<u32> {
    let imm = ((int_val + 127) & 0xff) as u32;
    let imm_mmbini = ((int_val + 128) & 0xff) as u32;

    match op {
        BinaryOperator::OpShr => {
            emit(c, Instruction::encode_abc(OpCode::ShrI, result_reg, left_reg, imm));
            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, imm_mmbini, TagMethod::Shr.as_u32(), false));
            Some(result_reg)
        }
        BinaryOperator::OpShl => {
            // x << n is equivalent to x >> -n
            let neg_imm = ((-int_val + 127) & 0xff) as u32;
            emit(c, Instruction::encode_abc(OpCode::ShrI, result_reg, left_reg, neg_imm));
            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, imm_mmbini, TagMethod::Shl.as_u32(), false));
            Some(result_reg)
        }
        _ => None,
    }
}

/// Emit register-register binary operation with MMBIN
pub fn emit_binop_rr(
    c: &mut Compiler,
    op: BinaryOperator,
    left_reg: u32,
    right_reg: u32,
    result_reg: u32,
) {
    let (opcode, tm) = match op {
        BinaryOperator::OpAdd => (OpCode::Add, Some(TagMethod::Add)),
        BinaryOperator::OpSub => (OpCode::Sub, Some(TagMethod::Sub)),
        BinaryOperator::OpMul => (OpCode::Mul, Some(TagMethod::Mul)),
        BinaryOperator::OpDiv => (OpCode::Div, Some(TagMethod::Div)),
        BinaryOperator::OpIDiv => (OpCode::IDiv, Some(TagMethod::IDiv)),
        BinaryOperator::OpMod => (OpCode::Mod, Some(TagMethod::Mod)),
        BinaryOperator::OpPow => (OpCode::Pow, Some(TagMethod::Pow)),
        BinaryOperator::OpBAnd => (OpCode::BAnd, Some(TagMethod::BAnd)),
        BinaryOperator::OpBOr => (OpCode::BOr, Some(TagMethod::BOr)),
        BinaryOperator::OpBXor => (OpCode::BXor, Some(TagMethod::BXor)),
        BinaryOperator::OpShl => (OpCode::Shl, Some(TagMethod::Shl)),
        BinaryOperator::OpShr => (OpCode::Shr, Some(TagMethod::Shr)),
        _ => return,
    };

    emit(c, Instruction::encode_abc(opcode, result_reg, left_reg, right_reg));
    if let Some(tm) = tm {
        emit(c, Instruction::create_abck(OpCode::MmBin, left_reg, right_reg, tm.as_u32(), false));
    }
}
