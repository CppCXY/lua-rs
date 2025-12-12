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
            let text = lit.get_text();
            
            // Check the literal type by text
            if text == "nil" {
                Ok(ExpDesc::new_nil())
            } else if text == "true" {
                Ok(ExpDesc::new_true())
            } else if text == "false" {
                Ok(ExpDesc::new_false())
            } else if text.starts_with('"') || text.starts_with('\'') {
                // String literal
                let s = text.trim_matches(|c| c == '"' || c == '\'');
                let k = helpers::string_k(c, s.to_string());
                Ok(ExpDesc::new_k(k))
            } else if let Ok(i) = text.parse::<i64>() {
                // Integer
                Ok(ExpDesc::new_int(i))
            } else if let Ok(f) = text.parse::<f64>() {
                // Float
                Ok(ExpDesc::new_float(f))
            } else {
                Ok(ExpDesc::new_nil())
            }
        }
        _ => {
            // TODO: Handle other expression types (variables, calls, tables, etc.)
            Ok(ExpDesc::new_nil())
        }
    }
}
