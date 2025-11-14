// Expression compilation - Using strong-typed AST nodes

use super::Compiler;
use super::helpers::*;
use crate::compiler::compile_block;
use crate::lua_value::UpvalueDesc;
use crate::lua_value::{Chunk, LuaValue};
use crate::opcode::{Instruction, OpCode};
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
        LuaExpr::ClosureExpr(e) => compile_closure_expr_to(c, e, dest),
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
            // Try to get the string representation and parse it
            let num_value = if num.is_float() {
                LuaValue::float(num.get_float_value())
            } else {
                LuaValue::integer(num.get_int_value())
            };
            let const_idx = add_constant(c, num_value);
            emit_load_constant(c, reg, const_idx);
        }
        LuaLiteralToken::String(s) => {
            let lua_string = intern_string(c, s.get_value());
            let const_idx = add_constant(c, LuaValue::from_string_rc(lua_string));
            emit_load_constant(c, reg, const_idx);
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

fn compile_binary_expr_to(
    c: &mut Compiler,
    expr: &LuaBinaryExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    // Get left and right expressions from children
    let (left, right) = expr.get_exprs().ok_or("error")?;
    let op = expr.get_op_token().ok_or("error")?;
    let op_kind = op.get_op();

    let left_reg = compile_expr(c, &left)?;
    let right_reg = compile_expr(c, &right)?;
    let result_reg = dest.unwrap_or_else(|| alloc_register(c));

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
    compile_call_expr_with_returns(c, expr, 1)
}

fn compile_call_expr_to(
    c: &mut Compiler,
    expr: &LuaCallExpr,
    _dest: Option<u32>,
) -> Result<u32, String> {
    // Note: Call results are always placed in consecutive registers starting from function register
    // Cannot honor dest for calls, use compile_call_expr_with_returns instead
    compile_call_expr_with_returns(c, expr, 1)
}

/// Compile a call expression with specified number of expected return values
pub fn compile_call_expr_with_returns(
    c: &mut Compiler,
    expr: &LuaCallExpr,
    num_returns: usize,
) -> Result<u32, String> {
    // Get prefix (function) and arguments from children
    let prefix_expr = expr.get_prefix_expr().ok_or("missing prefix expr")?;
    let arg_exprs = expr
        .get_args_list()
        .ok_or("missing args list")?
        .get_args()
        .collect::<Vec<_>>();

    // Handle method call (colon syntax: obj:method(args))
    // For method calls, we compile obj once, then use GetTable to get the method
    let (func_src_reg, actual_args, self_reg_opt) = if expr.is_colon_call()
        && let LuaExpr::IndexExpr(prefix_index_expr) = prefix_expr
    {
        // For obj:method(args), we compile obj once
        let self_expr = prefix_index_expr
            .get_prefix_expr()
            .ok_or("missing self expr")?;
        let obj_reg = compile_expr(c, &self_expr)?;

        // Get the method key
        let key = prefix_index_expr
            .get_index_key()
            .ok_or("Index expression missing key")?;
        let key_reg = match key {
            LuaIndexKey::Name(name_token) => {
                let field_name = name_token.get_name_text().to_string();
                let lua_str = intern_string(c, field_name);
                let const_idx = add_constant(c, LuaValue::from_string_rc(lua_str));
                let key_reg = alloc_register(c);
                emit_load_constant(c, key_reg, const_idx);
                key_reg
            }
            LuaIndexKey::String(string_token) => {
                let string_value = string_token.get_value();
                let lua_str = intern_string(c, string_value);
                let const_idx = add_constant(c, LuaValue::from_string_rc(lua_str));
                let key_reg = alloc_register(c);
                emit_load_constant(c, key_reg, const_idx);
                key_reg
            }
            LuaIndexKey::Expr(key_expr) => compile_expr(c, &key_expr)?,
            _ => return Err("Unsupported method key type".to_string()),
        };

        // Get the method: func = obj[key]
        let func_reg = alloc_register(c);
        emit(
            c,
            Instruction::encode_abc(OpCode::GetTable, func_reg, obj_reg, key_reg),
        );

        // Return: (function_register, args, Some(self_register))
        // obj_reg is reused as self parameter
        (func_reg, arg_exprs, Some(obj_reg))
    } else {
        // Regular call: func(args)
        let func_reg = compile_expr(c, &prefix_expr)?;
        (func_reg, arg_exprs, None)
    };

    // Calculate total argument count (including self for method calls)
    let arg_count = if self_reg_opt.is_some() {
        actual_args.len() + 1 // +1 for self
    } else {
        actual_args.len()
    };

    // Allocate a new register for the call (to avoid overwriting source)
    let func_reg = alloc_register(c);

    // Copy function to call register if different
    if func_src_reg != func_reg {
        emit_move(c, func_reg, func_src_reg);
    }

    // Allocate space for arguments starting after the function register
    let args_start = func_reg + 1;

    // Reserve registers for all arguments
    while c.next_register < args_start + arg_count as u32 {
        alloc_register(c);
    }

    // For method calls, place self as the first argument
    let mut current_arg_pos = 0;
    if let Some(self_reg) = self_reg_opt {
        let target_reg = args_start;
        if self_reg != target_reg {
            emit_move(c, target_reg, self_reg);
        }
        current_arg_pos = 1;
    }

    // Compile remaining arguments into consecutive registers after self (if method call)
    for i in 0..actual_args.len() {
        let target_reg = args_start + current_arg_pos as u32;
        let arg_reg = compile_expr(c, &actual_args[i])?;

        // If expression compiled to different register, move it
        if arg_reg != target_reg {
            emit_move(c, target_reg, arg_reg);
        }
        current_arg_pos += 1;
    }

    // Emit call instruction
    // B = number of arguments + 1 (includes the function itself)
    // C = number of expected return values + 1 (0 means all returns, 1 means 0 returns, 2 means 1 return, etc.)
    let c_param = if num_returns == 0 {
        0
    } else {
        (num_returns + 1) as u32
    };
    emit(
        c,
        Instruction::encode_abc(OpCode::Call, func_reg, (arg_count + 1) as u32, c_param),
    );

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
            let int_value = number_token.get_int_value();
            if int_value >= 0 && int_value <= u32::MAX as i64 {
                // Use GetTableI: R(A) := R(B)[C]
                emit(
                    c,
                    Instruction::encode_abc(
                        OpCode::GetTableI,
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
            // Optimized: table.field -> GetTableK
            let field_name = name_token.get_name_text().to_string();
            let lua_str = intern_string(c, field_name);
            let const_idx = add_constant(c, LuaValue::from_string_rc(lua_str));
            // Use GetTableK: R(A) := R(B)[K(C)]
            // ABC format: A=dest, B=table, C=const_idx
            if const_idx <= 511 {
                // C field is 9 bits
                emit(
                    c,
                    Instruction::encode_abc(OpCode::GetTableK, result_reg, table_reg, const_idx),
                );
                return Ok(result_reg);
            }
            // Fallback for large const_idx
            let key_reg = alloc_register(c);
            emit_load_constant(c, key_reg, const_idx);
            emit(
                c,
                Instruction::encode_abc(OpCode::GetTable, result_reg, table_reg, key_reg),
            );
            Ok(result_reg)
        }
        LuaIndexKey::String(string_token) => {
            // Optimized: table["string"] -> GetTableK
            let string_value = string_token.get_value();
            let lua_str = intern_string(c, string_value);
            let const_idx = add_constant(c, LuaValue::from_string_rc(lua_str));
            if const_idx <= 511 {
                emit(
                    c,
                    Instruction::encode_abc(OpCode::GetTableK, result_reg, table_reg, const_idx),
                );
                return Ok(result_reg);
            }
            // Fallback
            let key_reg = alloc_register(c);
            emit_load_constant(c, key_reg, const_idx);
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

    // Create empty table
    emit(c, Instruction::encode_abc(OpCode::NewTable, reg, 0, 0));

    // Track array index for list-style entries
    let mut array_index = 1i64;

    // Compile table fields
    for field in expr.get_fields() {
        if field.is_value_field() {
            // Value only format: { 10, 20, 30 } - array part
            let value_reg = if let Some(value_expr) = field.get_value_expr() {
                compile_expr(c, &value_expr)?
            } else {
                let r = alloc_register(c);
                emit_load_nil(c, r);
                r
            };

            // Generate numeric key for array element
            let key_const = add_constant(c, LuaValue::integer(array_index));
            let key_reg = alloc_register(c);
            emit_load_constant(c, key_reg, key_const);

            // Set table[array_index] = value
            emit(
                c,
                Instruction::encode_abc(OpCode::SetTable, reg, key_reg, value_reg),
            );

            array_index += 1;
        } else {
            let Some(field_key) = field.get_field_key() else {
                continue;
            };

            let key_reg = match field_key {
                LuaIndexKey::Name(name_token) => {
                    // key is an identifier
                    let key_name = name_token.get_name_text().to_string();
                    let lua_str = intern_string(c, key_name);
                    let const_idx = add_constant(c, LuaValue::from_string_rc(lua_str));
                    let key_reg = alloc_register(c);
                    emit_load_constant(c, key_reg, const_idx);
                    key_reg
                }
                LuaIndexKey::String(string_token) => {
                    // key is a string literal
                    let string_value = string_token.get_value();
                    let lua_str = intern_string(c, string_value);
                    let const_idx = add_constant(c, LuaValue::from_string_rc(lua_str));
                    let key_reg = alloc_register(c);
                    emit_load_constant(c, key_reg, const_idx);
                    key_reg
                }
                LuaIndexKey::Integer(number_token) => {
                    // key is a numeric literal
                    let num_value = number_token.get_float_value();
                    let const_idx = add_constant(c, LuaValue::number(num_value));
                    let key_reg = alloc_register(c);
                    emit_load_constant(c, key_reg, const_idx);
                    key_reg
                }
                LuaIndexKey::Expr(key_expr) => {
                    // key is an expression
                    compile_expr(c, &key_expr)?
                }
                LuaIndexKey::Idx(_i) => {
                    return Err("Unsupported table field key type".to_string());
                }
            };

            // Compile value expression
            let value_reg = if let Some(value_expr) = field.get_value_expr() {
                compile_expr(c, &value_expr)?
            } else {
                let r = alloc_register(c);
                emit_load_nil(c, r);
                r
            };

            // Set table field: table[key] = value
            emit(
                c,
                Instruction::encode_abc(OpCode::SetTable, reg, key_reg, value_reg),
            );
        }
    }

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
                    let int_value = number_token.get_int_value();
                    if int_value >= 0 && int_value <= u32::MAX as i64 {
                        // Use SetTableI: R(A)[B] := R(C)
                        emit(
                            c,
                            Instruction::encode_abc(
                                OpCode::SetTableI,
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
                    // Optimized: table.field = value -> SetTableK
                    let field_name = name_token.get_name_text().to_string();
                    let lua_str = intern_string(c, field_name);
                    let const_idx = add_constant(c, LuaValue::from_string_rc(lua_str));
                    // Use SetTableK: R(A)[K(B)] := R(C)
                    // ABC format: A=table, B=const_idx, C=value
                    if const_idx <= 511 {
                        emit(
                            c,
                            Instruction::encode_abc(
                                OpCode::SetTableK,
                                table_reg,
                                const_idx,
                                value_reg,
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
                    let lua_str = intern_string(c, string_value);
                    let const_idx = add_constant(c, LuaValue::from_string_rc(lua_str));
                    if const_idx <= 511 {
                        emit(
                            c,
                            Instruction::encode_abc(
                                OpCode::SetTableK,
                                table_reg,
                                const_idx,
                                value_reg,
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

pub fn compile_closure_expr(c: &mut Compiler, closure: &LuaClosureExpr) -> Result<u32, String> {
    compile_closure_expr_to(c, closure, None)
}

pub fn compile_closure_expr_to(
    c: &mut Compiler,
    closure: &LuaClosureExpr,
    dest: Option<u32>,
) -> Result<u32, String> {
    let params_list = closure
        .get_params_list()
        .ok_or("closure missing params list")?;

    let params = params_list.get_params().collect::<Vec<_>>();

    let body = closure.get_block().ok_or("closure missing body")?;

    // Create a new compiler for the function body with parent scope chain
    // No need to sync anymore - scope_chain is already current
    let mut func_compiler = Compiler::new_with_parent(c.scope_chain.clone(), c.string_pool.clone());

    // Set up parameters as local variables
    for (i, param) in params.iter().enumerate() {
        // Try to get parameter name
        let param_name = if let Some(name_token) = param.get_name_token() {
            name_token.get_name_text().to_string()
        } else {
            format!("arg{}", i)
        };

        func_compiler
            .scope_chain
            .borrow_mut()
            .locals
            .push(super::Local {
                name: param_name.clone(),
                depth: 0,
                register: i as u32,
            });
        func_compiler.chunk.locals.push(param_name);
    }

    func_compiler.chunk.param_count = params.len();
    func_compiler.next_register = params.len() as u32;

    // Compile function body
    compile_block(&mut func_compiler, &body)?;

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

    let closure_instr = Instruction::encode_abx(OpCode::Closure, dest_reg, chunk_index as u32);
    c.chunk.code.push(closure_instr);

    Ok(dest_reg)
}
