// Statement compilation

use super::expr::{compile_call_expr, compile_expr, compile_var_expr};
use super::{Compiler, helpers::*};
use crate::opcode::{Instruction, OpCode};
use emmylua_parser::{LuaAssignStat, LuaCallExprStat, LuaDoStat, LuaForRangeStat, LuaForStat, LuaIfStat, LuaLocalStat, LuaRepeatStat, LuaReturnStat, LuaStat, LuaWhileStat};

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
fn compile_if_stat(c: &mut Compiler, stat: &LuaIfStat) -> Result<(), String> {
    // TODO: Implement if statement compilation
    // Structure: if <condition> then <block> [elseif <condition> then <block>]* [else <block>] end
    // 
    // Pseudo-implementation:
    // 1. Compile condition expression -> cond_reg
    // 2. Emit Test instruction: if cond_reg is false, jump to else/end
    // 3. Compile then block
    // 4. Emit Jump to end (skip else blocks)
    // 5. Patch jump target from step 2
    // 6. Repeat for each elseif clause
    // 7. Compile else block if exists
    // 8. Patch all end jumps
    //
    // Need to find correct API for:
    // - _stat.get_condition() or similar to get condition expr
    // - _stat.get_then_block() to get main block
    // - _stat.get_elseif_clauses() for elseif handling
    // - _stat.get_else_block() for else handling
    
    let _ = c; // suppress unused warning
    Ok(())
}

/// Compile while loop
fn compile_while_stat(
    c: &mut Compiler,
    stat: &LuaWhileStat,
) -> Result<(), String> {
    // TODO: Implement while loop compilation
    // Structure: while <condition> do <block> end
    //
    // Pseudo-implementation:
    // 1. Mark loop_start position
    // 2. Compile condition expression -> cond_reg
    // 3. Emit Test instruction: if cond_reg is false, jump to loop_end
    // 4. Compile body block
    // 5. Emit Jump back to loop_start
    // 6. Patch loop_end jump target
    //
    // Need to find correct API for:
    // - _stat.get_condition() to get condition expr
    // - _stat.get_block() to get loop body
    
    let _ = c;
    Ok(())
}

/// Compile repeat-until loop
fn compile_repeat_stat(
    c: &mut Compiler,
    stat: &LuaRepeatStat,
) -> Result<(), String> {
    // TODO: Implement repeat-until loop compilation
    // Structure: repeat <block> until <condition>
    //
    // Pseudo-implementation:
    // 1. Mark loop_start position
    // 2. Compile body block
    // 3. Compile condition expression -> cond_reg
    // 4. Emit Test instruction: if cond_reg is false, jump back to loop_start
    //
    // Need to find correct API for:
    // - _stat.get_block() to get loop body
    // - _stat.get_condition() to get condition expr
    
    let _ = c;
    Ok(())
}

/// Compile numeric for loop
fn compile_for_stat(c: &mut Compiler, _stat: &LuaForStat) -> Result<(), String> {
    // TODO: Implement numeric for loop compilation
    // Structure: for <var> = <start>, <end> [, <step>] do <block> end
    //
    // Pseudo-implementation:
    // 1. Compile start, end, step expressions
    // 2. Allocate iterator variable register
    // 3. Mark loop_start position
    // 4. Check if iterator <= end (or >= end if step < 0)
    // 5. If false, jump to loop_end
    // 6. Compile body block (with iterator variable in scope)
    // 7. Increment iterator by step
    // 8. Jump back to loop_start
    // 9. Patch loop_end jump target
    //
    // Need to find correct API for:
    // - _stat.get_var_name() to get loop variable name
    // - _stat.get_start_expr(), get_end_expr(), get_step_expr()
    // - _stat.get_block() to get loop body
    
    let _ = c;
    Ok(())
}

/// Compile generic for loop
fn compile_for_range_stat(
    c: &mut Compiler,
    stat: &LuaForRangeStat,
) -> Result<(), String> {
    // TODO: Implement generic for-in loop compilation
    // Structure: for <var-list> in <expr-list> do <block> end
    //
    // Pseudo-implementation:
    // 1. Compile iterator expressions (usually function calls like pairs, ipairs)
    // 2. Allocate registers for iterator function, state, control variable
    // 3. Mark loop_start position
    // 4. Call iterator function with state and control variable
    // 5. If result is nil, jump to loop_end
    // 6. Assign results to loop variables
    // 7. Compile body block
    // 8. Jump back to loop_start
    // 9. Patch loop_end jump target
    //
    // Need to find correct API for:
    // - _stat.get_var_list() to get loop variable names
    // - _stat.get_expr_list() to get iterator expressions
    // - _stat.get_block() to get loop body
    
    let _ = c;
    Ok(())
}

/// Compile do-end block
fn compile_do_stat(c: &mut Compiler, stat: &LuaDoStat) -> Result<(), String> {
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
