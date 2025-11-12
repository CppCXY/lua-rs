// Expression compilation - Using strong-typed AST nodes

use super::Compiler;
use super::helpers::*;
use crate::opcode::{Instruction, OpCode};
use crate::value::{LuaString, LuaValue};
use emmylua_parser::LuaIndexExpr;
use emmylua_parser::LuaIndexKey;
use emmylua_parser::LuaParenExpr;
use emmylua_parser::LuaTableExpr;
use emmylua_parser::UnaryOperator;
use emmylua_parser::{
    BinaryOperator, LuaAstNode, LuaBinaryExpr, LuaCallExpr, LuaExpr, LuaLiteralExpr,
    LuaLiteralToken, LuaNameExpr, LuaUnaryExpr, LuaVarExpr,
};

/// Compile any expression and return the register containing the result
pub fn compile_expr(c: &mut Compiler, expr: &LuaExpr) -> Result<u32, String> {
    match expr {
        LuaExpr::LiteralExpr(e) => compile_literal_expr(c, e),
        LuaExpr::NameExpr(e) => compile_name_expr(c, e),
        LuaExpr::BinaryExpr(e) => compile_binary_expr(c, e),
        LuaExpr::UnaryExpr(e) => compile_unary_expr(c, e),
        LuaExpr::ParenExpr(e) => compile_paren_expr(c, e),
        LuaExpr::CallExpr(e) => compile_call_expr(c, e),
        LuaExpr::IndexExpr(e) => compile_index_expr(c, e),
        LuaExpr::TableExpr(e) => compile_table_expr(c, e),
        LuaExpr::ClosureExpr(_) => {
            // TODO: implement closure compilation
            let reg = alloc_register(c);
            emit_load_nil(c, reg);
            Ok(reg)
        }
    }
}

/// Compile literal expression (number, string, true, false, nil)
fn compile_literal_expr(c: &mut Compiler, expr: &LuaLiteralExpr) -> Result<u32, String> {
    let reg = alloc_register(c);

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
            // Try to get the string representation and parse it
            let num_value = if num.is_float() {
                LuaValue::Float(num.get_float_value())
            } else {
                LuaValue::Integer(num.get_int_value())
            };
            let const_idx = add_constant(c, num_value);
            emit_load_constant(c, reg, const_idx);
        }
        LuaLiteralToken::String(s) => {
            let lua_string = LuaString::new(s.get_value());
            let const_idx = add_constant(c, LuaValue::string(lua_string));
            emit_load_constant(c, reg, const_idx);
        }
        _ => {}
    }

    Ok(reg)
}

/// Compile name (identifier) expression
fn compile_name_expr(c: &mut Compiler, expr: &LuaNameExpr) -> Result<u32, String> {
    // Get the identifier name
    let name = expr.get_name_text().unwrap_or("".to_string());

    // Check if it's a local variable
    if let Some(local) = resolve_local(c, &name) {
        return Ok(local.register);
    }

    // It's a global variable
    let reg = alloc_register(c);
    emit_get_global(c, &name, reg);
    Ok(reg)
}

/// Compile binary expression (a + b, a - b, etc.)
fn compile_binary_expr(c: &mut Compiler, expr: &LuaBinaryExpr) -> Result<u32, String> {
    // Get left and right expressions from children
    let (left, right) = expr.get_exprs().ok_or("error")?;
    let op = expr.get_op_token().ok_or("error")?;
    let op_kind = op.get_op();

    let left_reg = compile_expr(c, &left)?;
    let right_reg = compile_expr(c, &right)?;
    let result_reg = alloc_register(c);

    let opcode = match op_kind {
        BinaryOperator::OpAdd => OpCode::Add,
        BinaryOperator::OpSub => OpCode::Sub,
        BinaryOperator::OpMul => OpCode::Mul,
        BinaryOperator::OpDiv => OpCode::Div,
        BinaryOperator::OpMod => OpCode::Mod,
        BinaryOperator::OpPow => OpCode::Pow,
        BinaryOperator::OpConcat => OpCode::Concat,
        BinaryOperator::OpEq => OpCode::Eq,
        BinaryOperator::OpLt => OpCode::Lt,
        BinaryOperator::OpLe => OpCode::Le,
        BinaryOperator::OpNe => OpCode::Ne,
        BinaryOperator::OpGt => OpCode::Gt,
        BinaryOperator::OpGe => OpCode::Ge,
        BinaryOperator::OpAnd => OpCode::And,
        BinaryOperator::OpOr => OpCode::Or,
        BinaryOperator::OpBAnd => OpCode::BAnd,
        BinaryOperator::OpBOr => OpCode::BOr,
        BinaryOperator::OpBXor => OpCode::BXor,
        BinaryOperator::OpShl => OpCode::Shl,
        BinaryOperator::OpShr => OpCode::Shr,
        BinaryOperator::OpIDiv => OpCode::IDiv,
        _ => return Err(format!("Unsupported binary operator: {:?}", op_kind)),
    };

    emit(
        c,
        Instruction::encode_abc(opcode, result_reg, left_reg, right_reg),
    );
    Ok(result_reg)
}

/// Compile unary expression (-a, not a, #a)
fn compile_unary_expr(c: &mut Compiler, expr: &LuaUnaryExpr) -> Result<u32, String> {
    // Get operand from children
    let operand = expr.get_expr().ok_or("Unary expression missing operand")?;
    let operand_reg = compile_expr(c, &operand)?;
    let result_reg = alloc_register(c);

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
            emit_move(c, result_reg, operand_reg);
        }
    }

    Ok(result_reg)
}

/// Compile parenthesized expression
fn compile_paren_expr(c: &mut Compiler, expr: &LuaParenExpr) -> Result<u32, String> {
    // Get inner expression from children
    let inner_expr = expr.get_expr().ok_or("missing inner expr")?;
    let reg = compile_expr(c, &inner_expr)?;
    Ok(reg)
}

/// Compile function call expression
pub fn compile_call_expr(c: &mut Compiler, expr: &LuaCallExpr) -> Result<u32, String> {
    // Get prefix (function) and arguments from children
    let prefix_expr = expr.get_prefix_expr().ok_or("missing prefix expr")?;
    let arg_exprs = expr
        .get_args_list()
        .ok_or("missing args list")?
        .get_args()
        .collect::<Vec<_>>();

    let func_reg = compile_expr(c, &prefix_expr)?;
    let arg_count = arg_exprs.len();

    // Compile arguments
    for i in 0..arg_count {
        compile_expr(c, &arg_exprs[i])?;
    }

    // Emit call instruction
    emit(
        c,
        Instruction::encode_abc(OpCode::Call, func_reg, (arg_count + 1) as u32, 2),
    );

    Ok(func_reg)
}

/// Compile index expression (table[key] or table.field)
fn compile_index_expr(c: &mut Compiler, expr: &LuaIndexExpr) -> Result<u32, String> {
    // Get prefix (table) expression
    let prefix_expr = expr
        .get_prefix_expr()
        .ok_or("Index expression missing table")?;
    let table_reg = compile_expr(c, &prefix_expr)?;

    // Get index key
    let key = expr.get_index_key().ok_or("Index expression missing key")?;
    let key_reg = match key {
        LuaIndexKey::Expr(key_expr) => {
            // table[expr]
            compile_expr(c, &key_expr)?
        }
        LuaIndexKey::Name(name_token) => {
            // table.field
            let field_name = name_token.get_name_text().to_string();
            let const_idx = add_constant(c, LuaValue::string(LuaString::new(field_name)));
            let key_reg = alloc_register(c);
            emit_load_constant(c, key_reg, const_idx);
            key_reg
        }
        LuaIndexKey::String(string_token) => {
            // table["string"]
            let string_value = string_token.get_value();
            let const_idx = add_constant(c, LuaValue::string(LuaString::new(string_value)));
            let key_reg = alloc_register(c);
            emit_load_constant(c, key_reg, const_idx);
            key_reg
        }
        LuaIndexKey::Integer(number_token) => {
            // table[123]
            let num_value = number_token.get_float_value();
            let const_idx = add_constant(c, LuaValue::number(num_value));
            let key_reg = alloc_register(c);
            emit_load_constant(c, key_reg, const_idx);
            key_reg
        }
        LuaIndexKey::Idx(_i) => {
            // Fallback for other index types
            return Err("Unsupported index key type".to_string());
        }
    };

    let result_reg = alloc_register(c);
    emit(
        c,
        Instruction::encode_abc(OpCode::GetTable, result_reg, table_reg, key_reg),
    );

    Ok(result_reg)
}

/// Compile table constructor expression
fn compile_table_expr(c: &mut Compiler, _expr: &LuaTableExpr) -> Result<u32, String> {
    let reg = alloc_register(c);

    // Create empty table
    emit(c, Instruction::encode_abc(OpCode::NewTable, reg, 0, 0));

    // TODO: Compile table fields

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
            } else {
                // Set global
                emit_set_global(c, &name, value_reg);
            }
            Ok(())
        }
        LuaVarExpr::IndexExpr(index_expr) => {
            // Get table and key expressions from children
            let mut exprs = Vec::new();
            for child in index_expr.syntax().children() {
                if let Some(expr_node) = LuaExpr::cast(child.clone()) {
                    exprs.push(expr_node);
                }
            }

            if exprs.is_empty() {
                return Err("Index expression missing table in assignment".to_string());
            }

            let table_reg = compile_expr(c, &exprs[0])?;

            // Determine key
            let text = index_expr.syntax().text().to_string();
            let key_reg = if text.contains('.') && !text.contains('[') {
                // table.field
                let parts: Vec<&str> = text.split('.').collect();
                if parts.len() >= 2 {
                    let field_name = parts[1].trim();
                    let const_idx =
                        add_constant(c, LuaValue::string(LuaString::new(field_name.to_string())));
                    let key_reg = alloc_register(c);
                    emit_load_constant(c, key_reg, const_idx);
                    key_reg
                } else {
                    return Err("Invalid dot index in assignment".to_string());
                }
            } else {
                // table[expr]
                if exprs.len() >= 2 {
                    compile_expr(c, &exprs[1])?
                } else {
                    return Err("Bracket index missing key in assignment".to_string());
                }
            };

            emit(
                c,
                Instruction::encode_abc(OpCode::SetTable, table_reg, key_reg, value_reg),
            );
            Ok(())
        }
    }
}
