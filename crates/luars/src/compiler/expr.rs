use crate::compiler::parse_lua_number::NumberResult;

// Expression compilation (对齐lparser.c的expression parsing)
use super::expdesc::*;
use super::*;
use emmylua_parser::*;

/// Compile an expression (对齐expr)
pub(crate) fn expr(c: &mut Compiler, node: &LuaExpr) -> Result<ExpDesc, String> {
    subexpr(c, node, 0)
}

/// Compile a sub-expression with precedence (对齐subexpr)
pub(crate) fn subexpr(c: &mut Compiler, node: &LuaExpr, _limit: u32) -> Result<ExpDesc, String> {
    // For now, just handle simple expressions
    // TODO: Implement binary operators with precedence
    // TODO: Implement unary operators
    simple_exp(c, node)
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
        _ => {
            // TODO: Handle other expression types (variables, calls, tables, etc.)
            Ok(ExpDesc::new_nil())
        }
    }
}
