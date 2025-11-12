// Statement compilation

use super::expr::{compile_call_expr, compile_expr, compile_var_expr};
use super::{Compiler, helpers::*};
use crate::opcode::{Instruction, OpCode};
use emmylua_parser::{LuaAssignStat, LuaCallExprStat, LuaLocalStat, LuaReturnStat, LuaStat};

/// Compile any statement
pub fn compile_stat(c: &mut Compiler, stat: &LuaStat) -> Result<(), String> {
    match stat {
        LuaStat::LocalStat(s) => compile_local_stat(c, s),
        LuaStat::AssignStat(s) => compile_assign_stat(c, s),
        LuaStat::CallExprStat(s) => compile_call_stat(c, s),
        LuaStat::ReturnStat(s) => compile_return_stat(c, s),
        LuaStat::IfStat(s) => compile_if_stat(c, s),
        LuaStat::WhileStat(s) => compile_while_stat(c, s),
        LuaStat::RepeatStat(s) => compile_repeat_stat(c, s),
        LuaStat::ForStat(s) => compile_for_stat(c, s),
        LuaStat::ForRangeStat(s) => compile_for_range_stat(c, s),
        LuaStat::DoStat(s) => compile_do_stat(c, s),
        LuaStat::BreakStat(_) => compile_break_stat(c),
        LuaStat::EmptyStat(_) => Ok(()),
        _ => Ok(()), // Other statements not yet implemented
    }
}

/// Compile local variable declaration
fn compile_local_stat(c: &mut Compiler, stat: &LuaLocalStat) -> Result<(), String> {
    let names: Vec<_> = stat.get_local_name_list().collect();
    let exprs: Vec<_> = stat.get_value_exprs().collect();

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
        // Get name text from LocalName node
        if let Some(name_token) = name.get_name_token() {
            let name_text = name_token.get_name_text().to_string();
            add_local(c, name_text, regs[i]);
        }
    }

    Ok(())
}

/// Compile assignment statement
fn compile_assign_stat(c: &mut Compiler, stat: &LuaAssignStat) -> Result<(), String> {
    // Get vars and expressions from children
    let (vars, exprs) = stat.get_var_and_expr_list();

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
fn compile_call_stat(c: &mut Compiler, stat: &LuaCallExprStat) -> Result<(), String> {
    // Get call expression from children
    let call_expr = stat
        .get_call_expr()
        .ok_or("Missing call expression in call statement")?;
    compile_call_expr(c, &call_expr)?;
    Ok(())
}

/// Compile return statement
fn compile_return_stat(c: &mut Compiler, stat: &LuaReturnStat) -> Result<(), String> {
    // Get expressions from children
    let exprs = stat.get_expr_list().collect::<Vec<_>>();

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
fn compile_if_stat(_c: &mut Compiler, _stat: &emmylua_parser::LuaIfStat) -> Result<(), String> {
    // TODO: implement if statement properly
    Ok(())
}

/// Compile while loop
fn compile_while_stat(
    _c: &mut Compiler,
    _stat: &emmylua_parser::LuaWhileStat,
) -> Result<(), String> {
    // TODO: implement while loop properly
    Ok(())
}

/// Compile repeat-until loop
fn compile_repeat_stat(
    _c: &mut Compiler,
    _stat: &emmylua_parser::LuaRepeatStat,
) -> Result<(), String> {
    // TODO: implement repeat-until loop properly
    Ok(())
}

/// Compile numeric for loop
fn compile_for_stat(_c: &mut Compiler, _stat: &emmylua_parser::LuaForStat) -> Result<(), String> {
    // TODO: implement for loop properly
    Ok(())
}

/// Compile generic for loop
fn compile_for_range_stat(
    _c: &mut Compiler,
    _stat: &emmylua_parser::LuaForRangeStat,
) -> Result<(), String> {
    // TODO: implement for-in loop properly
    Ok(())
}

/// Compile do-end block
fn compile_do_stat(c: &mut Compiler, stat: &emmylua_parser::LuaDoStat) -> Result<(), String> {
    use super::compile_block;

    begin_scope(c);

    if let Some(block) = stat.get_block() {
        compile_block(c, &block)?;
    }

    end_scope(c);

    Ok(())
}

/// Compile break statement
fn compile_break_stat(_c: &mut Compiler) -> Result<(), String> {
    // TODO: implement break properly
    Ok(())
}
