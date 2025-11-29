// Expression compilation - Using ExpDesc system (Lua 5.4 compatible)

use super::Compiler;
use super::TagMethod;
use super::exp2reg::*;
use super::expdesc::*;
use super::helpers::*;
use crate::compiler::compile_block;
use crate::lua_value::UpvalueDesc;
use crate::lua_value::{Chunk, LuaValue};
use crate::lua_vm::{Instruction, OpCode};
use emmylua_parser::LuaClosureExpr;
use emmylua_parser::LuaIndexExpr;
use emmylua_parser::LuaIndexKey;
use emmylua_parser::LuaParenExpr;
use emmylua_parser::LuaTableExpr;
use emmylua_parser::UnaryOperator;
use emmylua_parser::{
    BinaryOperator, LuaBinaryExpr, LuaCallExpr, LuaExpr, LuaLiteralExpr, LuaLiteralToken,
    LuaNameExpr, LuaUnaryExpr, LuaVarExpr,
};

//======================================================================================
// Helper functions
//======================================================================================

/// Check if an expression is a vararg (...) literal
fn is_vararg_expr(expr: &LuaExpr) -> bool {
    if let LuaExpr::LiteralExpr(lit) = expr {
        matches!(lit.get_literal(), Some(LuaLiteralToken::Dots(_)))
    } else {
        false
    }
}

//======================================================================================
// NEW API: ExpDesc-based expression compilation (Lua 5.4 compatible)
//======================================================================================

/// Core function: Compile expression and return ExpDesc
/// This is the NEW primary API that replaces the old u32-based compile_expr
pub fn compile_expr_desc(c: &mut Compiler, expr: &LuaExpr) -> Result<ExpDesc, String> {
    match expr {
        LuaExpr::LiteralExpr(e) => compile_literal_expr_desc(c, e),
        LuaExpr::NameExpr(e) => compile_name_expr_desc(c, e),
        LuaExpr::BinaryExpr(e) => compile_binary_expr_desc(c, e),
        LuaExpr::UnaryExpr(e) => compile_unary_expr_desc(c, e),
        LuaExpr::ParenExpr(e) => compile_paren_expr_desc(c, e),
        LuaExpr::CallExpr(e) => compile_call_expr_desc(c, e),
        LuaExpr::IndexExpr(e) => compile_index_expr_desc(c, e),
        LuaExpr::TableExpr(e) => compile_table_expr_desc(c, e),
        LuaExpr::ClosureExpr(e) => compile_closure_expr_desc(c, e),
    }
}

//======================================================================================
// OLD API: Backward compatibility wrappers
//======================================================================================

/// OLD API: Compile any expression and return the register containing the result
/// This is now a WRAPPER around compile_expr_desc() + exp_to_any_reg()
/// If dest is Some(reg), try to compile directly into that register to avoid extra Move
pub fn compile_expr(c: &mut Compiler, expr: &LuaExpr) -> Result<u32, String> {
    compile_expr_to(c, expr, None)
}

//======================================================================================
// NEW IMPLEMENTATIONS: ExpDesc-based expression compilation
//======================================================================================

/// Helper: Ensure ExpDesc constant is in constant table and return its index
fn ensure_constant(c: &mut Compiler, e: &ExpDesc) -> Result<u32, String> {
    match e.kind {
        ExpKind::VKInt => {
            let val = LuaValue::integer(e.ival);
            Ok(add_constant_dedup(c, val))
        }
        ExpKind::VKFlt => {
            let val = LuaValue::number(e.nval);
            Ok(add_constant_dedup(c, val))
        }
        ExpKind::VK => {
            Ok(e.info) // Already in constant table
        }
        _ => Err(format!("Cannot get constant index for {:?}", e.kind)),
    }
}

/// NEW: Compile literal expression (returns ExpDesc)
fn compile_literal_expr_desc(c: &mut Compiler, expr: &LuaLiteralExpr) -> Result<ExpDesc, String> {
    let literal_token = expr
        .get_literal()
        .ok_or("Literal expression missing token")?;

    match literal_token {
        LuaLiteralToken::Bool(b) => {
            if b.is_true() {
                Ok(ExpDesc::new_true())
            } else {
                Ok(ExpDesc::new_false())
            }
        }
        LuaLiteralToken::Nil(_) => Ok(ExpDesc::new_nil()),
        LuaLiteralToken::Number(num) => {
            if !num.is_float() {
                let int_val = num.get_int_value();
                // Use VKInt for integers
                Ok(ExpDesc::new_int(int_val))
            } else {
                let float_val = num.get_float_value();
                // Use VKFlt for floats
                Ok(ExpDesc::new_float(float_val))
            }
        }
        LuaLiteralToken::String(s) => {
            // Add string to constant table
            let lua_string = create_string_value(c, &s.get_value());
            let const_idx = add_constant_dedup(c, lua_string);
            Ok(ExpDesc::new_k(const_idx))
        }
        LuaLiteralToken::Dots(_) => {
            // Variable arguments: ...
            // Allocate register and emit VARARG
            let reg = alloc_register(c);
            emit(c, Instruction::encode_abc(OpCode::Vararg, reg, 2, 0));
            Ok(ExpDesc::new_nonreloc(reg))
        }
        _ => Err("Unsupported literal type".to_string()),
    }
}

/// NEW: Compile name expression (returns ExpDesc)
fn compile_name_expr_desc(c: &mut Compiler, expr: &LuaNameExpr) -> Result<ExpDesc, String> {
    let name = expr.get_name_text().unwrap_or("".to_string());

    // Check if it's a local variable
    if let Some(local) = resolve_local(c, &name) {
        // Local variables: use VLocal
        // vidx is the index in the current function's locals array
        return Ok(ExpDesc {
            kind: ExpKind::VLocal,
            info: local.register,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo {
                ridx: local.register,
                vidx: local.register as usize,
            },
            t: 0,
            f: 0,
        });
    }

    // Try to resolve as upvalue
    if let Some(upvalue_index) = resolve_upvalue_from_chain(c, &name) {
        return Ok(ExpDesc {
            kind: ExpKind::VUpval,
            info: upvalue_index as u32,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: 0,
            f: 0,
        });
    }

    // It's a global variable - return VIndexUp
    // _ENV is at upvalue index 0 (standard Lua convention)
    let lua_string = create_string_value(c, &name);
    let key_const_idx = add_constant_dedup(c, lua_string);
    Ok(ExpDesc {
        kind: ExpKind::VIndexUp,
        info: 0,
        ival: 0,
        nval: 0.0,
        ind: IndexInfo {
            t: 0,
            idx: key_const_idx,
        },
        var: VarInfo { ridx: 0, vidx: 0 },
        t: 0,
        f: 0,
    })
}

/// NEW: Compile binary expression (returns ExpDesc)
/// This is the CRITICAL optimization - uses delayed code generation
fn compile_binary_expr_desc(c: &mut Compiler, expr: &LuaBinaryExpr) -> Result<ExpDesc, String> {
    // Get operands and operator
    let (left, right) = expr
        .get_exprs()
        .ok_or("Binary expression missing operands")?;
    let op = expr
        .get_op_token()
        .ok_or("Binary expression missing operator")?;
    let op_kind = op.get_op();

    // For now, use simplified implementation that discharges to registers
    // TODO: Add constant folding, immediate operands, etc.

    // Compile left operand to ExpDesc
    let mut left_desc = compile_expr_desc(c, &left)?;

    // Discharge left to any register (this will allocate if needed)
    let left_reg = exp_to_any_reg(c, &mut left_desc);

    // CRITICAL: Ensure freereg is at least left_reg+1 to prevent right expression
    // from overwriting left's register during nested compilation
    if c.freereg <= left_reg {
        c.freereg = left_reg + 1;
    }

    // Determine if we can reuse left's register
    // We can only reuse if left_reg is a temporary register (>= nactvar)
    // If left_reg < nactvar, it's an active local variable - DO NOT modify!
    let nactvar = nvarstack(c) as u32;
    let can_reuse_left = left_reg >= nactvar;

    // Compile right operand to ExpDesc
    let mut right_desc = compile_expr_desc(c, &right)?;

    // Check if we can use RK (constant in K table) instructions
    // For MUL/DIV/MOD/POW, if right is a constant, use *K variant
    // Check BEFORE discharging to register
    let can_use_rk = matches!(
        right_desc.kind,
        ExpKind::VKInt | ExpKind::VKFlt | ExpKind::VK
    );

    // For arithmetic operations, check if right is a small integer constant
    // and can use immediate instruction (takes precedence over RK)
    let use_immediate = if let ExpKind::VKInt = right_desc.kind {
        let int_val = right_desc.ival;
        int_val >= -256 && int_val <= 255
    } else {
        false
    };

    // Save constant info before discharging
    let saved_right_desc = right_desc.clone();

    // Discharge right to register ONLY if not using RK or immediate
    // This prevents unnecessary LOADK instructions before *K opcodes
    let right_reg = if !can_use_rk && !use_immediate {
        exp_to_any_reg(c, &mut right_desc)
    } else {
        0 // Dummy value, won't be used in RK/immediate paths
    };

    // Handle immediate instructions for ADD/SUB
    if use_immediate && matches!(op_kind, BinaryOperator::OpAdd | BinaryOperator::OpSub) {
        let int_val = right_desc.ival;
        // Use immediate instruction with sC field encoding (add OFFSET_SC = 127)
        let imm = ((int_val + 127) & 0xff) as u32;
        // For MMBINI, we need the signed B field encoding (add OFFSET_SB = 128)
        let imm_mmbini = ((int_val + 128) & 0xff) as u32;

        // Allocate result register (can't reuse left if it's a local variable)
        let result_reg = if can_reuse_left {
            left_reg
        } else {
            alloc_register(c)
        };

        match op_kind {
            BinaryOperator::OpAdd => {
                emit(
                    c,
                    Instruction::encode_abc(OpCode::AddI, result_reg, left_reg, imm),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBinI,
                        left_reg,
                        imm_mmbini,
                        TagMethod::Add.as_u32(),
                        false,
                    ),
                );
                return Ok(ExpDesc::new_nonreloc(result_reg));
            }
            BinaryOperator::OpSub => {
                // Lua 5.4: Subtraction uses ADDI with negated immediate (x - N => x + (-N))
                let neg_imm = ((-int_val + 127) & 0xff) as u32;
                let neg_imm_mmbini = ((-int_val + 128) & 0xff) as u32;
                emit(
                    c,
                    Instruction::encode_abc(OpCode::AddI, result_reg, left_reg, neg_imm),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBinI,
                        left_reg,
                        neg_imm_mmbini,
                        TagMethod::Sub.as_u32(),
                        false,
                    ),
                );
                return Ok(ExpDesc::new_nonreloc(result_reg));
            }
            _ => unreachable!(),
        }
    }

    // Determine result register - will be set in each branch
    let result_reg;

    match op_kind {
        BinaryOperator::OpAdd => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::Add, result_reg, left_reg, right_reg),
            );
            emit(
                c,
                Instruction::create_abck(
                    OpCode::MmBin,
                    left_reg,
                    right_reg,
                    TagMethod::Add.as_u32(),
                    false,
                ),
            );
        }
        BinaryOperator::OpSub => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::Sub, result_reg, left_reg, right_reg),
            );
            emit(
                c,
                Instruction::create_abck(
                    OpCode::MmBin,
                    left_reg,
                    right_reg,
                    TagMethod::Sub.as_u32(),
                    false,
                ),
            );
        }
        BinaryOperator::OpMul => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            if can_use_rk {
                // Right is constant, use MULK
                let const_idx = ensure_constant(c, &saved_right_desc)?;
                emit(
                    c,
                    Instruction::encode_abc(OpCode::MulK, result_reg, left_reg, const_idx),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBinK,
                        left_reg,
                        const_idx,
                        TagMethod::Mul.as_u32(),
                        false,
                    ),
                );
            } else {
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Mul, result_reg, left_reg, right_reg),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBin,
                        left_reg,
                        right_reg,
                        TagMethod::Mul.as_u32(),
                        false,
                    ),
                );
            }
        }
        BinaryOperator::OpDiv => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            if can_use_rk {
                let const_idx = ensure_constant(c, &saved_right_desc)?;
                emit(
                    c,
                    Instruction::encode_abc(OpCode::DivK, result_reg, left_reg, const_idx),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBinK,
                        left_reg,
                        const_idx,
                        TagMethod::Div.as_u32(),
                        false,
                    ),
                );
            } else {
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Div, result_reg, left_reg, right_reg),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBin,
                        left_reg,
                        right_reg,
                        TagMethod::Div.as_u32(),
                        false,
                    ),
                );
            }
        }
        BinaryOperator::OpIDiv => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            if can_use_rk {
                let const_idx = ensure_constant(c, &saved_right_desc)?;
                emit(
                    c,
                    Instruction::encode_abc(OpCode::IDivK, result_reg, left_reg, const_idx),
                );
                emit(
                    c,
                    Instruction::create_abck(OpCode::MmBinK, left_reg, const_idx, 10, false),
                );
            } else {
                emit(
                    c,
                    Instruction::encode_abc(OpCode::IDiv, result_reg, left_reg, right_reg),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBin,
                        left_reg,
                        right_reg,
                        TagMethod::IDiv.as_u32(),
                        false,
                    ),
                );
            }
        }
        BinaryOperator::OpMod => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            if can_use_rk {
                let const_idx = ensure_constant(c, &saved_right_desc)?;
                emit(
                    c,
                    Instruction::encode_abc(OpCode::ModK, result_reg, left_reg, const_idx),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBinK,
                        left_reg,
                        const_idx,
                        TagMethod::Mod.as_u32(),
                        false,
                    ),
                );
            } else {
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Mod, result_reg, left_reg, right_reg),
                );
            }
        }
        BinaryOperator::OpPow => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            if can_use_rk {
                let const_idx = ensure_constant(c, &saved_right_desc)?;
                emit(
                    c,
                    Instruction::encode_abc(OpCode::PowK, result_reg, left_reg, const_idx),
                );
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::MmBinK,
                        left_reg,
                        const_idx,
                        TagMethod::Pow.as_u32(),
                        false,
                    ),
                );
            } else {
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Pow, result_reg, left_reg, right_reg),
                );
            }
        }
        BinaryOperator::OpBAnd => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::BAnd, result_reg, left_reg, right_reg),
            );
        }
        BinaryOperator::OpBOr => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::BOr, result_reg, left_reg, right_reg),
            );
        }
        BinaryOperator::OpBXor => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::BXor, result_reg, left_reg, right_reg),
            );
        }
        BinaryOperator::OpShl => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::Shl, result_reg, left_reg, right_reg),
            );
        }
        BinaryOperator::OpShr => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::Shr, result_reg, left_reg, right_reg),
            );
        }
        BinaryOperator::OpConcat => {
            // CONCAT A B: concatenate R[A] to R[A+B], result in R[A]
            // CRITICAL: CONCAT reads all operands BEFORE writing result to R[A]
            // So it's ALWAYS safe to reuse left_reg, even if it's a local variable!
            // The instruction will read R[A]'s old value, then overwrite it with the result.

            // Check if operands are already consecutive
            if right_reg == left_reg + 1 {
                // Perfect case: operands are consecutive, reuse left for result
                result_reg = left_reg;
                emit(c, Instruction::encode_abc(OpCode::Concat, result_reg, 1, 0));
                // CRITICAL: freereg should be result_reg + 1, not beyond right_reg
                // because CONCAT consumes the right operand
                c.freereg = result_reg + 1;
            } else {
                // Need to arrange operands to be consecutive
                // CRITICAL FIX: Use fresh registers starting from freereg to avoid
                // overwriting already allocated values (like function references)
                let concat_base = c.freereg;
                alloc_register(c); // for left operand copy
                alloc_register(c); // for right operand

                emit_move(c, concat_base, left_reg);
                emit_move(c, concat_base + 1, right_reg);
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Concat, concat_base, 1, 0),
                );

                result_reg = concat_base;
                // Reset freereg to result_reg + 1 (concat consumes right operand)
                c.freereg = result_reg + 1;
            }
        }
        BinaryOperator::OpEq => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::Eq, result_reg, left_reg, right_reg),
            );
        }
        BinaryOperator::OpNe => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::Eq, result_reg, left_reg, right_reg),
            );
            // NEQ is EQ with negated result - handled by control flow
        }
        BinaryOperator::OpLt => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::Lt, result_reg, left_reg, right_reg),
            );
        }
        BinaryOperator::OpLe => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::Le, result_reg, left_reg, right_reg),
            );
        }
        BinaryOperator::OpGt => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            // GT is LT with swapped operands
            emit(
                c,
                Instruction::encode_abc(OpCode::Lt, result_reg, right_reg, left_reg),
            );
        }
        BinaryOperator::OpGe => {
            result_reg = if can_reuse_left {
                left_reg
            } else {
                alloc_register(c)
            };
            // GE is LE with swapped operands
            emit(
                c,
                Instruction::encode_abc(OpCode::Le, result_reg, right_reg, left_reg),
            );
        }
        BinaryOperator::OpAnd | BinaryOperator::OpOr => {
            // Boolean operators need special handling with jumps
            // For now, fallback to simple approach
            return Err("Boolean operators not yet implemented in ExpDesc mode".to_string());
        }
        BinaryOperator::OpNop => {
            return Err("Invalid binary operator OpNop".to_string());
        }
    }

    // Free the right register if it's temporary
    free_exp(c, &right_desc);

    // Result is in left_reg (which we reused)
    Ok(ExpDesc::new_nonreloc(result_reg))
}

/// NEW: Compile unary expression (stub - uses old implementation)
fn compile_unary_expr_desc(c: &mut Compiler, expr: &LuaUnaryExpr) -> Result<ExpDesc, String> {
    // For now, call old implementation
    let reg = compile_unary_expr_to(c, expr, None)?;
    Ok(ExpDesc::new_nonreloc(reg))
}

/// NEW: Compile parenthesized expression (stub)
fn compile_paren_expr_desc(c: &mut Compiler, expr: &LuaParenExpr) -> Result<ExpDesc, String> {
    let reg = compile_paren_expr_to(c, expr, None)?;
    Ok(ExpDesc::new_nonreloc(reg))
}

/// NEW: Compile function call (stub)
fn compile_call_expr_desc(c: &mut Compiler, expr: &LuaCallExpr) -> Result<ExpDesc, String> {
    let reg = compile_call_expr_to(c, expr, None)?;
    Ok(ExpDesc::new_nonreloc(reg))
}

/// NEW: Compile index expression (stub)
fn compile_index_expr_desc(c: &mut Compiler, expr: &LuaIndexExpr) -> Result<ExpDesc, String> {
    let reg = compile_index_expr_to(c, expr, None)?;
    Ok(ExpDesc::new_nonreloc(reg))
}

/// NEW: Compile table constructor (stub)
fn compile_table_expr_desc(c: &mut Compiler, expr: &LuaTableExpr) -> Result<ExpDesc, String> {
    let reg = compile_table_expr_to(c, expr, None)?;
    Ok(ExpDesc::new_nonreloc(reg))
}

/// NEW: Compile closure/function expression (stub)
fn compile_closure_expr_desc(c: &mut Compiler, expr: &LuaClosureExpr) -> Result<ExpDesc, String> {
    let reg = compile_closure_expr_to(c, expr, None, false)?;
    Ok(ExpDesc::new_nonreloc(reg))
}

//======================================================================================
// OLD IMPLEMENTATIONS: Keep for backward compatibility
//======================================================================================

/// Compile expression to a specific destination register if possible
pub fn compile_expr_to(c: &mut Compiler, expr: &LuaExpr, dest: Option<u32>) -> Result<u32, String> {
    match expr {
        LuaExpr::LiteralExpr(e) => compile_literal_expr(c, e, dest),
        LuaExpr::NameExpr(e) => compile_name_expr_to(c, e, dest),
        LuaExpr::BinaryExpr(e) => compile_binary_expr_to(c, e, dest),
        LuaExpr::UnaryExpr(e) => compile_unary_expr_to(c, e, dest),
        LuaExpr::ParenExpr(e) => compile_paren_expr_to(c, e, dest),
        LuaExpr::CallExpr(e) => compile_call_expr_to(c, e, dest),
        LuaExpr::IndexExpr(e) => compile_index_expr_to(c, e, dest),
        LuaExpr::TableExpr(e) => compile_table_expr_to(c, e, dest),
        LuaExpr::ClosureExpr(e) => compile_closure_expr_to(c, e, dest, false),
    }
}

/// Compile literal expression (number, string, true, false, nil)
fn compile_literal_expr(
    c: &mut Compiler,
    expr: &LuaLiteralExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    let reg = dest.unwrap_or_else(|| alloc_register(c));

    // Ensure max_stack_size can accommodate this register
    ensure_register(c, reg);

    let literal_token = expr
        .get_literal()
        .ok_or("Literal expression missing token")?;
    match literal_token {
        LuaLiteralToken::Bool(b) => {
            emit_load_bool(c, reg, b.is_true());
        }
        LuaLiteralToken::Nil(_) => {
            emit_load_nil(c, reg);
        }
        LuaLiteralToken::Number(num) => {
            // Lua 5.4 optimization: Try LoadI for integers, LoadF for simple floats
            if !num.is_float() {
                let int_val = num.get_int_value();
                // Try LoadI first (fast path for small integers)
                if let Some(_) = emit_loadi(c, reg, int_val) {
                    return Ok(reg);
                }
                // LoadI failed, add to constant table
                let const_idx = add_constant_dedup(c, LuaValue::integer(int_val));
                emit_loadk(c, reg, const_idx);
            } else {
                let float_val = num.get_float_value();
                // Try LoadF for integer-representable floats
                if emit_loadf(c, reg, float_val).is_none() {
                    // LoadF failed, add to constant table
                    let const_idx = add_constant_dedup(c, LuaValue::float(float_val));
                    emit_loadk(c, reg, const_idx);
                }
            }
        }
        LuaLiteralToken::String(s) => {
            let string_val = s.get_value();
            let lua_string = create_string_value(c, &string_val);
            let const_idx = add_constant_dedup(c, lua_string);
            emit_loadk(c, reg, const_idx);
        }
        LuaLiteralToken::Dots(_) => {
            // Variable arguments: ...
            // VarArg instruction: R(A), ..., R(A+B-2) = vararg
            // B=1 means load 0 varargs (empty)
            // B=2 means load 1 vararg into R(A)
            // B=0 means load all varargs starting from R(A)
            // For expression context, we load 1 vararg into the register
            emit(c, Instruction::encode_abc(OpCode::Vararg, reg, 2, 0));
        }
        _ => {}
    }

    Ok(reg)
}

fn compile_name_expr_to(
    c: &mut Compiler,
    expr: &LuaNameExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    // Get the identifier name
    let name = expr.get_name_text().unwrap_or("".to_string());

    // Check if it's a local variable
    if let Some(local) = resolve_local(c, &name) {
        // If local is already in dest register, no move needed
        if let Some(dest_reg) = dest {
            if local.register != dest_reg {
                emit_move(c, dest_reg, local.register);
            }
            return Ok(dest_reg);
        }
        return Ok(local.register);
    }

    // Try to resolve as upvalue from parent scope chain
    if let Some(upvalue_index) = resolve_upvalue_from_chain(c, &name) {
        let reg = dest.unwrap_or_else(|| alloc_register(c));
        let instr = Instruction::encode_abc(OpCode::GetUpval, reg, upvalue_index as u32, 0);
        c.chunk.code.push(instr);
        return Ok(reg);
    }

    // It's a global variable
    let reg = dest.unwrap_or_else(|| alloc_register(c));
    emit_get_global(c, &name, reg);
    Ok(reg)
}

/// Try to evaluate an expression as a constant integer (for SETI/GETI optimization)
/// Returns Some(int_value) if the expression is a compile-time constant integer
fn try_eval_const_int(expr: &LuaExpr) -> Option<i64> {
    match expr {
        LuaExpr::LiteralExpr(lit) => {
            if let Some(LuaLiteralToken::Number(num)) = lit.get_literal() {
                if !num.is_float() {
                    return Some(num.get_int_value());
                }
            }
            None
        }
        LuaExpr::BinaryExpr(bin_expr) => {
            // Try to evaluate binary expressions with constant operands
            let (left, right) = bin_expr.get_exprs()?;
            let left_val = try_eval_const_int(&left)?;
            let right_val = try_eval_const_int(&right)?;

            let op = bin_expr.get_op_token()?.get_op();
            match op {
                BinaryOperator::OpAdd => Some(left_val + right_val),
                BinaryOperator::OpSub => Some(left_val - right_val),
                BinaryOperator::OpMul => Some(left_val * right_val),
                BinaryOperator::OpDiv => {
                    let result = left_val as f64 / right_val as f64;
                    if result.fract() == 0.0 {
                        Some(result as i64)
                    } else {
                        None
                    }
                }
                BinaryOperator::OpIDiv => Some(left_val / right_val),
                BinaryOperator::OpMod => Some(left_val % right_val),
                BinaryOperator::OpBAnd => Some(left_val & right_val),
                BinaryOperator::OpBOr => Some(left_val | right_val),
                BinaryOperator::OpBXor => Some(left_val ^ right_val),
                BinaryOperator::OpShl => Some(left_val << (right_val & 0x3f)),
                BinaryOperator::OpShr => Some(left_val >> (right_val & 0x3f)),
                _ => None,
            }
        }
        LuaExpr::UnaryExpr(un_expr) => {
            // Try to evaluate unary expressions
            let operand = un_expr.get_expr()?;
            let op_val = try_eval_const_int(&operand)?;

            let op = un_expr.get_op_token()?.get_op();
            match op {
                UnaryOperator::OpUnm => Some(-op_val),
                UnaryOperator::OpBNot => Some(!op_val),
                _ => None,
            }
        }
        _ => None,
    }
}

fn compile_binary_expr_to(
    c: &mut Compiler,
    expr: &LuaBinaryExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    // Get left and right expressions from children
    let (left, right) = expr.get_exprs().ok_or("error")?;
    let op = expr.get_op_token().ok_or("error")?;
    let op_kind = op.get_op();

    // CONSTANT FOLDING for boolean literals (and, or)
    if matches!(op_kind, BinaryOperator::OpAnd | BinaryOperator::OpOr) {
        // Check if left operand is a boolean literal
        if let LuaExpr::LiteralExpr(left_lit) = &left {
            if let Some(LuaLiteralToken::Bool(b)) = left_lit.get_literal() {
                let result_reg = dest.unwrap_or_else(|| alloc_register(c));

                if op_kind == BinaryOperator::OpAnd {
                    // true and X -> X, false and X -> false
                    if b.is_true() {
                        // Result is right operand
                        return compile_expr_to(c, &right, Some(result_reg));
                    } else {
                        // Result is false
                        emit(
                            c,
                            Instruction::encode_abc(OpCode::LoadFalse, result_reg, 0, 0),
                        );
                        return Ok(result_reg);
                    }
                } else {
                    // true or X -> true, false or X -> X
                    if b.is_true() {
                        // Result is true
                        emit(
                            c,
                            Instruction::encode_abc(OpCode::LoadTrue, result_reg, 0, 0),
                        );
                        return Ok(result_reg);
                    } else {
                        // Result is right operand
                        return compile_expr_to(c, &right, Some(result_reg));
                    }
                }
            }
        }
    }

    // CONSTANT FOLDING: Check if both operands are numeric constants (including nested expressions)
    // This matches luac behavior: 1+1 -> 2, 1+2*3 -> 7, etc.
    // Use try_eval_const_int to recursively evaluate constant expressions
    if matches!(
        op_kind,
        BinaryOperator::OpAdd
            | BinaryOperator::OpSub
            | BinaryOperator::OpMul
            | BinaryOperator::OpDiv
            | BinaryOperator::OpIDiv
            | BinaryOperator::OpMod
            | BinaryOperator::OpPow
            | BinaryOperator::OpBAnd
            | BinaryOperator::OpBOr
            | BinaryOperator::OpBXor
            | BinaryOperator::OpShl
            | BinaryOperator::OpShr
    ) {
        if let (Some(left_int), Some(right_int)) =
            (try_eval_const_int(&left), try_eval_const_int(&right))
        {
            let left_val = left_int as f64;
            let right_val = right_int as f64;

            let result_opt: Option<f64> = match op_kind {
                BinaryOperator::OpAdd => Some(left_val + right_val),
                BinaryOperator::OpSub => Some(left_val - right_val),
                BinaryOperator::OpMul => Some(left_val * right_val),
                BinaryOperator::OpDiv => Some(left_val / right_val),
                BinaryOperator::OpIDiv => Some((left_val / right_val).floor()),
                BinaryOperator::OpMod => Some(left_val % right_val),
                BinaryOperator::OpPow => Some(left_val.powf(right_val)),
                BinaryOperator::OpBAnd => Some((left_int & right_int) as f64),
                BinaryOperator::OpBOr => Some((left_int | right_int) as f64),
                BinaryOperator::OpBXor => Some((left_int ^ right_int) as f64),
                BinaryOperator::OpShl => Some((left_int << (right_int & 0x3f)) as f64),
                BinaryOperator::OpShr => Some((left_int >> (right_int & 0x3f)) as f64),
                _ => None,
            };

            if let Some(result) = result_opt {
                let result_reg = dest.unwrap_or_else(|| alloc_register(c));

                // Emit the folded constant as LOADI or LOADF
                let result_int = result as i64;
                if result == result_int as f64 {
                    // Integer result - try LOADI first
                    if emit_loadi(c, result_reg, result_int).is_none() {
                        // Too large for LOADI, use LOADK
                        let lua_val = LuaValue::integer(result_int);
                        let const_idx = add_constant(c, lua_val);
                        emit(
                            c,
                            Instruction::encode_abx(OpCode::LoadK, result_reg, const_idx as u32),
                        );
                    }
                } else {
                    // Float result - try LOADF first, then LOADK
                    if emit_loadf(c, result_reg, result).is_none() {
                        let lua_val = LuaValue::number(result);
                        let const_idx = add_constant(c, lua_val);
                        emit(
                            c,
                            Instruction::encode_abx(OpCode::LoadK, result_reg, const_idx as u32),
                        );
                    }
                }
                return Ok(result_reg);
            }
        }
    }

    // OLD CONSTANT FOLDING (literal-only, kept for compatibility)
    // This is now redundant but kept as fallback
    if let (LuaExpr::LiteralExpr(left_lit), LuaExpr::LiteralExpr(right_lit)) = (&left, &right) {
        if let (Some(LuaLiteralToken::Number(left_num)), Some(LuaLiteralToken::Number(right_num))) =
            (left_lit.get_literal(), right_lit.get_literal())
        {
            let left_val = if left_num.is_float() {
                left_num.get_float_value()
            } else {
                left_num.get_int_value() as f64
            };

            let right_val = if right_num.is_float() {
                right_num.get_float_value()
            } else {
                right_num.get_int_value() as f64
            };

            // Calculate result based on operator
            let result_opt: Option<f64> = match op_kind {
                BinaryOperator::OpAdd => Some(left_val + right_val),
                BinaryOperator::OpSub => Some(left_val - right_val),
                BinaryOperator::OpMul => Some(left_val * right_val),
                BinaryOperator::OpDiv => Some(left_val / right_val),
                BinaryOperator::OpIDiv => Some((left_val / right_val).floor()),
                BinaryOperator::OpMod => Some(left_val % right_val),
                BinaryOperator::OpPow => Some(left_val.powf(right_val)),
                // Bitwise operations require integers
                BinaryOperator::OpBAnd
                | BinaryOperator::OpBOr
                | BinaryOperator::OpBXor
                | BinaryOperator::OpShl
                | BinaryOperator::OpShr => {
                    if !left_num.is_float() && !right_num.is_float() {
                        let left_int = left_num.get_int_value() as i64;
                        let right_int = right_num.get_int_value() as i64;
                        let result_int = match op_kind {
                            BinaryOperator::OpBAnd => left_int & right_int,
                            BinaryOperator::OpBOr => left_int | right_int,
                            BinaryOperator::OpBXor => left_int ^ right_int,
                            BinaryOperator::OpShl => left_int << (right_int & 0x3f),
                            BinaryOperator::OpShr => left_int >> (right_int & 0x3f),
                            _ => unreachable!(),
                        };
                        Some(result_int as f64)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(result) = result_opt {
                let result_reg = dest.unwrap_or_else(|| alloc_register(c));

                // Emit the folded constant as LOADI or LOADK
                let result_int = result as i64;
                if result == result_int as f64 {
                    // Integer result - try LOADI first
                    if emit_loadi(c, result_reg, result_int).is_none() {
                        // Too large for LOADI, use LOADK
                        let lua_val = LuaValue::integer(result_int);
                        let const_idx = add_constant(c, lua_val);
                        emit(
                            c,
                            Instruction::encode_abx(OpCode::LoadK, result_reg, const_idx as u32),
                        );
                    }
                } else {
                    // Float result - use LOADK
                    let lua_val = LuaValue::number(result);
                    let const_idx = add_constant(c, lua_val);
                    emit(
                        c,
                        Instruction::encode_abx(OpCode::LoadK, result_reg, const_idx as u32),
                    );
                }
                return Ok(result_reg);
            }
        }
    }

    // Try to optimize with immediate operands (Lua 5.4 optimization)
    // Check if right operand is a small integer constant
    if let LuaExpr::LiteralExpr(lit) = &right {
        if let Some(LuaLiteralToken::Number(num)) = lit.get_literal() {
            if !num.is_float() {
                let int_val = num.get_int_value();
                // Use signed 9-bit immediate: range [-256, 255]
                if int_val >= -256 && int_val <= 255 {
                    // Encode immediate value for sC field (8-bit signed with OFFSET_SC)
                    let imm = ((int_val + 127) & 0xff) as u32;
                    // For MMBINI, encode with OFFSET_SB (128) instead of OFFSET_SC (127)
                    let imm_mmbini = ((int_val + 128) & 0xff) as u32;

                    // Try immediate arithmetic instructions
                    // Only compile left operand if we actually use immediate instruction
                    match op_kind {
                        BinaryOperator::OpAdd => {
                            // CRITICAL: Protect freereg if dest specified
                            if let Some(d) = dest {
                                if c.freereg < d + 1 {
                                    c.freereg = d + 1;
                                }
                            }
                            // Compile left operand first to get its register
                            let left_reg = compile_expr(c, &left)?;
                            // Use dest if provided, otherwise check if left_reg can be reused
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left {
                                    left_reg
                                } else {
                                    alloc_register(c)
                                }
                            });
                            emit(
                                c,
                                Instruction::encode_abc(OpCode::AddI, result_reg, left_reg, imm),
                            );
                            // Emit MMBINI for metamethod call (TM_ADD = 6)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinI,
                                    left_reg,
                                    imm_mmbini,
                                    6,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpSub => {
                            if let Some(d) = dest {
                                if c.freereg < d + 1 {
                                    c.freereg = d + 1;
                                }
                            }
                            let left_reg = compile_expr(c, &left)?;
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left {
                                    left_reg
                                } else {
                                    alloc_register(c)
                                }
                            });
                            // Lua 5.4: Subtraction uses ADDI with negated immediate (x - N => x + (-N))
                            // Encode -int_val with OFFSET_SC
                            let neg_imm = ((-int_val + 127) & 0xff) as u32;
                            emit(
                                c,
                                Instruction::encode_abc(
                                    OpCode::AddI,
                                    result_reg,
                                    left_reg,
                                    neg_imm,
                                ),
                            );
                            // Emit MMBINI for metamethod call - use ORIGINAL immediate value from source (TM_SUB = 7)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinI,
                                    left_reg,
                                    imm_mmbini,
                                    7,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpMul => {
                            // For MulK, need to add constant to constant table
                            let const_idx = add_constant_dedup(c, LuaValue::integer(int_val));
                            let left_reg = compile_expr(c, &left)?;
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left { left_reg } else { alloc_register(c) }
                            });
                            // Lua 5.4: Use MulK for constant operand
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MulK,
                                    result_reg,
                                    left_reg,
                                    const_idx,
                                    false,
                                ),
                            );
                            // Emit MMBINK for metamethod call (TM_MUL = 8)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinK,
                                    left_reg,
                                    const_idx,
                                    8,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpMod => {
                            let const_idx = add_constant_dedup(c, LuaValue::integer(int_val));
                            let left_reg = compile_expr(c, &left)?;
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left { left_reg } else { alloc_register(c) }
                            });
                            // Lua 5.4: Use ModK for constant operand
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::ModK,
                                    result_reg,
                                    left_reg,
                                    const_idx,
                                    false,
                                ),
                            );
                            // Emit MMBINK for metamethod call (TM_MOD = 9)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinK,
                                    left_reg,
                                    const_idx,
                                    9,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpPow => {
                            let const_idx = add_constant_dedup(c, LuaValue::integer(int_val));
                            let left_reg = compile_expr(c, &left)?;
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left { left_reg } else { alloc_register(c) }
                            });
                            // Lua 5.4: Use PowK for constant operand
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::PowK,
                                    result_reg,
                                    left_reg,
                                    const_idx,
                                    false,
                                ),
                            );
                            // Emit MMBINK for metamethod call (TM_POW = 10)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinK,
                                    left_reg,
                                    const_idx,
                                    10,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpDiv => {
                            let const_idx = add_constant_dedup(c, LuaValue::integer(int_val));
                            let left_reg = compile_expr(c, &left)?;
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left { left_reg } else { alloc_register(c) }
                            });
                            // Lua 5.4: Use DivK for constant operand
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::DivK,
                                    result_reg,
                                    left_reg,
                                    const_idx,
                                    false,
                                ),
                            );
                            // Emit MMBINK for metamethod call (TM_DIV = 11)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinK,
                                    left_reg,
                                    const_idx,
                                    11,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpIDiv => {
                            let const_idx = add_constant_dedup(c, LuaValue::integer(int_val));
                            let left_reg = compile_expr(c, &left)?;
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left { left_reg } else { alloc_register(c) }
                            });
                            // Lua 5.4: Use IDivK for constant operand
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::IDivK,
                                    result_reg,
                                    left_reg,
                                    const_idx,
                                    false,
                                ),
                            );
                            // Emit MMBINK for metamethod call (TM_IDIV = 12)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinK,
                                    left_reg,
                                    const_idx,
                                    12,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpShr => {
                            if let Some(d) = dest {
                                if c.freereg < d + 1 {
                                    c.freereg = d + 1;
                                }
                            }
                            let left_reg = compile_expr(c, &left)?;
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left { left_reg } else { alloc_register(c) }
                            });
                            // Lua 5.4: Use ShrI for immediate right shift
                            emit(
                                c,
                                Instruction::encode_abc(OpCode::ShrI, result_reg, left_reg, imm),
                            );
                            // Emit MMBINI for metamethod call (TM_SHR = 17)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinI,
                                    left_reg,
                                    imm_mmbini,
                                    17,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpShl => {
                            if let Some(d) = dest {
                                if c.freereg < d + 1 {
                                    c.freereg = d + 1;
                                }
                            }
                            let left_reg = compile_expr(c, &left)?;
                            // CRITICAL: Never reuse a local variable's register!
                            let nvarstack = nvarstack(c);
                            let can_reuse_left = left_reg >= nvarstack;
                            let result_reg = dest.unwrap_or_else(|| {
                                if can_reuse_left { left_reg } else { alloc_register(c) }
                            });
                            // Lua 5.4: Use ShlI for immediate left shift
                            // Note: ShlI uses negated immediate: sC << R[B] where sC is the immediate
                            // To shift left by N, we use -N as the immediate
                            let neg_imm = if int_val < 0 {
                                ((-int_val) + 256) as u32
                            } else {
                                ((-int_val) + 512) as u32 % 512
                            };
                            emit(
                                c,
                                Instruction::encode_abc(
                                    OpCode::ShlI,
                                    result_reg,
                                    left_reg,
                                    neg_imm,
                                ),
                            );
                            // Emit MMBINI for metamethod call (TM_SHL = 16)
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::MmBinI,
                                    left_reg,
                                    imm_mmbini,
                                    16,
                                    false,
                                ),
                            );
                            return Ok(result_reg);
                        }
                        // Immediate comparison operators - generate boolean result
                        BinaryOperator::OpEq
                        | BinaryOperator::OpNe
                        | BinaryOperator::OpLt
                        | BinaryOperator::OpLe
                        | BinaryOperator::OpGt
                        | BinaryOperator::OpGe => {
                            // Comparison instructions use sB field: range [-128, 127]
                            if int_val >= -128 && int_val <= 127 {
                                let left_reg = compile_expr(c, &left)?;
                                // Allocate a new register for the boolean result
                                // (can't reuse left_reg because comparison needs the original value)
                                let result_reg = dest.unwrap_or_else(|| alloc_register(c));

                                // Use immediate comparison instruction with boolean result pattern
                                return compile_comparison_imm_to_bool(
                                    c,
                                    op_kind,
                                    left_reg,
                                    result_reg,
                                    int_val as i32,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // FLOAT CONSTANT OPTIMIZATION: Check if right operand is a float literal
    // Generate *K instructions for MUL/DIV/MOD/POW with constant operands
    if let LuaExpr::LiteralExpr(lit) = &right {
        if let Some(LuaLiteralToken::Number(num)) = lit.get_literal() {
            // Handle both float and integer literals
            let is_float_lit = num.is_float();
            let const_val = if is_float_lit {
                LuaValue::float(num.get_float_value())
            } else {
                // Integer literal - still use *K instruction instead of immediate
                let int_val = num.get_int_value();
                // Skip small integers that were already handled by immediate optimization
                if int_val >= -256 && int_val <= 255 {
                    match op_kind {
                        BinaryOperator::OpAdd | BinaryOperator::OpSub => {
                            // Already handled by ADDI above
                            // Fall through to normal code path
                            LuaValue::integer(int_val) // dummy, won't be used
                        }
                        _ => LuaValue::integer(int_val),
                    }
                } else {
                    LuaValue::integer(int_val)
                }
            };

            // Generate *K instruction for supported operations
            match op_kind {
                BinaryOperator::OpMul => {
                    let const_idx = add_constant_dedup(c, const_val);
                    // CRITICAL: Protect freereg if dest specified
                    if let Some(d) = dest {
                        if c.freereg < d + 1 {
                            c.freereg = d + 1;
                        }
                    }
                    let left_reg = compile_expr(c, &left)?;
                    let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::MulK, result_reg, left_reg, const_idx),
                    );
                    emit(
                        c,
                        Instruction::create_abck(
                            OpCode::MmBinK,
                            left_reg,
                            const_idx,
                            TagMethod::Mul.as_u32(),
                            false,
                        ),
                    );
                    return Ok(result_reg);
                }
                BinaryOperator::OpDiv => {
                    let const_idx = add_constant_dedup(c, const_val);
                    // CRITICAL: Protect freereg if dest specified
                    if let Some(d) = dest {
                        if c.freereg < d + 1 {
                            c.freereg = d + 1;
                        }
                    }
                    let left_reg = compile_expr(c, &left)?;
                    let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::DivK, result_reg, left_reg, const_idx),
                    );
                    emit(
                        c,
                        Instruction::create_abck(
                            OpCode::MmBinK,
                            left_reg,
                            const_idx,
                            TagMethod::Div.as_u32(),
                            false,
                        ),
                    );
                    return Ok(result_reg);
                }
                BinaryOperator::OpMod => {
                    let const_idx = add_constant_dedup(c, const_val);
                    // CRITICAL: Protect freereg if dest specified
                    if let Some(d) = dest {
                        if c.freereg < d + 1 {
                            c.freereg = d + 1;
                        }
                    }
                    let left_reg = compile_expr(c, &left)?;
                    let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::ModK, result_reg, left_reg, const_idx),
                    );
                    emit(
                        c,
                        Instruction::create_abck(
                            OpCode::MmBinK,
                            left_reg,
                            const_idx,
                            TagMethod::Mod.as_u32(),
                            false,
                        ),
                    );
                    return Ok(result_reg);
                }
                BinaryOperator::OpPow => {
                    let const_idx = add_constant_dedup(c, const_val);
                    // CRITICAL: Protect freereg if dest specified
                    if let Some(d) = dest {
                        if c.freereg < d + 1 {
                            c.freereg = d + 1;
                        }
                    }
                    let left_reg = compile_expr(c, &left)?;
                    let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::PowK, result_reg, left_reg, const_idx),
                    );
                    emit(
                        c,
                        Instruction::create_abck(
                            OpCode::MmBinK,
                            left_reg,
                            const_idx,
                            TagMethod::Pow.as_u32(),
                            false,
                        ),
                    );
                    return Ok(result_reg);
                }
                BinaryOperator::OpIDiv if !is_float_lit => {
                    // IDiv only for integer constants
                    let const_idx = add_constant_dedup(c, const_val);
                    // CRITICAL: Protect freereg if dest specified
                    if let Some(d) = dest {
                        if c.freereg < d + 1 {
                            c.freereg = d + 1;
                        }
                    }
                    let left_reg = compile_expr(c, &left)?;
                    let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::IDivK, result_reg, left_reg, const_idx),
                    );
                    emit(
                        c,
                        Instruction::create_abck(
                            OpCode::MmBinK,
                            left_reg,
                            const_idx,
                            TagMethod::IDiv.as_u32(),
                            false,
                        ),
                    );
                    return Ok(result_reg);
                }
                _ => {
                    // Not a *K-supported operation, fall through
                }
            }
        }
    }

    // Fall back to normal two-operand instruction
    // CRITICAL: If dest is specified, protect freereg BEFORE compiling operands
    // This prevents nested expressions from allocating temps that conflict with dest
    if let Some(d) = dest {
        if c.freereg < d + 1 {
            c.freereg = d + 1;
        }
    }

    // Compile left and right first to get their registers
    let left_reg = compile_expr(c, &left)?;

    // Ensure right doesn't overwrite left
    if c.freereg <= left_reg {
        c.freereg = left_reg + 1;
    }

    let right_reg = compile_expr(c, &right)?;
    // Then allocate result register
    let result_reg = dest.unwrap_or_else(|| alloc_register(c));

    // Determine opcode and metamethod event (TM)
    let (opcode, mm_event_opt) = match op_kind {
        BinaryOperator::OpAdd => (OpCode::Add, Some(TagMethod::Add)),
        BinaryOperator::OpSub => (OpCode::Sub, Some(TagMethod::Sub)),
        BinaryOperator::OpMul => (OpCode::Mul, Some(TagMethod::Mul)),
        BinaryOperator::OpMod => (OpCode::Mod, Some(TagMethod::Mod)),
        BinaryOperator::OpPow => (OpCode::Pow, Some(TagMethod::Pow)),
        BinaryOperator::OpDiv => (OpCode::Div, Some(TagMethod::Div)),
        BinaryOperator::OpIDiv => (OpCode::IDiv, Some(TagMethod::IDiv)),
        BinaryOperator::OpBAnd => (OpCode::BAnd, Some(TagMethod::BAnd)),
        BinaryOperator::OpBOr => (OpCode::BOr, Some(TagMethod::BOr)),
        BinaryOperator::OpBXor => (OpCode::BXor, Some(TagMethod::BXor)),
        BinaryOperator::OpShl => (OpCode::Shl, Some(TagMethod::Shl)),
        BinaryOperator::OpShr => (OpCode::Shr, Some(TagMethod::Shr)),
        BinaryOperator::OpConcat => {
            // CONCAT has special instruction format: CONCAT A B
            // Concatenates R[A] through R[A+B] (B+1 values), result in R[A]

            // LUA 5.4 OPTIMIZATION: Merge consecutive CONCAT operations
            // When compiling "e1 .. e2", if e2's code just generated a CONCAT instruction,
            // we can merge them into a single CONCAT that concatenates all values at once.
            // This is critical for performance with chains like "a" .. "b" .. "c"

            let code = &c.chunk.code;
            if !code.is_empty() {
                let prev_instr = code[code.len() - 1];
                let prev_opcode = Instruction::get_opcode(prev_instr);

                if prev_opcode == OpCode::Concat {
                    let prev_a = Instruction::get_a(prev_instr);
                    let prev_b = Instruction::get_b(prev_instr);

                    // Check if right_reg is the result of previous CONCAT
                    // Previous CONCAT: R[prev_a] = R[prev_a]..R[prev_a+1]..R[prev_a+prev_b]
                    // If right_reg == prev_a and left_reg == prev_a - 1, we can merge
                    if right_reg as u32 == prev_a && left_reg as u32 + 1 == prev_a {
                        // Perfect! Extend the CONCAT to include left_reg
                        // New CONCAT: R[left_reg] = R[left_reg]..R[left_reg+1]..R[left_reg+1+prev_b]
                        let last_idx = code.len() - 1;
                        c.chunk.code[last_idx] = Instruction::encode_abc(
                            OpCode::Concat,
                            left_reg,   // Start from left_reg instead
                            prev_b + 1, // Increase count by 1
                            0,
                        );

                        // BUGFIX: Respect dest parameter
                        if let Some(d) = dest {
                            if d != left_reg {
                                emit_move(c, d, left_reg);
                                return Ok(d);
                            }
                        }
                        return Ok(left_reg);
                    }
                }
            }

            // Standard case: No merge possible, emit new CONCAT
            // Check if operands are already consecutive
            if right_reg == left_reg + 1 {
                // Perfect case: operands are consecutive
                let concat_reg = left_reg;
                emit(c, Instruction::encode_abc(OpCode::Concat, concat_reg, 1, 0));
                if let Some(d) = dest {
                    if d != concat_reg {
                        emit_move(c, d, concat_reg);
                    }
                    return Ok(d);
                } else {
                    return Ok(concat_reg);
                }
            } else {
                // Need to arrange operands into consecutive registers
                // CRITICAL FIX: Use fresh registers starting from freereg to avoid
                // overwriting already allocated values (like function references)
                let concat_reg = c.freereg;
                alloc_register(c); // for left operand copy
                alloc_register(c); // for right operand

                emit_move(c, concat_reg, left_reg);
                emit_move(c, concat_reg + 1, right_reg);
                emit(c, Instruction::encode_abc(OpCode::Concat, concat_reg, 1, 0));

                // Reset freereg (concat consumes right operand)
                c.freereg = concat_reg + 1;

                if let Some(d) = dest {
                    if d != concat_reg {
                        emit_move(c, d, concat_reg);
                    }
                    return Ok(d);
                }
                return Ok(concat_reg);
            }
        }

        // Comparison operators need special handling - they don't produce values directly
        // Instead, they skip the next instruction if the comparison is true
        // We need to generate: CMP + JMP + LFALSESKIP + LOADTRUE pattern
        BinaryOperator::OpEq
        | BinaryOperator::OpNe
        | BinaryOperator::OpLt
        | BinaryOperator::OpLe
        | BinaryOperator::OpGt
        | BinaryOperator::OpGe => {
            // Handle comparison operators with proper boolean result generation
            return compile_comparison_to_bool(c, op_kind, left_reg, right_reg, result_reg);
        }

        BinaryOperator::OpAnd | BinaryOperator::OpOr => {
            // Boolean operators with proper short-circuit evaluation
            // Pattern: TESTSET + JMP + MOVE
            // and: if left is false, return left; else return right
            // or: if left is true, return left; else return right
            let k_flag = matches!(op_kind, BinaryOperator::OpOr);

            // TestSet: if (is_truthy == k) then R[A] := R[B] else pc++
            emit(
                c,
                Instruction::create_abck(OpCode::TestSet, result_reg, left_reg, 0, k_flag),
            );
            // JMP: skip the MOVE if TestSet assigned the value
            let jump_pos = emit_jump(c, OpCode::Jmp);
            // MOVE: use right operand if TestSet didn't assign
            emit(
                c,
                Instruction::create_abc(OpCode::Move, result_reg, right_reg, 0),
            );
            // Patch the jump to point after MOVE
            patch_jump(c, jump_pos);

            return Ok(result_reg);
        }

        _ => return Err(format!("Unsupported binary operator: {:?}", op_kind)),
    };

    emit(
        c,
        Instruction::encode_abc(opcode, result_reg, left_reg, right_reg),
    );

    // Emit MMBIN instruction for metamethod call if this is an arithmetic/bitwise operation
    // Lua 5.4: MMBIN follows the main instruction to call metamethod if operation fails
    if let Some(mm_event) = mm_event_opt {
        emit(
            c,
            Instruction::create_abck(OpCode::MmBin, left_reg, right_reg, mm_event.as_u32(), false),
        );
    }

    Ok(result_reg)
}

/// Compile comparison operator to produce boolean result
/// Generates: CMP + JMP + LFALSESKIP + LOADTRUE pattern
fn compile_comparison_to_bool(
    c: &mut Compiler,
    op_kind: BinaryOperator,
    left_reg: u32,
    right_reg: u32,
    result_reg: u32,
) -> Result<u32, String> {
    // Pattern: CMP with k=1 (skip if true) + JMP to true_label + LFALSESKIP + LOADTRUE
    // If comparison is true: skip JMP, execute LFALSESKIP (skip LOADTRUE), wait that's wrong...
    // Actually: CMP with k=1 (skip if true) means "skip next if comparison IS true"
    // So: CMP(k=1) + JMP(to after_false) + LFALSESKIP + LOADTRUE
    // If true: skip JMP, go to LFALSESKIP... no that's still wrong.

    // Let me trace luac output again:
    // EQI 0 8 1      # if (R[0] == 8) != 1 then skip; which means: if R[0] != 8 then skip
    // JMP 1          # jump over LFALSESKIP
    // LFALSESKIP 0   # R[0] = false, skip LOADTRUE
    // LOADTRUE 0     # R[0] = true

    // So when R[0] == 8:
    //   - EQI: condition is true, DON'T skip (k=1 means skip if result != 1)
    //   - Execute JMP: jump to LOADTRUE
    //   - Execute LOADTRUE: R[0] = true ✓

    // When R[0] != 8:
    //   - EQI: condition is false, skip JMP
    //   - Execute LFALSESKIP: R[0] = false, skip LOADTRUE ✓

    let (cmp_opcode, swap_operands, negate) = match op_kind {
        BinaryOperator::OpEq => (OpCode::Eq, false, false),
        BinaryOperator::OpNe => (OpCode::Eq, false, true),
        BinaryOperator::OpLt => (OpCode::Lt, false, false),
        BinaryOperator::OpLe => (OpCode::Le, false, false),
        BinaryOperator::OpGt => (OpCode::Lt, true, false), // a > b == b < a
        BinaryOperator::OpGe => (OpCode::Le, true, false), // a >= b == b <= a
        _ => unreachable!(),
    };

    let (op1, op2) = if swap_operands {
        (right_reg, left_reg)
    } else {
        (left_reg, right_reg)
    };

    // k=1 means "skip if comparison is true", k=0 means "skip if comparison is false"
    // For boolean result, we want: if true -> set true, if false -> set false
    // So we use k=1 (skip if true) with the JMP pattern
    let k = if negate { false } else { true }; // k=1 for normal comparison

    // EQ A B k: compare R[A] with R[B]
    // Note: comparison instructions don't produce results, they only test and skip
    emit(c, Instruction::create_abck(cmp_opcode, op1, op2, 0, k));

    // JMP over LFALSESKIP (offset = 1)
    emit(c, Instruction::create_sj(OpCode::Jmp, 1));

    // LFALSESKIP: load false into result register and skip next instruction
    emit(
        c,
        Instruction::encode_abc(OpCode::LFalseSkip, result_reg, 0, 0),
    );

    // LOADTRUE: load true into result register
    emit(
        c,
        Instruction::encode_abc(OpCode::LoadTrue, result_reg, 0, 0),
    );

    Ok(result_reg)
}

/// Compile comparison operator with immediate value to produce boolean result
/// Generates: CMPI + JMP + LFALSESKIP + LOADTRUE pattern
fn compile_comparison_imm_to_bool(
    c: &mut Compiler,
    op_kind: BinaryOperator,
    operand_reg: u32,
    result_reg: u32,
    imm_val: i32,
) -> Result<u32, String> {
    // Immediate comparison instructions: EQI, LTI, LEI, GTI, GEI
    // Pattern same as register comparison: CMPI(k=1) + JMP + LFALSESKIP + LOADTRUE

    let (cmp_opcode, negate) = match op_kind {
        BinaryOperator::OpEq => (OpCode::EqI, false),
        BinaryOperator::OpNe => (OpCode::EqI, true),
        BinaryOperator::OpLt => (OpCode::LtI, false),
        BinaryOperator::OpLe => (OpCode::LeI, false),
        BinaryOperator::OpGt => (OpCode::GtI, false),
        BinaryOperator::OpGe => (OpCode::GeI, false),
        _ => unreachable!(),
    };

    // Encode immediate value with OFFSET_SB = 128 for signed B field
    let imm = ((imm_val + 128) & 0xFF) as u32;

    let k = if negate { false } else { true };

    // EQI A sB k: compare R[A] with immediate sB, k controls skip behavior
    emit(
        c,
        Instruction::create_abck(cmp_opcode, operand_reg, imm, 0, k),
    );

    // JMP over LFALSESKIP (offset = 1)
    emit(c, Instruction::create_sj(OpCode::Jmp, 1));

    // LFALSESKIP: load false and skip next instruction
    emit(
        c,
        Instruction::encode_abc(OpCode::LFalseSkip, result_reg, 0, 0),
    );

    // LOADTRUE: load true
    emit(
        c,
        Instruction::encode_abc(OpCode::LoadTrue, result_reg, 0, 0),
    );

    Ok(result_reg)
}

fn compile_unary_expr_to(
    c: &mut Compiler,
    expr: &LuaUnaryExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    let result_reg = dest.unwrap_or_else(|| alloc_register(c));

    // Get operator from text
    let op_token = expr.get_op_token().ok_or("error")?;
    let op_kind = op_token.get_op();

    let operand = expr.get_expr().ok_or("Unary expression missing operand")?;

    // Constant folding optimizations
    if let LuaExpr::LiteralExpr(lit_expr) = &operand {
        match lit_expr.get_literal() {
            Some(LuaLiteralToken::Number(num_token)) => {
                if op_kind == UnaryOperator::OpUnm {
                    // Negative number literal: emit LOADI/LOADK with negated value
                    if !num_token.is_float() {
                        let int_val = num_token.get_int_value();
                        let neg_val = -int_val;

                        // Use LOADI for small integers
                        if let Some(_) = emit_loadi(c, result_reg, neg_val) {
                            return Ok(result_reg);
                        }
                    }

                    // For floats or large integers, use constant
                    let num_val = if num_token.is_float() {
                        LuaValue::number(-num_token.get_float_value())
                    } else {
                        LuaValue::integer(-num_token.get_int_value())
                    };
                    let const_idx = add_constant(c, num_val);
                    emit_loadk(c, result_reg, const_idx);
                    return Ok(result_reg);
                }
            }
            Some(LuaLiteralToken::Bool(b)) => {
                if op_kind == UnaryOperator::OpNot {
                    // not true -> LOADFALSE, not false -> LOADTRUE
                    if b.is_true() {
                        emit(
                            c,
                            Instruction::encode_abc(OpCode::LoadFalse, result_reg, 0, 0),
                        );
                    } else {
                        emit(
                            c,
                            Instruction::encode_abc(OpCode::LoadTrue, result_reg, 0, 0),
                        );
                    }
                    return Ok(result_reg);
                }
            }
            Some(LuaLiteralToken::Nil(_)) => {
                if op_kind == UnaryOperator::OpNot {
                    // not nil -> LOADTRUE
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::LoadTrue, result_reg, 0, 0),
                    );
                    return Ok(result_reg);
                }
            }
            _ => {}
        }
    }

    // Regular unary operation
    let operand_reg = compile_expr(c, &operand)?;

    match op_kind {
        UnaryOperator::OpBNot => {
            emit(
                c,
                Instruction::encode_abc(OpCode::BNot, result_reg, operand_reg, 0),
            );
        }
        UnaryOperator::OpUnm => {
            emit(
                c,
                Instruction::encode_abc(OpCode::Unm, result_reg, operand_reg, 0),
            );
        }
        UnaryOperator::OpNot => {
            emit(
                c,
                Instruction::encode_abc(OpCode::Not, result_reg, operand_reg, 0),
            );
        }
        UnaryOperator::OpLen => {
            emit(
                c,
                Instruction::encode_abc(OpCode::Len, result_reg, operand_reg, 0),
            );
        }
        UnaryOperator::OpNop => {
            // No operation, just move operand to result
            if operand_reg != result_reg {
                emit_move(c, result_reg, operand_reg);
            }
        }
    }

    Ok(result_reg)
}

fn compile_paren_expr_to(
    c: &mut Compiler,
    expr: &LuaParenExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    // Get inner expression from children
    let inner_expr = expr.get_expr().ok_or("missing inner expr")?;
    let reg = compile_expr_to(c, &inner_expr, dest)?;
    Ok(reg)
}

/// Compile function call expression
pub fn compile_call_expr(c: &mut Compiler, expr: &LuaCallExpr) -> Result<u32, String> {
    // For statement context (discard returns), use num_returns = 0
    // This will generate CALL with C=1 (0 returns expected)
    compile_call_expr_with_returns(c, expr, 0)
}

/// Compile a call expression with specified number of expected return values (public API)
pub fn compile_call_expr_with_returns(
    c: &mut Compiler,
    expr: &LuaCallExpr,
    num_returns: usize,
) -> Result<u32, String> {
    compile_call_expr_with_returns_and_dest(c, expr, num_returns, None)
}

fn compile_call_expr_to(
    c: &mut Compiler,
    expr: &LuaCallExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    // Compile call with specified destination
    // The call handler will decide whether to use dest as func_reg or allocate fresh registers
    compile_call_expr_with_returns_and_dest(c, expr, 1, dest)
}

/// Compile a call expression with specified number of expected return values and optional dest
pub fn compile_call_expr_with_returns_and_dest(
    c: &mut Compiler,
    expr: &LuaCallExpr,
    num_returns: usize,
    dest: Option<u32>,
) -> Result<u32, String> {
    use emmylua_parser::{LuaExpr, LuaIndexKey};

    // Get prefix (function) and arguments from children
    let prefix_expr = expr.get_prefix_expr().ok_or("missing prefix expr")?;
    let arg_exprs = expr
        .get_args_list()
        .ok_or("missing args list")?
        .get_args()
        .collect::<Vec<_>>();

    // Check if this is a method call (obj:method syntax)
    let is_method = if let LuaExpr::IndexExpr(index_expr) = &prefix_expr {
        index_expr
            .get_index_token()
            .map(|t| t.is_colon())
            .unwrap_or(false)
    } else {
        false
    };

    // Track if we need to move return values back to original dest
    let mut need_move_to_dest = false;
    let original_dest = dest;

    // Handle method call with SELF instruction
    let func_reg = if is_method {
        if let LuaExpr::IndexExpr(index_expr) = &prefix_expr {
            // Method call: obj:method(args) → SELF instruction
            // SELF A B C: R(A+1) = R(B); R(A) = R(B)[C]
            // A = function register, A+1 = self parameter
            let func_reg = dest.unwrap_or_else(|| alloc_register(c));

            // Ensure func_reg+1 is allocated for self parameter
            while c.freereg <= func_reg + 1 {
                alloc_register(c);
            }

            // Compile object (table)
            let obj_expr = index_expr
                .get_prefix_expr()
                .ok_or("Method call missing object")?;
            let obj_reg = compile_expr(c, &obj_expr)?;

            // Get method name
            let method_name =
                if let Some(LuaIndexKey::Name(name_token)) = index_expr.get_index_key() {
                    name_token.get_name_text().to_string()
                } else {
                    return Err("Method call requires name index".to_string());
                };

            // Add method name to constants
            let lua_str = create_string_value(c, &method_name);
            let key_idx = add_constant_dedup(c, lua_str);

            // Emit SELF instruction: R(func_reg+1) = R(obj_reg); R(func_reg) = R(obj_reg)[key]
            emit(
                c,
                Instruction::create_abck(
                    OpCode::Self_,
                    func_reg,
                    obj_reg,
                    key_idx,
                    true, // k=1: C is constant index
                ),
            );

            func_reg
        } else {
            unreachable!("is_method but not IndexExpr")
        }
    } else {
        // Regular call: compile function expression
        let temp_func_reg = compile_expr(c, &prefix_expr)?;

        // CRITICAL FIX: When function call returns values and will be used in an expression,
        // we need to avoid overwriting the function value itself.
        //
        // Example: `f()` where f is in R[0] - if we call directly, return value overwrites f!
        // Solution: If temp_func_reg is an "old" register (< freereg when we started),
        // move the function to a new register first.
        //
        // However, we need to distinguish:
        // - `f = load(...)` - first assignment, can reuse register ✓
        // - `assert(f() == 30)` - f exists, must preserve it ✓
        //
        // The key insight: If dest is specified, caller wants a specific target.
        // If dest is NOT specified AND we need returns, allocate fresh register to be safe.

        let func_reg = if let Some(d) = dest {
            // Caller specified a destination - use it
            // CRITICAL CHECK: Verify that arguments won't overwrite active local variables!
            // Arguments will be placed at R[d+1], R[d+2], etc.
            // If any of these overlap with active locals (< nactvar), we need to use a different register
            let nactvar = c.nactvar as u32;
            let args_start = d + 1;

            // Check if args_start would overwrite active locals
            // Active locals occupy R[0] through R[nactvar-1]
            // If args_start < nactvar, arguments would overwrite locals!
            if args_start < nactvar {
                // Conflict! Arguments would overwrite active locals
                // Solution: Place function at freereg (after all locals) and move result back later
                let new_func_reg = if c.freereg < nactvar {
                    // Ensure freereg is at least nactvar
                    c.freereg = nactvar;
                    alloc_register(c)
                } else {
                    alloc_register(c)
                };
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Move, new_func_reg, temp_func_reg, 0),
                );
                need_move_to_dest = true; // Remember to move result back to original dest
                new_func_reg
            } else {
                // No conflict - safe to use dest
                if d != temp_func_reg {
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::Move, d, temp_func_reg, 0),
                    );
                }
                // CRITICAL: Reset freereg to just past func_reg
                // This ensures arguments compile into consecutive registers starting from func_reg+1
                c.freereg = d + 1;
                d
            }
        } else if num_returns > 0 {
            // No dest specified, but need return values - this is expression context!
            // CRITICAL: Must preserve local variables!
            // Check if temp_func_reg is a local variable (< nactvar)
            let nactvar = c.nactvar as u32;
            if temp_func_reg < nactvar {
                // Function is a local variable - must preserve it!
                let new_reg = alloc_register(c);
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Move, new_reg, temp_func_reg, 0),
                );
                new_reg
            } else if temp_func_reg + 1 == c.freereg {
                // Function was just loaded into a fresh temporary register - safe to reuse
                temp_func_reg
            } else {
                // Function is in an "old" temporary register - must preserve it!
                let new_reg = alloc_register(c);
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Move, new_reg, temp_func_reg, 0),
                );
                new_reg
            }
        } else {
            // No return values needed - can reuse temp_func_reg
            temp_func_reg
        };

        func_reg
    };

    // Compile arguments into consecutive registers
    // For method calls: func_reg+1 is self, args start at func_reg+2
    // For regular calls: args start at func_reg+1
    let args_start = if is_method {
        func_reg + 2
    } else {
        func_reg + 1
    };
    let mut arg_regs = Vec::new();
    let mut last_arg_is_call_all_out = false;

    // Save freereg before compiling arguments
    // We'll reset it before each argument so they compile into consecutive registers
    let saved_freereg = c.freereg;

    // CRITICAL: Pre-reserve all argument registers before compiling any arguments
    // This prevents nested call expressions from overwriting earlier argument registers
    let num_fixed_args = arg_exprs.len();
    let args_end = args_start + num_fixed_args as u32;

    // Allocate all argument registers upfront
    while c.freereg < args_end {
        alloc_register(c);
    }

    // CRITICAL: Compile arguments directly to their target positions
    // This is how standard Lua ensures arguments are in consecutive registers
    // Each argument should be compiled to args_start + i
    for (i, arg_expr) in arg_exprs.iter().enumerate() {
        let is_last = i == arg_exprs.len() - 1;
        let arg_dest = args_start + i as u32;

        // CRITICAL: Before compiling each argument, ensure freereg is beyond ALL argument slots
        // This prevents expressions from allocating temps that conflict with argument positions
        if c.freereg < args_end {
            c.freereg = args_end;
        }

        // Ensure max_stack_size can accommodate this register
        if arg_dest as usize >= c.chunk.max_stack_size {
            c.chunk.max_stack_size = (arg_dest + 1) as usize;
        }

        // OPTIMIZATION: If last argument is ... (vararg), use "all out" mode
        if is_last {
            if let LuaExpr::LiteralExpr(lit_expr) = arg_expr {
                if matches!(lit_expr.get_literal(), Some(LuaLiteralToken::Dots(_))) {
                    // Vararg as last argument: VARARG with C=0 (all out)
                    emit(c, Instruction::encode_abc(OpCode::Vararg, arg_dest, 0, 0));
                    arg_regs.push(arg_dest);
                    last_arg_is_call_all_out = true;
                    break;
                }
            }
        }

        // OPTIMIZATION: If last argument is a call, use "all out" mode
        // Use recursive compile_call_expr_with_returns to support method calls (SELF instruction)
        if is_last && matches!(arg_expr, LuaExpr::CallExpr(_)) {
            if let LuaExpr::CallExpr(call_expr) = arg_expr {
                // Compile inner call with "all out" mode (num_returns = 0 means variable returns)
                // Note: We need to handle this specially for method calls
                let inner_prefix = call_expr.get_prefix_expr().ok_or("missing call prefix")?;

                // Check if inner call is a method call
                let inner_is_method = if let LuaExpr::IndexExpr(index_expr) = &inner_prefix {
                    index_expr
                        .get_index_token()
                        .map(|t| t.is_colon())
                        .unwrap_or(false)
                } else {
                    false
                };

                // Handle method call with SELF instruction
                let call_reg = if inner_is_method {
                    if let LuaExpr::IndexExpr(index_expr) = &inner_prefix {
                        let func_reg = alloc_register(c);
                        alloc_register(c); // Reserve A+1 for self

                        let obj_expr = index_expr
                            .get_prefix_expr()
                            .ok_or("Method call missing object")?;
                        let obj_reg = compile_expr(c, &obj_expr)?;

                        let method_name = if let Some(LuaIndexKey::Name(name_token)) =
                            index_expr.get_index_key()
                        {
                            name_token.get_name_text().to_string()
                        } else {
                            return Err("Method call requires name index".to_string());
                        };

                        let lua_str = create_string_value(c, &method_name);
                        let key_idx = add_constant_dedup(c, lua_str);

                        emit(
                            c,
                            Instruction::create_abck(
                                OpCode::Self_,
                                func_reg,
                                obj_reg,
                                key_idx,
                                true,
                            ),
                        );

                        func_reg
                    } else {
                        unreachable!("inner_is_method but not IndexExpr")
                    }
                } else {
                    // For regular call, we need to place it at arg_dest (outer call's argument position)
                    // to avoid register conflicts
                    let temp_func_reg = compile_expr(c, &inner_prefix)?;
                    if temp_func_reg != arg_dest {
                        ensure_register(c, arg_dest);
                        emit_move(c, arg_dest, temp_func_reg);
                        arg_dest
                    } else {
                        temp_func_reg
                    }
                };

                // Compile call arguments
                let call_args_start = if inner_is_method {
                    call_reg + 2
                } else {
                    call_reg + 1
                };
                let call_arg_exprs = call_expr
                    .get_args_list()
                    .ok_or("missing args list")?
                    .get_args()
                    .collect::<Vec<_>>();

                // CRITICAL: Reset freereg so inner call's arguments compile into correct positions
                c.freereg = call_args_start;

                let mut call_arg_regs = Vec::new();
                // CRITICAL: Allocate all argument registers upfront to prevent conflicts
                let num_call_args = call_arg_exprs.len();
                let call_args_end = call_args_start + num_call_args as u32;
                while c.freereg < call_args_end {
                    alloc_register(c);
                }

                for (j, call_arg) in call_arg_exprs.iter().enumerate() {
                    let call_arg_dest = call_args_start + j as u32;
                    // Reset freereg before each argument to protect argument slots
                    if c.freereg < call_args_end {
                        c.freereg = call_args_end;
                    }
                    let arg_reg = compile_expr_to(c, call_arg, Some(call_arg_dest))?;
                    call_arg_regs.push(arg_reg);
                }

                // Move call arguments if needed
                for (j, &reg) in call_arg_regs.iter().enumerate() {
                    let target = call_args_start + j as u32;
                    if reg != target {
                        while c.freereg <= target {
                            alloc_register(c);
                        }
                        emit_move(c, target, reg);
                    }
                }

                // Emit call with "all out" (C=0)
                let inner_arg_count = call_arg_exprs.len();
                let inner_b_param = if inner_is_method {
                    (inner_arg_count + 2) as u32 // +1 for self, +1 for Lua convention
                } else {
                    (inner_arg_count + 1) as u32
                };
                emit(
                    c,
                    Instruction::encode_abc(
                        OpCode::Call,
                        call_reg,
                        inner_b_param,
                        0, // C=0: all out
                    ),
                );

                arg_regs.push(call_reg);
                last_arg_is_call_all_out = true;
                break;
            }
        }

        // Compile argument directly to its target position
        let arg_reg = if matches!(arg_expr, LuaExpr::CallExpr(_)) {
            // Pass dest to allow nested expressions to use correct registers
            let call_reg = compile_expr_to(c, arg_expr, Some(arg_dest))?;
            // Move result to target position if needed
            if call_reg != arg_dest {
                ensure_register(c, arg_dest);
                emit_move(c, arg_dest, call_reg);
                arg_dest
            } else {
                call_reg
            }
        } else {
            compile_expr_to(c, arg_expr, Some(arg_dest))?
        };
        arg_regs.push(arg_reg);
    }

    // Restore freereg to saved value or update to after last argument
    // whichever is higher (to account for any temporary registers used)
    let after_args = args_start + arg_regs.len() as u32;
    c.freereg = std::cmp::max(saved_freereg, after_args);

    // Check if arguments are already in the correct positions
    let mut need_move = false;
    if !last_arg_is_call_all_out {
        for (i, &arg_reg) in arg_regs.iter().enumerate() {
            if arg_reg != args_start + i as u32 {
                need_move = true;
                break;
            }
        }
    }

    // If arguments are not in consecutive registers, we need to move them
    // CRITICAL FIX: Move from back to front to avoid overwriting!
    if need_move {
        // Reserve registers for arguments
        while c.freereg < args_start + arg_regs.len() as u32 {
            alloc_register(c);
        }

        // Move arguments to correct positions FROM BACK TO FRONT to avoid overwriting
        for i in (0..arg_regs.len()).rev() {
            let arg_reg = arg_regs[i];
            let target_reg = args_start + i as u32;
            if arg_reg != target_reg {
                emit_move(c, target_reg, arg_reg);
            }
        }
    }

    // Emit call instruction
    // A = function register
    // B = number of arguments + 1, or 0 if last arg was "all out" call
    //     For method calls, B includes the implicit self parameter
    // C = number of expected return values + 1 (1 means 0 returns, 2 means 1 return, 0 means all returns)
    let arg_count = arg_exprs.len();
    let b_param = if last_arg_is_call_all_out {
        0 // B=0: all in
    } else {
        // For method calls, add 1 for implicit self parameter
        let total_args = if is_method { arg_count + 1 } else { arg_count };
        (total_args + 1) as u32
    };
    let c_param = (num_returns + 1) as u32;

    emit(
        c,
        Instruction::encode_abc(OpCode::Call, func_reg, b_param, c_param),
    );

    // After CALL: adjust freereg based on return values
    // CALL places return values starting at func_reg
    // If num_returns == 0, CALL discards all returns
    // If num_returns > 0, return values are in func_reg .. func_reg + num_returns - 1
    //
    // CRITICAL: freereg can only be set to func_reg + num_returns if that's >= nactvar
    // We cannot reclaim registers occupied by active local variables!
    let new_freereg = func_reg + num_returns as u32;
    if new_freereg >= c.nactvar as u32 {
        c.freereg = new_freereg;
    }
    // If new_freereg < nactvar, keep freereg unchanged (locals are still alive)

    // If we had to move function to avoid conflicts, move return values back to original dest
    if need_move_to_dest {
        if let Some(d) = original_dest {
            // Move return values from func_reg to original dest
            for i in 0..num_returns {
                emit_move(c, d + i as u32, func_reg + i as u32);
            }
            return Ok(d);
        }
    }

    Ok(func_reg)
}

fn compile_index_expr_to(
    c: &mut Compiler,
    expr: &LuaIndexExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    // Get prefix (table) expression
    let prefix_expr = expr
        .get_prefix_expr()
        .ok_or("Index expression missing table")?;
    let table_reg = compile_expr(c, &prefix_expr)?;

    // Lua optimization: reuse table_reg if:
    // 1. No specific dest is requested
    // 2. table_reg is a temporary register (>= nvarstack, meaning it's not a local variable)
    // 3. table_reg is the last allocated register (freereg - 1)
    // This generates GETFIELD A B C where A=B (overwrites the table with the result)
    // CRITICAL: Never reuse a local variable's register as it would corrupt the variable!
    let nvarstack = nvarstack(c);
    let can_reuse_table = table_reg >= nvarstack && table_reg + 1 == c.freereg;
    let result_reg = dest.unwrap_or_else(|| {
        if can_reuse_table {
            table_reg
        } else {
            alloc_register(c)
        }
    });

    // Get index key and emit optimized instruction if possible
    let key = expr.get_index_key().ok_or("Index expression missing key")?;
    match key {
        LuaIndexKey::Integer(number_token) => {
            // Optimized: table[integer_literal] -> GetTableI
            // C field is 9 bits, so max value is 511
            let int_value = number_token.get_int_value();
            if int_value >= 0 && int_value <= 511 {
                // Use GetTableI: R(A) := R(B)[C]
                emit(
                    c,
                    Instruction::encode_abc(OpCode::GetI, result_reg, table_reg, int_value as u32),
                );
                return Ok(result_reg);
            }
            // Fallback for out-of-range integers
            let num_value = LuaValue::integer(int_value);
            let const_idx = add_constant(c, num_value);
            let key_reg = alloc_register(c);
            emit_load_constant(c, key_reg, const_idx);
            emit(
                c,
                Instruction::encode_abc(OpCode::GetTable, result_reg, table_reg, key_reg),
            );
            Ok(result_reg)
        }
        LuaIndexKey::Name(name_token) => {
            // Optimized: table.field -> GetField
            let field_name = name_token.get_name_text();
            let lua_str = create_string_value(c, field_name);
            let const_idx = add_constant_dedup(c, lua_str);
            // Use GetField: R(A) := R(B)[K(C)] with k=1
            // ABC format: A=dest, B=table, C=const_idx
            if const_idx <= Instruction::MAX_B {
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::GetField,
                        result_reg,
                        table_reg,
                        const_idx,
                        true,
                    ),
                );
                return Ok(result_reg);
            }
            // Fallback for large const_idx
            let key_reg = alloc_register(c);
            emit_loadk(c, key_reg, const_idx);
            emit(
                c,
                Instruction::encode_abc(OpCode::GetTable, result_reg, table_reg, key_reg),
            );
            Ok(result_reg)
        }
        LuaIndexKey::String(string_token) => {
            // Optimized: table["string"] -> GetField
            let string_value = string_token.get_value();
            let lua_str = create_string_value(c, &string_value);
            let const_idx = add_constant_dedup(c, lua_str);
            if const_idx <= Instruction::MAX_B {
                emit(
                    c,
                    Instruction::create_abck(
                        OpCode::GetField,
                        result_reg,
                        table_reg,
                        const_idx,
                        true,
                    ),
                );
                return Ok(result_reg);
            }
            // Fallback
            let key_reg = alloc_register(c);
            emit_loadk(c, key_reg, const_idx);
            emit(
                c,
                Instruction::encode_abc(OpCode::GetTable, result_reg, table_reg, key_reg),
            );
            Ok(result_reg)
        }
        LuaIndexKey::Expr(key_expr) => {
            // Generic: table[expr] -> GetTable
            let key_reg = compile_expr(c, &key_expr)?;
            emit(
                c,
                Instruction::encode_abc(OpCode::GetTable, result_reg, table_reg, key_reg),
            );
            Ok(result_reg)
        }
        LuaIndexKey::Idx(_i) => {
            // Fallback for other index types
            Err("Unsupported index key type".to_string())
        }
    }
}

fn compile_table_expr_to(
    c: &mut Compiler,
    expr: &LuaTableExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    let reg = dest.unwrap_or_else(|| alloc_register(c));

    // Get all fields
    let fields: Vec<_> = expr.get_fields().collect();

    // Separate array part from hash part to count sizes
    let mut array_count = 0;
    let mut hash_count = 0;

    for (i, field) in fields.iter().enumerate() {
        if field.is_value_field() {
            // Check if it's a simple value (not ... or call as last element)
            if let Some(value_expr) = field.get_value_expr() {
                let is_dots = is_vararg_expr(&value_expr);
                let is_call = matches!(&value_expr, LuaExpr::CallExpr(_));
                let is_last = i == fields.len() - 1;

                // Stop counting if we hit ... or call as last element
                if is_last && (is_dots || is_call) {
                    break;
                }
            }
            array_count += 1;
        } else {
            // Hash field
            hash_count += 1;
        }
    }

    // Helper function to encode table size (Lua's int2fb encoding)
    fn int2fb(x: usize) -> u32 {
        if x < 8 {
            x as u32
        } else {
            let mut e = 0;
            let mut x = x - 1;
            while x >= 16 {
                x = (x + 1) >> 1;
                e += 1;
            }
            if x < 8 {
                ((e + 1) << 3 | x) as u32
            } else {
                ((e + 2) << 3 | (x - 8)) as u32
            }
        }
    }

    // Create table with size hints
    // NEWTABLE A B C: B = hash size (encoded), C = array size (encoded)
    let b_param = int2fb(hash_count);
    let c_param = int2fb(array_count);
    emit(
        c,
        Instruction::encode_abc(OpCode::NewTable, reg, b_param, c_param),
    );

    // EXTRAARG instruction (always 0 for now, used for extended parameters)
    emit(c, Instruction::create_ax(OpCode::ExtraArg, 0));

    if fields.is_empty() {
        return Ok(reg);
    }

    // Track array indices that need to be processed
    let mut array_idx = 0;
    let values_start = reg + 1;
    let mut has_vararg_at_end = false;
    let mut has_call_at_end = false;

    // Process all fields in source order
    // Array elements are loaded to registers, hash fields are set immediately
    for (field_idx, field) in fields.iter().enumerate() {
        let is_last_field = field_idx == fields.len() - 1;

        if field.is_value_field() {
            // Array element or special case (vararg/call at end)
            if let Some(value_expr) = field.get_value_expr() {
                let is_dots = is_vararg_expr(&value_expr);
                let is_call = matches!(&value_expr, LuaExpr::CallExpr(_));

                if is_last_field && is_dots {
                    // VarArg expansion: {...} or {a, b, ...}
                    // Will be handled after all hash fields
                    has_vararg_at_end = true;
                    continue;
                } else if is_last_field && is_call {
                    // Call as last element: returns multiple values
                    // Will be handled after all hash fields
                    has_call_at_end = true;
                    continue;
                }

                // Regular array element: load to consecutive register
                let target_reg = values_start + array_idx;
                while c.freereg <= target_reg {
                    alloc_register(c);
                }
                let value_reg = compile_expr_to(c, &value_expr, Some(target_reg))?;
                if value_reg != target_reg {
                    emit_move(c, target_reg, value_reg);
                }
                array_idx += 1;
            }
        } else {
            // Hash field: process immediately with SETFIELD/SETI/SETTABLE
            let Some(field_key) = field.get_field_key() else {
                continue;
            };

            let key_reg = match field_key {
                LuaIndexKey::Name(name_token) => {
                    // key is an identifier - use SetField optimization
                    let key_name = name_token.get_name_text();
                    let lua_str = create_string_value(c, key_name);
                    let const_idx = add_constant_dedup(c, lua_str);

                    // Try to compile value as constant first (for RK optimization)
                    let (value_operand, use_constant) =
                        if let Some(value_expr) = field.get_value_expr() {
                            if let Some(k_idx) = try_expr_as_constant(c, &value_expr) {
                                (k_idx, true)
                            } else {
                                (compile_expr(c, &value_expr)?, false)
                            }
                        } else {
                            let r = alloc_register(c);
                            emit_load_nil(c, r);
                            (r, false)
                        };

                    // Use SetField: R(A)[K(B)] := RK(C)
                    // k=1 means C is constant index, k=0 means C is register
                    emit(
                        c,
                        Instruction::create_abck(
                            OpCode::SetField,
                            reg,
                            const_idx,
                            value_operand,
                            use_constant,
                        ),
                    );

                    continue; // Skip the SetTable at the end
                }
                LuaIndexKey::String(string_token) => {
                    // key is a string literal - use SetField optimization
                    let string_value = string_token.get_value();
                    let lua_str = create_string_value(c, &string_value);
                    let const_idx = add_constant_dedup(c, lua_str);

                    // Try to compile value as constant first (for RK optimization)
                    let (value_operand, use_constant) =
                        if let Some(value_expr) = field.get_value_expr() {
                            if let Some(k_idx) = try_expr_as_constant(c, &value_expr) {
                                (k_idx, true)
                            } else {
                                (compile_expr(c, &value_expr)?, false)
                            }
                        } else {
                            let r = alloc_register(c);
                            emit_load_nil(c, r);
                            (r, false)
                        };

                    // Use SetField: R(A)[K(B)] := RK(C)
                    // k=1 means C is constant index, k=0 means C is register
                    emit(
                        c,
                        Instruction::create_abck(
                            OpCode::SetField,
                            reg,
                            const_idx,
                            value_operand,
                            use_constant,
                        ),
                    );

                    continue; // Skip the SetTable at the end
                }
                LuaIndexKey::Integer(number_token) => {
                    // key is a numeric literal - try SETI optimization
                    if !number_token.is_float() {
                        let int_value = number_token.get_int_value();
                        // SETI: B field is sB (signed byte), range -128 to 127
                        if int_value >= -128 && int_value <= 127 {
                            // Try to compile value as constant first (for RK optimization)
                            let (value_operand, use_constant) =
                                if let Some(value_expr) = field.get_value_expr() {
                                    if let Some(k_idx) = try_expr_as_constant(c, &value_expr) {
                                        (k_idx, true)
                                    } else {
                                        (compile_expr(c, &value_expr)?, false)
                                    }
                                } else {
                                    let r = alloc_register(c);
                                    emit_load_nil(c, r);
                                    (r, false)
                                };

                            // Use SETI: R(A)[sB] := RK(C) where sB is signed byte
                            // Encode sB: add OFFSET_SB (128)
                            let encoded_b = (int_value + 128) as u32;
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::SetI,
                                    reg,
                                    encoded_b,
                                    value_operand,
                                    use_constant,
                                ),
                            );

                            continue; // Skip the SetTable at the end
                        }
                    }

                    // Fall back to SETTABLE for floats or large integers
                    let const_idx = if number_token.is_float() {
                        let num_value = number_token.get_float_value();
                        add_constant(c, LuaValue::number(num_value))
                    } else {
                        let int_value = number_token.get_int_value();
                        let num_value = LuaValue::integer(int_value);
                        add_constant(c, num_value)
                    };

                    let key_reg = alloc_register(c);
                    emit_load_constant(c, key_reg, const_idx);
                    key_reg
                }
                LuaIndexKey::Expr(key_expr) => {
                    // key is an expression - try to evaluate as constant integer for SETI
                    if let Some(int_val) = try_eval_const_int(&key_expr) {
                        // SETI: B field is sB (signed byte), range -128 to 127
                        if int_val >= -128 && int_val <= 127 {
                            // Use SETI for small integer keys
                            let (value_operand, use_constant) =
                                if let Some(value_expr) = field.get_value_expr() {
                                    if let Some(k_idx) = try_expr_as_constant(c, &value_expr) {
                                        (k_idx, true)
                                    } else {
                                        (compile_expr(c, &value_expr)?, false)
                                    }
                                } else {
                                    let r = alloc_register(c);
                                    emit_load_nil(c, r);
                                    (r, false)
                                };

                            // Encode sB: add OFFSET_SB (128)
                            let encoded_b = (int_val + 128) as u32;
                            emit(
                                c,
                                Instruction::create_abck(
                                    OpCode::SetI,
                                    reg,
                                    encoded_b,
                                    value_operand,
                                    use_constant,
                                ),
                            );

                            continue; // Skip the SetTable at the end
                        }
                    }

                    // Fall back to compiling key as expression
                    compile_expr(c, &key_expr)?
                }
                LuaIndexKey::Idx(_i) => {
                    return Err("Unsupported table field key type".to_string());
                }
            };

            // Compile value expression
            // Try to use constant optimization (RK operand)
            let (value_operand, use_constant) = if let Some(value_expr) = field.get_value_expr() {
                if let Some(k_idx) = try_expr_as_constant(c, &value_expr) {
                    (k_idx, true)
                } else {
                    (compile_expr(c, &value_expr)?, false)
                }
            } else {
                let r = alloc_register(c);
                emit_load_nil(c, r);
                (r, false)
            };

            // Set table field: table[key] = value
            // Use k-suffix if value is a constant
            emit(
                c,
                Instruction::create_abck(
                    OpCode::SetTable,
                    reg,
                    key_reg,
                    value_operand,
                    use_constant,
                ),
            );
        }
    }

    // Handle vararg or call at end (after all hash fields)
    if has_vararg_at_end {
        // VarArg expansion: {...} or {a, b, ...}
        emit(
            c,
            Instruction::encode_abc(OpCode::Vararg, values_start + array_idx, 0, 0),
        );

        // SetList with B=0 (all remaining values)
        let c_param = (array_idx as usize / 50) as u32;
        emit(c, Instruction::encode_abc(OpCode::SetList, reg, 0, c_param));

        c.freereg = reg + 1;
        return Ok(reg);
    }

    if has_call_at_end {
        // Call as last element - for now treat as single value
        // TODO: handle multiple return values properly
        let target_reg = values_start + array_idx;
        while c.freereg <= target_reg {
            alloc_register(c);
        }
        // Simplified: just count as one more array element
        array_idx += 1;
    }

    // Emit SETLIST for all array elements at the end
    // Process in batches of 50 (LFIELDS_PER_FLUSH)
    if array_idx > 0 {
        const BATCH_SIZE: u32 = 50;
        let mut batch_start = 0;

        while batch_start < array_idx {
            let batch_end = (batch_start + BATCH_SIZE).min(array_idx);
            let batch_count = batch_end - batch_start;
            let c_param = (batch_start / BATCH_SIZE) as u32;

            emit(
                c,
                Instruction::encode_abc(OpCode::SetList, reg, batch_count, c_param),
            );

            batch_start = batch_end;
        }
    }

    // Free temporary registers used during table construction
    // Reset to table_reg + 1 to match luac's register allocation behavior
    c.freereg = reg + 1;

    Ok(reg)
}

/// Compile a variable expression for assignment
pub fn compile_var_expr(c: &mut Compiler, var: &LuaVarExpr, value_reg: u32) -> Result<(), String> {
    match var {
        LuaVarExpr::NameExpr(name_expr) => {
            let name = name_expr.get_name_text().unwrap_or("".to_string());

            // Check if it's a local variable
            if let Some(local) = resolve_local(c, &name) {
                // Move to local register
                emit_move(c, local.register, value_reg);
                return Ok(());
            }

            // Try to resolve as upvalue from parent scope chain
            if let Some(upvalue_index) = resolve_upvalue_from_chain(c, &name) {
                let instr =
                    Instruction::encode_abc(OpCode::SetUpval, value_reg, upvalue_index as u32, 0);
                c.chunk.code.push(instr);
                return Ok(());
            }

            // Set global
            emit_set_global(c, &name, value_reg);
            Ok(())
        }
        LuaVarExpr::IndexExpr(index_expr) => {
            // Get table and key expressions from children
            let prefix_expr = index_expr
                .get_prefix_expr()
                .ok_or("Index expression missing table")?;

            let table_reg = compile_expr(c, &prefix_expr)?;

            // Determine key and emit optimized instruction if possible
            let index_key = index_expr
                .get_index_key()
                .ok_or("Index expression missing key")?;

            match index_key {
                LuaIndexKey::Integer(number_token) => {
                    // Optimized: table[integer] = value -> SETI A sB C k
                    // B field is sB (signed byte), range -128 to 127
                    let int_value = number_token.get_int_value();
                    if int_value >= -128 && int_value <= 127 {
                        // Use SETI: R(A)[sB] := RK(C)
                        // Encode sB: add OFFSET_SB (128) to get 0-255 range
                        let encoded_b = (int_value + 128) as u32;
                        emit(
                            c,
                            Instruction::encode_abc(OpCode::SetI, table_reg, encoded_b, value_reg),
                        );
                        return Ok(());
                    }
                    // Fallback for out-of-range integers
                    let num_value = LuaValue::integer(int_value);
                    let const_idx = add_constant(c, num_value);
                    let key_reg = alloc_register(c);
                    emit_load_constant(c, key_reg, const_idx);
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::SetTable, table_reg, key_reg, value_reg),
                    );
                    Ok(())
                }
                LuaIndexKey::Name(name_token) => {
                    // Optimized: table.field = value -> SetField
                    let field_name = name_token.get_name_text().to_string();
                    let lua_str = create_string_value(c, &field_name);
                    let const_idx = add_constant_dedup(c, lua_str);
                    // Use SetField: R(A)[K(B)] := RK(C)
                    // k=0 because value_reg is a register (already compiled)
                    if const_idx <= Instruction::MAX_B {
                        emit(
                            c,
                            Instruction::create_abck(
                                OpCode::SetField,
                                table_reg,
                                const_idx,
                                value_reg,
                                false, // k=0: C is register
                            ),
                        );
                        return Ok(());
                    }
                    // Fallback
                    let key_reg = alloc_register(c);
                    emit_load_constant(c, key_reg, const_idx);
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::SetTable, table_reg, key_reg, value_reg),
                    );
                    Ok(())
                }
                LuaIndexKey::String(string_token) => {
                    // Optimized: table["string"] = value -> SETFIELD A B C k
                    // A: table register
                    // B: key (constant index)
                    // C: value (register or constant, determined by k)
                    let string_value = string_token.get_value();
                    let lua_str = create_string_value(c, &string_value);
                    let const_idx = add_constant_dedup(c, lua_str);
                    if const_idx <= Instruction::MAX_B {
                        emit(
                            c,
                            Instruction::create_abck(
                                OpCode::SetField,
                                table_reg,
                                const_idx,
                                value_reg,
                                false, // k=0: C is register (value_reg is a register!)
                            ),
                        );
                        return Ok(());
                    }
                    // Fallback
                    let key_reg = alloc_register(c);
                    emit_load_constant(c, key_reg, const_idx);
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::SetTable, table_reg, key_reg, value_reg),
                    );
                    Ok(())
                }
                LuaIndexKey::Expr(key_expr) => {
                    // Generic: table[expr] = value -> SetTable
                    let key_reg = compile_expr(c, &key_expr)?;
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::SetTable, table_reg, key_reg, value_reg),
                    );
                    Ok(())
                }
                LuaIndexKey::Idx(_i) => Err("Unsupported index key type".to_string()),
            }
        }
    }
}

pub fn compile_closure_expr(
    c: &mut Compiler,
    closure: &LuaClosureExpr,
    is_method: bool,
) -> Result<u32, String> {
    compile_closure_expr_to(c, closure, None, is_method)
}

pub fn compile_closure_expr_to(
    c: &mut Compiler,
    closure: &LuaClosureExpr,
    dest: Option<u32>,
    is_method: bool,
) -> Result<u32, String> {
    let params_list = closure
        .get_params_list()
        .ok_or("closure missing params list")?;

    let params = params_list.get_params().collect::<Vec<_>>();

    // Handle empty function body (e.g., function noop() end)
    let has_body = closure.get_block().is_some();

    // Create a new compiler for the function body with parent scope chain
    // No need to sync anymore - scope_chain is already current
    let mut func_compiler = Compiler::new_with_parent(c.scope_chain.clone(), c.vm_ptr);

    // For methods (function defined with colon syntax), add implicit 'self' parameter
    let mut param_offset = 0;
    if is_method {
        func_compiler
            .scope_chain
            .borrow_mut()
            .locals
            .push(super::Local {
                name: "self".to_string(),
                depth: 0,
                register: 0,
                is_const: false,
                is_to_be_closed: false,
            });
        func_compiler.chunk.locals.push("self".to_string());
        param_offset = 1;
    }

    // Set up parameters as local variables
    let mut has_vararg = false;
    let mut regular_param_count = 0;
    for (i, param) in params.iter().enumerate() {
        // Check if this is a vararg parameter
        if param.is_dots() {
            has_vararg = true;
            // Don't add ... to locals or count it as a regular parameter
            continue;
        }

        // Try to get parameter name
        let param_name = if let Some(name_token) = param.get_name_token() {
            name_token.get_name_text().to_string()
        } else {
            format!("param{}", i + 1)
        };

        let reg_index = (regular_param_count + param_offset) as u32;
        func_compiler
            .scope_chain
            .borrow_mut()
            .locals
            .push(super::Local {
                name: param_name.clone(),
                depth: 0,
                register: reg_index,
                is_const: false,
                is_to_be_closed: false,
            });
        func_compiler.chunk.locals.push(param_name);
        regular_param_count += 1;
    }

    func_compiler.chunk.param_count = regular_param_count + param_offset;
    func_compiler.chunk.is_vararg = has_vararg;
    func_compiler.freereg = (regular_param_count + param_offset) as u32;
    func_compiler.nactvar = (regular_param_count + param_offset) as usize;

    // Emit VarargPrep instruction if function accepts varargs
    // VARARGPREP A: A = number of fixed parameters (not counting ...)
    if has_vararg {
        let varargprep_instr = Instruction::encode_abc(
            OpCode::VarargPrep,
            (regular_param_count + param_offset) as u32,
            0,
            0,
        );
        func_compiler.chunk.code.push(varargprep_instr);
    }

    // Compile function body (skip if empty)
    if has_body {
        let body = closure.get_block().unwrap();
        compile_block(&mut func_compiler, &body)?;
    }

    // Add implicit return if needed
    // Lua 5.4 ALWAYS adds a final RETURN0 at the end of functions for safety
    // This serves as a fallthrough in case execution reaches the end
    if func_compiler.chunk.code.is_empty() {
        // Empty function - use Return0
        let ret_instr = Instruction::encode_abc(OpCode::Return0, 0, 0, 0);
        func_compiler.chunk.code.push(ret_instr);
    } else {
        let last_opcode = Instruction::get_opcode(*func_compiler.chunk.code.last().unwrap());
        if last_opcode != OpCode::Return0 {
            // Always add final Return0 for fallthrough protection
            let ret_instr = Instruction::encode_abc(OpCode::Return0, 0, 0, 0);
            func_compiler.chunk.code.push(ret_instr);
        }
    }

    // Set max_stack_size to the maximum of peak_freereg and current max_stack_size
    // peak_freereg tracks registers allocated via alloc_register()
    // but max_stack_size may be higher due to direct register usage via dest parameter
    func_compiler.chunk.max_stack_size = std::cmp::max(
        func_compiler.peak_freereg as usize,
        func_compiler.chunk.max_stack_size,
    );

    // Store upvalue information from scope_chain
    let upvalues = func_compiler.scope_chain.borrow().upvalues.clone();
    func_compiler.chunk.upvalue_count = upvalues.len();
    func_compiler.chunk.upvalue_descs = upvalues
        .iter()
        .map(|uv| UpvalueDesc {
            is_local: uv.is_local,
            index: uv.index,
        })
        .collect();

    // Move child chunks from func_compiler to its own chunk's child_protos
    let child_protos: Vec<std::rc::Rc<Chunk>> = func_compiler
        .child_chunks
        .into_iter()
        .map(std::rc::Rc::new)
        .collect();
    func_compiler.chunk.child_protos = child_protos;

    // Add the function chunk to the parent compiler's child_chunks
    let chunk_index = c.child_chunks.len();
    c.child_chunks.push(func_compiler.chunk);

    // Emit Closure instruction - use dest if provided
    let dest_reg = dest.unwrap_or_else(|| {
        let r = c.freereg;
        c.freereg += 1;
        r
    });

    // Update peak_freereg to account for this register
    // This is crucial when dest is provided (e.g., in assignments)
    if dest_reg + 1 > c.peak_freereg {
        c.peak_freereg = dest_reg + 1;
    }

    // Ensure max_stack_size accounts for this register
    if (dest_reg + 1) as usize > c.chunk.max_stack_size {
        c.chunk.max_stack_size = (dest_reg + 1) as usize;
    }

    let closure_instr = Instruction::encode_abx(OpCode::Closure, dest_reg, chunk_index as u32);
    c.chunk.code.push(closure_instr);

    // Note: Upvalue initialization is handled by the VM's exec_closure function
    // using the upvalue_descs from the child chunk. No additional instructions needed.

    Ok(dest_reg)
}
