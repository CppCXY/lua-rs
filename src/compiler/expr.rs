// Expression compilation - Using strong-typed AST nodes

use super::Compiler;
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

/// Compile any expression and return the register containing the result
/// If dest is Some(reg), try to compile directly into that register to avoid extra Move
pub fn compile_expr(c: &mut Compiler, expr: &LuaExpr) -> Result<u32, String> {
    compile_expr_to(c, expr, None)
}

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
            let lua_string = create_string_value(c, &s.get_value());
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

    // CONSTANT FOLDING: Check if both operands are numeric constants (including nested expressions)
    // This matches luac behavior: 1+1 -> 2, 1+2*3 -> 7, etc.
    // Use try_eval_const_int to recursively evaluate constant expressions
    if matches!(op_kind, BinaryOperator::OpAdd | BinaryOperator::OpSub | BinaryOperator::OpMul | 
                BinaryOperator::OpDiv | BinaryOperator::OpIDiv | BinaryOperator::OpMod | 
                BinaryOperator::OpPow | BinaryOperator::OpBAnd | BinaryOperator::OpBOr | 
                BinaryOperator::OpBXor | BinaryOperator::OpShl | BinaryOperator::OpShr) {
        if let (Some(left_int), Some(right_int)) = (try_eval_const_int(&left), try_eval_const_int(&right)) {
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
                        emit(c, Instruction::encode_abx(OpCode::LoadK, result_reg, const_idx as u32));
                    }
                } else {
                    // Float result - try LOADF first, then LOADK
                    if emit_loadf(c, result_reg, result).is_none() {
                        let lua_val = LuaValue::number(result);
                        let const_idx = add_constant(c, lua_val);
                        emit(c, Instruction::encode_abx(OpCode::LoadK, result_reg, const_idx as u32));
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
            (left_lit.get_literal(), right_lit.get_literal()) {
            
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
                BinaryOperator::OpBAnd | BinaryOperator::OpBOr | BinaryOperator::OpBXor |
                BinaryOperator::OpShl | BinaryOperator::OpShr => {
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
                        emit(c, Instruction::encode_abx(OpCode::LoadK, result_reg, const_idx as u32));
                    }
                } else {
                    // Float result - use LOADK
                    let lua_val = LuaValue::number(result);
                    let const_idx = add_constant(c, lua_val);
                    emit(c, Instruction::encode_abx(OpCode::LoadK, result_reg, const_idx as u32));
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
                    // Encode immediate value (9 bits)
                    let imm = if int_val < 0 {
                        (int_val + 512) as u32
                    } else {
                        int_val as u32
                    };
                    
                    // Try immediate arithmetic instructions
                    // Only compile left operand if we actually use immediate instruction
                    match op_kind {
                        BinaryOperator::OpAdd => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            emit(c, Instruction::encode_abc(OpCode::AddI, result_reg, left_reg, imm));
                            // Emit MMBINI for metamethod call (TM_ADD = 6)
                            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, imm, 6, false));
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpSub => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            // Lua 5.4: Use SubK for constant operand
                            emit(c, Instruction::create_abck(OpCode::SubK, result_reg, left_reg, imm, true));
                            // Emit MMBINK for metamethod call (TM_SUB = 7)
                            emit(c, Instruction::create_abck(OpCode::MmBinK, left_reg, imm, 7, true));
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpMul => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            // Lua 5.4: Use MulK for constant operand
                            emit(c, Instruction::create_abck(OpCode::MulK, result_reg, left_reg, imm, true));
                            // Emit MMBINK for metamethod call (TM_MUL = 8)
                            emit(c, Instruction::create_abck(OpCode::MmBinK, left_reg, imm, 8, true));
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpMod => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            // Lua 5.4: Use ModK for constant operand
                            emit(c, Instruction::create_abck(OpCode::ModK, result_reg, left_reg, imm, true));
                            // Emit MMBINK for metamethod call (TM_MOD = 9)
                            emit(c, Instruction::create_abck(OpCode::MmBinK, left_reg, imm, 9, true));
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpPow => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            // Lua 5.4: Use PowK for constant operand
                            emit(c, Instruction::create_abck(OpCode::PowK, result_reg, left_reg, imm, true));
                            // Emit MMBINK for metamethod call (TM_POW = 10)
                            emit(c, Instruction::create_abck(OpCode::MmBinK, left_reg, imm, 10, true));
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpDiv => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            // Lua 5.4: Use DivK for constant operand
                            emit(c, Instruction::create_abck(OpCode::DivK, result_reg, left_reg, imm, true));
                            // Emit MMBINK for metamethod call (TM_DIV = 11)
                            emit(c, Instruction::create_abck(OpCode::MmBinK, left_reg, imm, 11, true));
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpIDiv => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            // Lua 5.4: Use IDivK for constant operand
                            emit(c, Instruction::create_abck(OpCode::IDivK, result_reg, left_reg, imm, true));
                            // Emit MMBINK for metamethod call (TM_IDIV = 12)
                            emit(c, Instruction::create_abck(OpCode::MmBinK, left_reg, imm, 12, true));
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpShr => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            // Lua 5.4: Use ShrI for immediate right shift
                            emit(c, Instruction::encode_abc(OpCode::ShrI, result_reg, left_reg, imm));
                            // Emit MMBINI for metamethod call (TM_SHR = 17)
                            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, imm, 17, false));
                            return Ok(result_reg);
                        }
                        BinaryOperator::OpShl => {
                            let left_reg = compile_expr(c, &left)?;
                            let result_reg = dest.unwrap_or_else(|| alloc_register(c));
                            // Lua 5.4: Use ShlI for immediate left shift
                            // Note: ShlI uses negated immediate: sC << R[B] where sC is the immediate
                            // To shift left by N, we use -N as the immediate
                            let neg_imm = if int_val < 0 {
                                ((-int_val) + 256) as u32
                            } else {
                                ((-int_val) + 512) as u32 % 512
                            };
                            emit(c, Instruction::encode_abc(OpCode::ShlI, result_reg, left_reg, neg_imm));
                            // Emit MMBINI for metamethod call (TM_SHL = 16)
                            emit(c, Instruction::create_abck(OpCode::MmBinI, left_reg, imm, 16, false));
                            return Ok(result_reg);
                        }
                        // Immediate comparison - NOT IMPLEMENTED YET
                        // (would need conditional skip logic in control flow statements)
                        _ => {}
                    }
                }
            }
        }
    }

    // Fall back to normal two-operand instruction
    let left_reg = compile_expr(c, &left)?;
    let right_reg = compile_expr(c, &right)?;
    let result_reg = dest.unwrap_or_else(|| alloc_register(c));

    // Determine opcode and metamethod event (TM)
    let (opcode, mm_event_opt) = match op_kind {
        BinaryOperator::OpAdd => (OpCode::Add, Some(6)),      // TM_ADD = 6
        BinaryOperator::OpSub => (OpCode::Sub, Some(7)),      // TM_SUB = 7
        BinaryOperator::OpMul => (OpCode::Mul, Some(8)),      // TM_MUL = 8
        BinaryOperator::OpMod => (OpCode::Mod, Some(9)),      // TM_MOD = 9
        BinaryOperator::OpPow => (OpCode::Pow, Some(10)),     // TM_POW = 10
        BinaryOperator::OpDiv => (OpCode::Div, Some(11)),     // TM_DIV = 11
        BinaryOperator::OpIDiv => (OpCode::IDiv, Some(12)),   // TM_IDIV = 12
        BinaryOperator::OpBAnd => (OpCode::BAnd, Some(13)),   // TM_BAND = 13
        BinaryOperator::OpBOr => (OpCode::BOr, Some(14)),     // TM_BOR = 14
        BinaryOperator::OpBXor => (OpCode::BXor, Some(15)),   // TM_BXOR = 15
        BinaryOperator::OpShl => (OpCode::Shl, Some(16)),     // TM_SHL = 16
        BinaryOperator::OpShr => (OpCode::Shr, Some(17)),     // TM_SHR = 17
        BinaryOperator::OpConcat => (OpCode::Concat, None),   // No MMBIN for concat
        BinaryOperator::OpEq => (OpCode::Eq, None),
        BinaryOperator::OpLt => (OpCode::Lt, None),
        BinaryOperator::OpLe => (OpCode::Le, None),
        BinaryOperator::OpNe => (OpCode::Eq, None), // Lua 5.4: Use Eq with negated k
        BinaryOperator::OpGt => (OpCode::Lt, None), // Lua 5.4: Use Lt with swapped operands
        BinaryOperator::OpGe => (OpCode::Le, None), // Lua 5.4: Use Le with swapped operands
        BinaryOperator::OpAnd => (OpCode::TestSet, None), // Lua 5.4: Use TestSet for short-circuit
        BinaryOperator::OpOr => (OpCode::TestSet, None), // Lua 5.4: Use TestSet for short-circuit
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
            Instruction::create_abck(OpCode::MmBin, left_reg, right_reg, mm_event, false),
        );
    }
    
    Ok(result_reg)
}

fn compile_unary_expr_to(
    c: &mut Compiler,
    expr: &LuaUnaryExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    // Get operand from children
    let operand = expr.get_expr().ok_or("Unary expression missing operand")?;
    let operand_reg = compile_expr(c, &operand)?;
    let result_reg = dest.unwrap_or_else(|| alloc_register(c));

    // Get operator from text
    let op_token = expr.get_op_token().ok_or("error")?;
    let op_kind = op_token.get_op();
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
    // If dest is specified and this is a simple call (not "all out" mode),
    // we can use dest as the function register to avoid extra Move instructions
    compile_call_expr_with_returns_and_dest(c, expr, 1, dest)
}

/// Compile a call expression with specified number of expected return values and optional dest
fn compile_call_expr_with_returns_and_dest(
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
    
    // Handle method call with SELF instruction
    let func_reg = if is_method {
        if let LuaExpr::IndexExpr(index_expr) = &prefix_expr {
            // Method call: obj:method(args) → SELF instruction
            // SELF A B C: R(A+1) = R(B); R(A) = R(B)[C]
            // A = function register, A+1 = self parameter
            let func_reg = dest.unwrap_or_else(|| alloc_register(c));
            
            // Ensure func_reg+1 is allocated for self parameter
            while c.next_register <= func_reg + 1 {
                alloc_register(c);
            }
            
            // Compile object (table)
            let obj_expr = index_expr.get_prefix_expr()
                .ok_or("Method call missing object")?;
            let obj_reg = compile_expr(c, &obj_expr)?;
            
            // Get method name
            let method_name = if let Some(LuaIndexKey::Name(name_token)) = index_expr.get_index_key() {
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
                    true,  // k=1: C is constant index
                ),
            );
            
            func_reg
        } else {
            unreachable!("is_method but not IndexExpr")
        }
    } else {
        // Regular call: compile function expression
        // For now, don't try to optimize with dest - let compile_expr handle it naturally
        compile_expr(c, &prefix_expr)?
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
    
    for (i, arg_expr) in arg_exprs.iter().enumerate() {
        let is_last = i == arg_exprs.len() - 1;
        
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
                        
                        let obj_expr = index_expr.get_prefix_expr()
                            .ok_or("Method call missing object")?;
                        let obj_reg = compile_expr(c, &obj_expr)?;
                        
                        let method_name = if let Some(LuaIndexKey::Name(name_token)) = index_expr.get_index_key() {
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
                    compile_expr(c, &inner_prefix)?
                };
                
                // Compile call arguments
                let call_args_start = if inner_is_method { call_reg + 2 } else { call_reg + 1 };
                let call_arg_exprs = call_expr
                    .get_args_list()
                    .ok_or("missing args list")?
                    .get_args()
                    .collect::<Vec<_>>();
                
                let mut call_arg_regs = Vec::new();
                for call_arg in call_arg_exprs.iter() {
                    let arg_reg = compile_expr(c, call_arg)?;
                    call_arg_regs.push(arg_reg);
                }
                
                // Move call arguments if needed
                for (j, &reg) in call_arg_regs.iter().enumerate() {
                    let target = call_args_start + j as u32;
                    if reg != target {
                        while c.next_register <= target {
                            alloc_register(c);
                        }
                        emit_move(c, target, reg);
                    }
                }
                
                // Emit call with "all out" (C=0)
                let inner_arg_count = call_arg_exprs.len();
                let inner_b_param = if inner_is_method {
                    (inner_arg_count + 2) as u32  // +1 for self, +1 for Lua convention
                } else {
                    (inner_arg_count + 1) as u32
                };
                emit(
                    c,
                    Instruction::encode_abc(
                        OpCode::Call,
                        call_reg,
                        inner_b_param,
                        0  // C=0: all out
                    ),
                );
                
                arg_regs.push(call_reg);
                last_arg_is_call_all_out = true;
                break;
            }
        }
        
        let arg_reg = compile_expr(c, arg_expr)?;
        arg_regs.push(arg_reg);
    }
    
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
    if need_move {
        // Reserve registers for arguments
        while c.next_register < args_start + arg_regs.len() as u32 {
            alloc_register(c);
        }
        
        // Move arguments to correct positions
        for (i, &arg_reg) in arg_regs.iter().enumerate() {
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
        0  // B=0: all in
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

    // After CALL: adjust next_register based on return values
    // CALL places return values starting at func_reg
    // If num_returns == 0, CALL discards all returns, free register is func_reg
    // If num_returns > 0, return values are in func_reg .. func_reg + num_returns - 1
    // So next free register should be func_reg + num_returns
    c.next_register = func_reg + num_returns as u32;

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

    let result_reg = dest.unwrap_or_else(|| alloc_register(c));

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
                    Instruction::encode_abc(
                        OpCode::GetI,
                        result_reg,
                        table_reg,
                        int_value as u32,
                    ),
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
                    Instruction::create_abck(OpCode::GetField, result_reg, table_reg, const_idx, true),
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
                    Instruction::create_abck(OpCode::GetField, result_reg, table_reg, const_idx, true),
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
                let is_dots = matches!(&value_expr, LuaExpr::LiteralExpr(lit) 
                    if matches!(lit.get_literal(), Some(LuaLiteralToken::Dots(_))));
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
    emit(c, Instruction::encode_abc(OpCode::NewTable, reg, b_param, c_param));
    
    // EXTRAARG instruction (always 0 for now, used for extended parameters)
    emit(c, Instruction::create_ax(OpCode::ExtraArg, 0));

    if fields.is_empty() {
        return Ok(reg);
    }

    // Array part: consecutive value-only fields from the start
    let array_end = array_count;

    // Process array part with SETLIST optimization
    if array_end > 0 {
        const BATCH_SIZE: usize = 50; // Lua uses LFIELDS_PER_FLUSH = 50
        let mut batch_start = 0;
        
        while batch_start < array_end {
            let batch_end = (batch_start + BATCH_SIZE).min(array_end);
            let batch_count = batch_end - batch_start;
            
            // Compile all values in this batch to consecutive registers
            let values_start = reg + 1;
            
            for (i, field) in fields[batch_start..batch_end].iter().enumerate() {
                let target_reg = values_start + i as u32;
                
                // Ensure we have enough registers allocated
                while c.next_register <= target_reg {
                    alloc_register(c);
                }
                
                if let Some(value_expr) = field.get_value_expr() {
                    let value_reg = compile_expr_to(c, &value_expr, Some(target_reg))?;
                    if value_reg != target_reg {
                        emit_move(c, target_reg, value_reg);
                    }
                } else {
                    emit_load_nil(c, target_reg);
                }
            }
            
            // Emit SETLIST to set table[batch_start+1..batch_end] = R(reg+1)..R(reg+batch_count)
            // SETLIST A B C: for i=1,B do table[C*50+i] = R(A+i) end
            // A = table register
            // B = number of elements (0 means "up to top of stack")
            // C = batch number (0 for first 50, 1 for next 50, etc.)
            let c_param = (batch_start / BATCH_SIZE) as u32;
            emit(
                c,
                Instruction::encode_abc(OpCode::SetList, reg, batch_count as u32, c_param),
            );
            
            // Free temporary registers used for array values
            // After SETLIST, the values have been copied into the table,
            // so we can reuse these registers. Reset to reg+1 to match luac behavior.
            c.next_register = reg + 1;
            
            batch_start = batch_end;
        }
    }

    // Process remaining fields (hash part or special cases)
    for (field_idx, field) in fields.iter().enumerate().skip(array_end) {
        let is_last_field = field_idx == fields.len() - 1;

        if field.is_value_field() {
            // This must be ... or call as last element
            if let Some(value_expr) = field.get_value_expr() {
                let is_dots = matches!(&value_expr, LuaExpr::LiteralExpr(lit) 
                    if matches!(lit.get_literal(), Some(LuaLiteralToken::Dots(_))));
                let is_call = matches!(&value_expr, LuaExpr::CallExpr(_));

                if is_last_field && is_dots {
                    // VarArg expansion: {...} or {a, b, ...}
                    let vararg_start = reg + 1;
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::Vararg, vararg_start, 0, 0),
                    );
                    
                    // SetList with B=0 (all remaining values), C=array_end/50
                    let c_param = (array_end / 50) as u32;
                    emit(
                        c,
                        Instruction::encode_abc(OpCode::SetList, reg, 0, c_param),
                    );
                } else if is_last_field && is_call {
                    // Call as last element: returns multiple values
                    if let LuaExpr::CallExpr(_call_expr) = &value_expr {
                        // Compile call to return all values starting at reg+1
                        // For now, use simplified approach
                        let value_reg = compile_expr(c, &value_expr)?;
                        
                        // Single value case - fall back to SetTable
                        let array_index = (array_end + 1) as i64;
                        let key_const = add_constant(c, LuaValue::integer(array_index));
                        let key_reg = alloc_register(c);
                        emit_load_constant(c, key_reg, key_const);
                        emit(
                            c,
                            Instruction::encode_abc(OpCode::SetTable, reg, key_reg, value_reg),
                        );
                    }
                }
            }
        } else {
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

                    // Use SetField: R(A)[K(B)] := RK(C)
                    // k=1 means C is constant index, k=0 means C is register
                    emit(
                        c,
                        Instruction::create_abck(OpCode::SetField, reg, const_idx, value_operand, use_constant),
                    );
                    
                    continue; // Skip the SetTable at the end
                }
                LuaIndexKey::String(string_token) => {
                    // key is a string literal - use SetField optimization  
                    let string_value = string_token.get_value();
                    let lua_str = create_string_value(c, &string_value);
                    let const_idx = add_constant_dedup(c, lua_str);
                    
                    // Try to compile value as constant first (for RK optimization)
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

                    // Use SetField: R(A)[K(B)] := RK(C)
                    // k=1 means C is constant index, k=0 means C is register
                    emit(
                        c,
                        Instruction::create_abck(OpCode::SetField, reg, const_idx, value_operand, use_constant),
                    );
                    
                    continue; // Skip the SetTable at the end
                }
                LuaIndexKey::Integer(number_token) => {
                    // key is a numeric literal - try SETI optimization
                    if !number_token.is_float() {
                        let int_value = number_token.get_int_value();
                        // SETI can handle integer keys directly (8-bit unsigned index)
                        if int_value >= 0 && int_value <= 255 {
                            // Try to compile value as constant first (for RK optimization)
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

                            // Use SETI: R(A)[B] := RK(C) where B is integer index (0-255)
                            emit(
                                c,
                                Instruction::create_abck(OpCode::SetI, reg, int_value as u32, value_operand, use_constant),
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
                        if int_val >= 0 && int_val <= 255 {
                            // Use SETI for small integer keys
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

                            emit(
                                c,
                                Instruction::create_abck(OpCode::SetI, reg, int_val as u32, value_operand, use_constant),
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
                Instruction::create_abck(OpCode::SetTable, reg, key_reg, value_operand, use_constant),
            );
        }
    }

    // Free temporary registers used during table construction
    // Reset to table_reg + 1 to match luac's register allocation behavior
    c.next_register = reg + 1;

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
                    // Optimized: table[integer] = value -> SetTableI
                    // B field is 9 bits, so max value is 511
                    let int_value = number_token.get_int_value();
                    if int_value >= 0 && int_value <= 511 {
                        // Use SetTableI: R(A)[B] := R(C)
                        emit(
                            c,
                            Instruction::encode_abc(
                                OpCode::SetI,
                                table_reg,
                                int_value as u32,
                                value_reg,
                            ),
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
                                false,  // k=0: C is register
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
                    // Optimized: table["string"] = value -> SetTableK
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
                                true,
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
            });
        func_compiler.chunk.locals.push(param_name);
        regular_param_count += 1;
    }

    func_compiler.chunk.param_count = regular_param_count + param_offset;
    func_compiler.chunk.is_vararg = has_vararg;
    func_compiler.next_register = (regular_param_count + param_offset) as u32;

    // Compile function body (skip if empty)
    if has_body {
        let body = closure.get_block().unwrap();
        compile_block(&mut func_compiler, &body)?;
    }

    // Add implicit return if needed
    if func_compiler.chunk.code.is_empty()
        || Instruction::get_opcode(*func_compiler.chunk.code.last().unwrap()) != OpCode::Return
    {
        let ret_instr = Instruction::encode_abc(OpCode::Return, 0, 1, 0);
        func_compiler.chunk.code.push(ret_instr);
    }

    func_compiler.chunk.max_stack_size = func_compiler.next_register as usize;

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
        let r = c.next_register;
        c.next_register += 1;
        r
    });

    // Ensure max_stack_size accounts for this register
    if (dest_reg + 1) as usize > c.chunk.max_stack_size {
        c.chunk.max_stack_size = (dest_reg + 1) as usize;
    }

    let closure_instr = Instruction::encode_abx(OpCode::Closure, dest_reg, chunk_index as u32);
    c.chunk.code.push(closure_instr);

    Ok(dest_reg)
}
