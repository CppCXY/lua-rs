// Expression compilation

use crate::opcode::{OpCode, Instruction};
use crate::value::{LuaValue, LuaString};
use emmylua_parser::{LuaSyntaxNode, LuaSyntaxKind};
use super::Compiler;
use super::helpers::*;

/// Compile any expression and return the register containing the result
pub fn compile_expr(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<u32, String> {
    match node.kind().to_syntax() {
        LuaSyntaxKind::LiteralExpr => compile_literal_expr(c, node),
        LuaSyntaxKind::NameExpr => compile_name_expr(c, node),
        LuaSyntaxKind::BinaryExpr => compile_binary_expr(c, node),
        LuaSyntaxKind::UnaryExpr => compile_unary_expr(c, node),
        LuaSyntaxKind::ParenExpr => compile_paren_expr(c, node),
        LuaSyntaxKind::CallExpr => compile_call_expr(c, node),
        LuaSyntaxKind::IndexExpr => compile_index_expr(c, node),
        LuaSyntaxKind::TableArrayExpr | LuaSyntaxKind::TableObjectExpr | LuaSyntaxKind::TableEmptyExpr => compile_table_expr(c, node),
        _ => {
            // Default: load nil
            let reg = alloc_register(c);
            emit_load_nil(c, reg);
            Ok(reg)
        }
    }
}

/// Compile literal expression (number, string, true, false, nil)
fn compile_literal_expr(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<u32, String> {
    let reg = alloc_register(c);
    
    // Get the text of the literal
    let text = node.text().to_string().trim().to_string();
    
    // Try to parse as number
    if let Ok(num) = text.parse::<f64>() {
        let const_idx = add_constant(c, LuaValue::number(num));
        emit_load_constant(c, reg, const_idx);
        return Ok(reg);
    }
    
    // Check for string (starts with quote)
    if text.starts_with('"') || text.starts_with('\'') || text.starts_with('[') {
        let s = text.trim_matches('"').trim_matches('\'').to_string();
        let const_idx = add_constant(c, LuaValue::string(LuaString::new(s)));
        emit_load_constant(c, reg, const_idx);
        return Ok(reg);
    }
    
    // Check for keywords
    match text.as_str() {
        "true" => emit_load_bool(c, reg, true),
        "false" => emit_load_bool(c, reg, false),
        "nil" => emit_load_nil(c, reg),
        _ => emit_load_nil(c, reg),
    }
    
    Ok(reg)
}

/// Compile name (identifier) expression
fn compile_name_expr(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<u32, String> {
    // Get the identifier name
    let name = node.text().to_string();
    
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
fn compile_binary_expr(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<u32, String> {
    let children: Vec<_> = node.children().collect();
    if children.len() < 2 {
        return Err("Binary expression requires two operands".to_string());
    }
    
    // Compile left and right operands
    let left_reg = compile_expr(c, &children[0])?;
    let right_reg = compile_expr(c, &children[1])?;
    let result_reg = alloc_register(c);
    
    // Find the operator
    let opcode = find_binary_operator(node)?;
    
    emit(c, Instruction::encode_abc(opcode, result_reg, left_reg, right_reg));
    
    Ok(result_reg)
}

/// Find binary operator from node text
fn find_binary_operator(node: &LuaSyntaxNode) -> Result<OpCode, String> {
    let text = node.text().to_string();
    
    if text.contains('+') {
        Ok(OpCode::Add)
    } else if text.contains('-') && !text.starts_with('-') {
        Ok(OpCode::Sub)
    } else if text.contains('*') {
        Ok(OpCode::Mul)
    } else if text.contains('/') && !text.contains("//") {
        Ok(OpCode::Div)
    } else if text.contains('%') {
        Ok(OpCode::Mod)
    } else if text.contains('^') {
        Ok(OpCode::Pow)
    } else if text.contains("==") {
        Ok(OpCode::Eq)
    } else if text.contains('<') && !text.contains("<=") {
        Ok(OpCode::Lt)
    } else if text.contains("<=") {
        Ok(OpCode::Le)
    } else if text.contains("..") {
        Ok(OpCode::Concat)
    } else {
        Err("Unknown binary operator".to_string())
    }
}

/// Compile unary expression (-a, not a, #a)
fn compile_unary_expr(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<u32, String> {
    let children: Vec<_> = node.children().collect();
    if children.is_empty() {
        return Err("Unary expression requires an operand".to_string());
    }
    
    let operand_reg = compile_expr(c, &children[0])?;
    let result_reg = alloc_register(c);
    
    // Find the operator
    let opcode = find_unary_operator(node)?;
    
    emit(c, Instruction::encode_abc(opcode, result_reg, operand_reg, 0));
    
    Ok(result_reg)
}

/// Find unary operator from node text
fn find_unary_operator(node: &LuaSyntaxNode) -> Result<OpCode, String> {
    let text = node.text().to_string();
    
    if text.trim().starts_with('-') {
        Ok(OpCode::Unm)
    } else if text.contains("not") {
        Ok(OpCode::Not)
    } else if text.contains('#') {
        Ok(OpCode::Len)
    } else {
        Err("Unknown unary operator".to_string())
    }
}

/// Compile parenthesized expression
fn compile_paren_expr(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<u32, String> {
    // Just compile the inner expression
    for child in node.children() {
        let kind = child.kind().to_syntax();
        if matches!(kind, 
            LuaSyntaxKind::LiteralExpr | LuaSyntaxKind::NameExpr | 
            LuaSyntaxKind::BinaryExpr | LuaSyntaxKind::UnaryExpr | 
            LuaSyntaxKind::CallExpr | LuaSyntaxKind::IndexExpr
        ) {
            return compile_expr(c, &child);
        }
    }
    
    let reg = alloc_register(c);
    emit_load_nil(c, reg);
    Ok(reg)
}

/// Compile function call expression
pub fn compile_call_expr(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<u32, String> {
    let children: Vec<_> = node.children().collect();
    if children.is_empty() {
        return Err("Call expression requires a function".to_string());
    }
    
    // Compile the function expression
    let func_reg = compile_expr(c, &children[0])?;
    
    // Compile arguments
    let mut arg_count = 0;
    for i in 1..children.len() {
        compile_expr(c, &children[i])?;
        arg_count += 1;
    }
    
    // Emit call instruction: Call(A, B, C)
    // R(A)(R(A+1), ..., R(A+B-1))
    // Results in R(A), ..., R(A+C-2)
    emit(c, Instruction::encode_abc(OpCode::Call, func_reg, arg_count + 1, 2));
    
    Ok(func_reg)
}

/// Compile index expression (table[key])
fn compile_index_expr(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<u32, String> {
    let children: Vec<_> = node.children().collect();
    if children.len() < 2 {
        return Err("Index expression requires table and key".to_string());
    }
    
    let table_reg = compile_expr(c, &children[0])?;
    let key_reg = compile_expr(c, &children[1])?;
    let result_reg = alloc_register(c);
    
    emit(c, Instruction::encode_abc(OpCode::GetTable, result_reg, table_reg, key_reg));
    
    Ok(result_reg)
}

/// Compile table constructor expression
fn compile_table_expr(c: &mut Compiler, _node: &LuaSyntaxNode) -> Result<u32, String> {
    let reg = alloc_register(c);
    
    // Create empty table
    emit(c, Instruction::encode_abc(OpCode::NewTable, reg, 0, 0));
    
    // TODO: Compile table fields
    // For now, just return empty table
    
    Ok(reg)
}

/// Compile a variable expression for assignment
pub fn compile_var_expr(c: &mut Compiler, node: &LuaSyntaxNode, value_reg: u32) -> Result<(), String> {
    match node.kind().to_syntax() {
        LuaSyntaxKind::NameExpr => {
            // Get the identifier name
            let name = node.text().to_string();
            
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
        LuaSyntaxKind::IndexExpr => {
            let children: Vec<_> = node.children().collect();
            if children.len() < 2 {
                return Err("Index expression requires table and key".to_string());
            }
            
            let table_reg = compile_expr(c, &children[0])?;
            let key_reg = compile_expr(c, &children[1])?;
            
            emit(c, Instruction::encode_abc(OpCode::SetTable, table_reg, key_reg, value_reg));
            Ok(())
        }
        _ => Err(format!("Invalid variable expression: {:?}", node.kind()))
    }
}
