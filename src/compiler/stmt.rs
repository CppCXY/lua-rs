// Statement compilation

use emmylua_parser::{LuaSyntaxNode, LuaSyntaxKind};
use crate::opcode::{OpCode, Instruction};
use super::{Compiler, helpers::*};
use super::expr::{compile_expr, compile_var_expr, compile_call_expr};

/// Compile any statement
pub fn compile_stat(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<(), String> {
    match node.kind().to_syntax() {
        LuaSyntaxKind::LocalStat => compile_local_stat(c, node),
        LuaSyntaxKind::AssignStat => compile_assign_stat(c, node),
        LuaSyntaxKind::CallExprStat => compile_call_stat(c, node),
        LuaSyntaxKind::ReturnStat => compile_return_stat(c, node),
        LuaSyntaxKind::IfStat => compile_if_stat(c, node),
        LuaSyntaxKind::WhileStat => compile_while_stat(c, node),
        LuaSyntaxKind::RepeatStat => compile_repeat_stat(c, node),
        LuaSyntaxKind::ForStat => compile_for_stat(c, node),
        LuaSyntaxKind::ForRangeStat => compile_for_range_stat(c, node),
        LuaSyntaxKind::DoStat => compile_do_stat(c, node),
        LuaSyntaxKind::BreakStat => compile_break_stat(c),
        LuaSyntaxKind::EmptyStat => Ok(()),
        _ => Ok(()), // Other statements not implemented yet
    }
}

/// Compile local variable declaration
fn compile_local_stat(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<(), String> {
    let mut names = Vec::new();
    let mut exprs = Vec::new();
    
    // Find names and expressions
    for child in node.children() {
        match child.kind().to_syntax() {
            LuaSyntaxKind::LocalName => {
                // Get name text from node
                let name = child.text().to_string();
                names.push(name);
            }
            // Expression nodes
            LuaSyntaxKind::LiteralExpr | LuaSyntaxKind::NameExpr | 
            LuaSyntaxKind::BinaryExpr | LuaSyntaxKind::UnaryExpr | 
            LuaSyntaxKind::CallExpr | LuaSyntaxKind::IndexExpr |
            LuaSyntaxKind::ParenExpr => {
                exprs.push(child);
            }
            _ => {}
        }
    }
    
    // Compile init expressions
    let mut regs = Vec::new();
    for expr in exprs {
        let reg = compile_expr(c, &expr)?;
        regs.push(reg);
    }
    
    // Fill missing values with nil
    while regs.len() < names.len() {
        let reg = alloc_register(c);
        emit_load_nil(c, reg);
        regs.push(reg);
    }
    
    // Define locals
    for (i, name) in names.iter().enumerate() {
        add_local(c, name.clone(), regs[i]);
    }
    
    Ok(())
}

/// Compile assignment statement
fn compile_assign_stat(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<(), String> {
    let mut vars = Vec::new();
    let mut exprs = Vec::new();
    let mut found_equals = false;
    
    // Parse vars and expressions - look for '=' in the text
    let text = node.text().to_string();
    let has_equals = text.contains('=');
    
    for child in node.children() {
        let kind = child.kind().to_syntax();
        match kind {
            LuaSyntaxKind::NameExpr | LuaSyntaxKind::IndexExpr => {
                if found_equals || (has_equals && !vars.is_empty()) {
                    exprs.push(child);
                } else {
                    vars.push(child);
                }
            }
            LuaSyntaxKind::LiteralExpr | LuaSyntaxKind::BinaryExpr | 
            LuaSyntaxKind::UnaryExpr | LuaSyntaxKind::CallExpr |
            LuaSyntaxKind::ParenExpr => {
                found_equals = true;
                exprs.push(child);
            }
            _ => {}
        }
    }
    
    // If we still haven't separated them, assume first child is var, rest are exprs
    if vars.is_empty() && !exprs.is_empty() {
        return Err("No variables found in assignment".to_string());
    }
    
    // Compile expressions
    let mut val_regs = Vec::new();
    for expr in exprs {
        let reg = compile_expr(c, &expr)?;
        val_regs.push(reg);
    }
    
    // Fill missing values with nil
    while val_regs.len() < vars.len() {
        let reg = alloc_register(c);
        emit_load_nil(c, reg);
        val_regs.push(reg);
    }
    
    // Compile assignments
    for (i, var) in vars.iter().enumerate() {
        compile_var_expr(c, var, val_regs[i])?;
    }
    
    Ok(())
}

/// Compile function call statement
fn compile_call_stat(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<(), String> {
    // Find the call expression child
    for child in node.children() {
        if matches!(child.kind().to_syntax(), LuaSyntaxKind::CallExpr) {
            compile_call_expr(c, &child)?;
            return Ok(());
        }
    }
    Ok(())
}

/// Compile return statement
fn compile_return_stat(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<(), String> {
    let mut exprs = Vec::new();
    
    // Find return expressions
    for child in node.children() {
        match child.kind().to_syntax() {
            LuaSyntaxKind::LiteralExpr | LuaSyntaxKind::NameExpr | 
            LuaSyntaxKind::BinaryExpr | LuaSyntaxKind::UnaryExpr | 
            LuaSyntaxKind::CallExpr | LuaSyntaxKind::IndexExpr |
            LuaSyntaxKind::ParenExpr => {
                exprs.push(child);
            }
            _ => {}
        }
    }
    
    if exprs.is_empty() {
        // return (no values)
        emit(c, Instruction::encode_abc(OpCode::Return, 0, 1, 0));
    } else {
        // Compile first expression
        let first_reg = compile_expr(c, &exprs[0])?;
        
        // For simplicity, only return first value for now
        emit(c, Instruction::encode_abc(OpCode::Return, first_reg, 2, 0));
    }
    
    Ok(())
}

/// Compile if statement
fn compile_if_stat(_c: &mut Compiler, _node: &LuaSyntaxNode) -> Result<(), String> {
    // TODO: implement if statement properly
    Ok(())
}

/// Compile while loop
fn compile_while_stat(_c: &mut Compiler, _node: &LuaSyntaxNode) -> Result<(), String> {
    // TODO: implement while loop properly
    Ok(())
}

/// Compile repeat-until loop
fn compile_repeat_stat(_c: &mut Compiler, _node: &LuaSyntaxNode) -> Result<(), String> {
    // TODO: implement repeat-until loop properly
    Ok(())
}

/// Compile numeric for loop
fn compile_for_stat(_c: &mut Compiler, _node: &LuaSyntaxNode) -> Result<(), String> {
    // TODO: implement for loop properly
    Ok(())
}

/// Compile generic for loop
fn compile_for_range_stat(_c: &mut Compiler, _node: &LuaSyntaxNode) -> Result<(), String> {
    // TODO: implement for-in loop properly
    Ok(())
}

/// Compile do-end block
fn compile_do_stat(c: &mut Compiler, node: &LuaSyntaxNode) -> Result<(), String> {
    use super::compile_block;
    
    begin_scope(c);
    
    // Find Block node
    for child in node.children() {
        if matches!(child.kind().to_syntax(), LuaSyntaxKind::Block) {
            compile_block(c, &child)?;
            break;
        }
    }
    
    end_scope(c);
    
    Ok(())
}

/// Compile break statement
fn compile_break_stat(_c: &mut Compiler) -> Result<(), String> {
    // TODO: implement break properly
    Ok(())
}
