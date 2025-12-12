use crate::compiler::parse_lua_number::NumberResult;

// Expression compilation (对齐lparser.c的expression parsing)
use super::expdesc::*;
use super::*;
use emmylua_parser::*;

/// 编译表达式 (对齐 lparser.c 的 expr)
/// emmylua_parser 的 AST 已经处理了优先级，直接递归编译即可
pub(crate) fn expr(c: &mut Compiler, node: &LuaExpr) -> Result<ExpDesc, String> {
    match node {
        // 一元运算符
        LuaExpr::UnaryExpr(unary) => {
            let operand = unary.get_expr()
                .ok_or("unary expression missing operand")?;
            let op_token = unary.get_op_token()
                .ok_or("unary expression missing operator")?;
            
            // 递归编译操作数
            let mut v = expr(c, &operand)?;
            
            // 应用一元运算符
            apply_unary_op(c, &op_token, &mut v)?;
            Ok(v)
        }
        
        // 二元运算符
        LuaExpr::BinaryExpr(binary) => {
            let (left, right) = binary.get_exprs()
                .ok_or("binary expression missing operands")?;
            let op_token = binary.get_op_token()
                .ok_or("binary expression missing operator")?;
            
            // 递归编译左操作数
            let mut v1 = expr(c, &left)?;
            
            // 中缀处理
            infix_op(c, &op_token, &mut v1)?;
            
            // 递归编译右操作数
            let mut v2 = expr(c, &right)?;
            
            // 后缀处理
            postfix_op(c, &op_token, &mut v1, &mut v2)?;
            
            Ok(v1)
        }
        
        // 其他表达式
        _ => simple_exp(c, node)
    }
}

/// Compile a simple expression (对齐simpleexp)
pub(crate) fn simple_exp(c: &mut Compiler, node: &LuaExpr) -> Result<ExpDesc, String> {
    use super::helpers;

    match node {
        LuaExpr::LiteralExpr(lit) => {
            // Try to get the text and parse it
            match lit.get_literal().unwrap() {
                LuaLiteralToken::Bool(b) => {
                    if b.is_true() {
                        Ok(ExpDesc::new_true())
                    } else {
                        Ok(ExpDesc::new_false())
                    }
                }
                LuaLiteralToken::Nil(_) => Ok(ExpDesc::new_nil()),
                LuaLiteralToken::Number(n) => {
                    if n.is_int() {
                        match parse_lua_number::int_token_value(n.syntax()) {
                            Ok(NumberResult::Int(i)) => Ok(ExpDesc::new_int(i)),
                            Ok(NumberResult::Uint(u)) => {
                                if u <= i64::MAX as u64 {
                                    Ok(ExpDesc::new_int(u as i64))
                                } else {
                                    Err(format!(
                                        "The integer literal '{}' is too large to be represented as a signed integer",
                                        n.syntax().text()
                                    ))
                                }
                            }
                            Ok(NumberResult::Float(f)) => Ok(ExpDesc::new_float(f)),
                            Err(e) => Err(e),
                        }
                    } else {
                        Ok(ExpDesc::new_float(n.get_float_value()))
                    }
                }
                LuaLiteralToken::String(s) => {
                    let str_val = s.get_value();
                    let k = helpers::string_k(c, str_val.to_string());
                    Ok(ExpDesc::new_k(k))
                }
                LuaLiteralToken::Dots(_) => {
                    // Vararg expression (对齐lparser.c中的TK_DOTS处理)
                    // 检查当前函数是否为vararg
                    if !c.chunk.is_vararg {
                        return Err("cannot use '...' outside a vararg function".to_string());
                    }
                    // OP_VARARG A B : R[A], R[A+1], ..., R[A+B-2] = vararg
                    // B=1 表示返回所有可变参数
                    let pc = helpers::code_abc(c, crate::lua_vm::OpCode::Vararg, 0, 1, 0);
                    Ok(ExpDesc {
                        kind: ExpKind::VVararg,
                        info: pc as u32,
                        ival: 0,
                        nval: 0.0,
                        ind: expdesc::IndexInfo { t: 0, idx: 0 },
                        var: expdesc::VarInfo { ridx: 0, vidx: 0 },
                        t: -1,
                        f: -1,
                    })
                }
                _ => Err("Unsupported literal type".to_string()),
            }
        }
        LuaExpr::NameExpr(name) => {
            // Variable reference (对齐singlevar)
            let name_text = name.get_name_token()
                .ok_or("Name expression missing token")?
                .get_name_text()
                .to_string();
            
            let mut v = ExpDesc::new_void();
            super::var::singlevar(c, &name_text, &mut v)?;
            Ok(v)
        }
        LuaExpr::IndexExpr(index_expr) => {
            // Table indexing: t[k] or t.k (对齐suffixedexp中的索引部分)
            compile_index_expr(c, index_expr)
        }
        LuaExpr::ParenExpr(paren) => {
            // Parenthesized expression
            if let Some(inner) = paren.get_expr() {
                let mut v = expr(c, &inner)?;
                // Discharge to ensure value is computed
                super::exp2reg::discharge_vars(c, &mut v);
                Ok(v)
            } else {
                Ok(ExpDesc::new_nil())
            }
        }
        _ => {
            // TODO: Handle other expression types (calls, tables, binary ops, etc.)
            Err(format!("Unsupported expression type: {:?}", node))
        }
    }
}

/// Compile index expression: t[k] or t.field or t:method (对齐yindex和fieldsel)
pub(crate) fn compile_index_expr(c: &mut Compiler, index_expr: &LuaIndexExpr) -> Result<ExpDesc, String> {
    // Get the prefix expression (table)
    let prefix = index_expr.get_prefix_expr()
        .ok_or("Index expression missing prefix")?;
    
    let mut t = expr(c, &prefix)?;
    
    // Discharge table to register or upvalue
    super::exp2reg::exp2anyregup(c, &mut t);
    
    // Get the index/key
    if let Some(index_token) = index_expr.get_index_token() {
        // 冒号语法不应该出现在普通索引表达式中
        // 冒号只在函数调用（t:method()）和函数定义（function t:method()）中有意义
        if index_token.is_colon() {
            return Err("colon syntax ':' is only valid in function calls or definitions".to_string());
        }
        
        if index_token.is_dot() {
            // Dot notation: t.field
            // Get the field name as a string constant
            if let Some(key) = index_expr.get_index_key() {
                let key_name = match key {
                    LuaIndexKey::Name(name_token) => {
                        name_token.get_name_text().to_string()
                    }
                    _ => return Err("Dot notation requires name key".to_string()),
                };
                
                // Create string constant for field name
                let k_idx = helpers::string_k(c, key_name);
                let mut k = ExpDesc::new_k(k_idx);
                
                // Create indexed expression
                super::exp2reg::indexed(c, &mut t, &mut k);
                return Ok(t);
            }
        } else if index_token.is_left_bracket() {
            // Bracket notation: t[expr]
            if let Some(key) = index_expr.get_index_key() {
                let mut k = match key {
                    LuaIndexKey::Expr(key_expr) => {
                        expr(c, &key_expr)?
                    }
                    LuaIndexKey::Name(name_token) => {
                        // In bracket context, treat name as variable reference
                        let name_text = name_token.get_name_text().to_string();
                        let mut v = ExpDesc::new_void();
                        super::var::singlevar(c, &name_text, &mut v)?;
                        v
                    }
                    LuaIndexKey::String(str_token) => {
                        // String literal key
                        let str_val = str_token.get_value();
                        let k_idx = helpers::string_k(c, str_val.to_string());
                        ExpDesc::new_k(k_idx)
                    }
                    LuaIndexKey::Integer(int_token) => {
                        // Integer literal key
                        ExpDesc::new_int(int_token.get_int_value())
                    }
                    LuaIndexKey::Idx(_) => {
                        // Generic index (shouldn't normally happen in well-formed code)
                        return Err("Invalid index key type".to_string());
                    }
                };
                
                // Ensure key value is computed
                super::exp2reg::exp2val(c, &mut k);
                
                // Create indexed expression
                super::exp2reg::indexed(c, &mut t, &mut k);
                return Ok(t);
            }
        }
    }
    
    Err("Invalid index expression".to_string())
}

/// 应用一元运算符 (对齐 luaK_prefix)
fn apply_unary_op(c: &mut Compiler, op_token: &LuaUnaryOpToken, v: &mut ExpDesc) -> Result<(), String> {
    use crate::lua_vm::OpCode;
    use super::helpers;
    use emmylua_parser::UnaryOperator;
    
    let op = op_token.get_op();
    
    match op {
        UnaryOperator::OpUnm => {
            // 负号：尝试常量折叠
            if v.kind == ExpKind::VKInt {
                v.ival = v.ival.wrapping_neg();
            } else if v.kind == ExpKind::VKFlt {
                v.nval = -v.nval;
            } else {
                // 生成 UNM 指令
                super::exp2reg::discharge_2any_reg(c, v);
                super::exp2reg::free_exp(c, v);
                v.info = helpers::code_abc(c, OpCode::Unm, 0, v.info, 0) as u32;
                v.kind = ExpKind::VReloc;
            }
        }
        UnaryOperator::OpNot => {
            // 逻辑非：常量折叠或生成 NOT 指令
            if expdesc::is_const(v) {
                // 常量折叠
                let val = matches!(v.kind, ExpKind::VNil | ExpKind::VFalse);
                *v = if val { ExpDesc::new_true() } else { ExpDesc::new_false() };
            } else {
                super::exp2reg::discharge_2any_reg(c, v);
                super::exp2reg::free_exp(c, v);
                v.info = helpers::code_abc(c, OpCode::Not, 0, v.info, 0) as u32;
                v.kind = ExpKind::VReloc;
            }
        }
        UnaryOperator::OpLen => {
            // 长度运算符
            super::exp2reg::discharge_2any_reg(c, v);
            super::exp2reg::free_exp(c, v);
            v.info = helpers::code_abc(c, OpCode::Len, 0, v.info, 0) as u32;
            v.kind = ExpKind::VReloc;
        }
        UnaryOperator::OpBNot => {
            // 按位取反
            if v.kind == ExpKind::VKInt {
                v.ival = !v.ival;
            } else {
                super::exp2reg::discharge_2any_reg(c, v);
                super::exp2reg::free_exp(c, v);
                v.info = helpers::code_abc(c, OpCode::BNot, 0, v.info, 0) as u32;
                v.kind = ExpKind::VReloc;
            }
        }
        UnaryOperator::OpNop => {
            // 空操作，不应该出现
        }
    }
    
    Ok(())
}

/// 中缀处理 (对齐 luaK_infix)
fn infix_op(c: &mut Compiler, op_token: &LuaBinaryOpToken, v: &mut ExpDesc) -> Result<(), String> {
    let op = op_token.get_op();
    
    // TODO: 实现中缀运算符处理
    // 参考 lcode.c 的 luaK_infix 函数
    let _ = (c, op, v);
    Ok(())
}

/// 后缀处理 (对齐 luaK_posfix)
fn postfix_op(c: &mut Compiler, op_token: &LuaBinaryOpToken, v1: &mut ExpDesc, v2: &mut ExpDesc) -> Result<(), String> {
    let op = op_token.get_op();
    
    // TODO: 实现后缀运算符处理
    // 参考 lcode.c 的 luaK_posfix 函数
    let _ = (c, op, v1, v2);
    Ok(())
}
