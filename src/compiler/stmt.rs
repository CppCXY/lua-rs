// Statement compilation

use super::expr::{compile_call_expr, compile_expr, compile_var_expr};
use super::{Compiler, helpers::*};
use crate::compiler::compile_block;
use crate::opcode::{Instruction, OpCode};
use crate::value::LuaValue;
use emmylua_parser::{
    LuaAssignStat, LuaCallExprStat, LuaDoStat, LuaForRangeStat, LuaForStat, LuaIfStat,
    LuaLocalStat, LuaRepeatStat, LuaReturnStat, LuaStat, LuaWhileStat,
};

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
    use emmylua_parser::LuaExpr;
    use super::expr::compile_call_expr_with_returns;
    
    let names: Vec<_> = stat.get_local_name_list().collect();
    let exprs: Vec<_> = stat.get_value_exprs().collect();

    // Compile init expressions
    let mut regs = Vec::new();
    
    if !exprs.is_empty() {
        // Compile all expressions except the last one
        for expr in exprs.iter().take(exprs.len().saturating_sub(1)) {
            let reg = compile_expr(c, expr)?;
            regs.push(reg);
        }
        
        // Handle the last expression specially if we need more values
        if let Some(last_expr) = exprs.last() {
            let remaining_vars = names.len().saturating_sub(regs.len());
            
            // Check if last expression is a function call (which might return multiple values)
            if let LuaExpr::CallExpr(call_expr) = last_expr {
                if remaining_vars > 1 {
                    // Compile call with multiple return values
                    let base_reg = compile_call_expr_with_returns(c, call_expr, remaining_vars)?;
                    
                    // The call result is in base_reg, and additional values in base_reg+1, base_reg+2, etc.
                    for i in 0..remaining_vars {
                        regs.push(base_reg + i as u32);
                    }
                } else {
                    // Single value needed
                    let reg = compile_expr(c, last_expr)?;
                    regs.push(reg);
                }
            } else {
                // Non-call expression
                let reg = compile_expr(c, last_expr)?;
                regs.push(reg);
            }
        }
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
    // Structure: if <condition> then <block> [elseif <condition> then <block>]* [else <block>] end
    let mut end_jumps = Vec::new();

    // Main if clause
    if let Some(cond) = stat.get_condition_expr() {
        let cond_reg = compile_expr(c, &cond)?;

        // Test condition: if false, jump to next clause
        emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
        let next_jump = emit_jump(c, OpCode::Jmp);

        // Compile then block
        if let Some(body) = stat.get_block() {
            compile_block(c, &body)?;
        }

        // After executing then block, jump to end
        end_jumps.push(emit_jump(c, OpCode::Jmp));
        
        // Patch jump to next clause (elseif or else)
        patch_jump(c, next_jump);
    }

    // Handle elseif clauses
    let elseif_clauses = stat.get_else_if_clause_list().collect::<Vec<_>>();
    for elseif_clause in elseif_clauses {
        if let Some(cond) = elseif_clause.get_condition_expr() {
            let cond_reg = compile_expr(c, &cond)?;

            // Test elseif condition
            emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
            let next_jump = emit_jump(c, OpCode::Jmp);

            // Compile elseif block
            if let Some(body) = elseif_clause.get_block() {
                compile_block(c, &body)?;
            }

            // After executing elseif block, jump to end
            end_jumps.push(emit_jump(c, OpCode::Jmp));
            
            // Patch jump to next clause
            patch_jump(c, next_jump);
        }
    }

    // Handle else clause
    if let Some(else_clause) = stat.get_else_clause() {
        if let Some(body) = else_clause.get_block() {
            compile_block(c, &body)?;
        }
    }

    // Patch all jumps to end
    for jump_pos in end_jumps {
        patch_jump(c, jump_pos);
    }

    Ok(())
}

/// Compile while loop
fn compile_while_stat(c: &mut Compiler, stat: &LuaWhileStat) -> Result<(), String> {
    // Structure: while <condition> do <block> end
    
    // Begin loop
    begin_loop(c);
    
    // Mark loop start
    let loop_start = c.chunk.code.len();

    // Compile condition
    let cond = stat
        .get_condition_expr()
        .ok_or("while statement missing condition")?;
    let cond_reg = compile_expr(c, &cond)?;

    // Test condition - if false (C=0), skip next instruction
    emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
    // If test passes (condition is false), jump to end
    let end_jump = emit_jump(c, OpCode::Jmp);

    // Compile body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Jump back to loop start
    let jump_offset = (c.chunk.code.len() - loop_start) as i32 + 1;
    emit(c, Instruction::encode_asbx(OpCode::Jmp, 0, -jump_offset));

    // Patch end jump
    patch_jump(c, end_jump);
    
    // End loop (patches all break statements)
    end_loop(c);

    Ok(())
}

/// Compile repeat-until loop
fn compile_repeat_stat(c: &mut Compiler, stat: &LuaRepeatStat) -> Result<(), String> {
    // Structure: repeat <block> until <condition>
    
    // Begin loop
    begin_loop(c);
    
    // Mark loop start
    let loop_start = c.chunk.code.len();

    // Compile body block
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Compile condition expression
    if let Some(cond_expr) = stat.get_condition_expr() {
        let cond_reg = compile_expr(c, &cond_expr)?;

        // repeat-until: continue loop if condition is false, exit if true
        // Test: if (is_truthy != c), skip next instruction
        // We want: if condition is true (1), skip Jmp (exit loop)
        // So c = 0: when true, 1 != 0 → skip Jmp
        emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
        
        // Jump back to loop start
        // PC will be at (current position + 1) when Jmp executes
        let jump_offset = loop_start as i32 - (c.chunk.code.len() as i32 + 1);
        emit(c, Instruction::encode_asbx(OpCode::Jmp, 0, jump_offset));
    }
    
    // End loop (patches all break statements)
    end_loop(c);

    Ok(())
}

/// Compile numeric for loop
fn compile_for_stat(c: &mut Compiler, stat: &LuaForStat) -> Result<(), String> {    
    // Structure: for <var> = <start>, <end> [, <step>] do <block> end
    
    // Get loop variable name
    let var_name = stat
        .get_var_name()
        .ok_or("for loop missing variable name")?
        .get_name_text()
        .to_string();

    // Get start, end, step expressions (as iterator)
    let exprs = stat.get_iter_expr().collect::<Vec<_>>();
    if exprs.len() < 2 {
        return Err("for loop requires at least start and end expressions".to_string());
    }

    // Compile expressions
    let start_reg = compile_expr(c, &exprs[0])?;
    let end_reg = compile_expr(c, &exprs[1])?;
    
    let step_reg = if exprs.len() >= 3 {
        compile_expr(c, &exprs[2])?
    } else {
        // Default step is 1 (positive)
        let const_idx = add_constant(c, crate::value::LuaValue::integer(1));
        let reg = alloc_register(c);
        emit_load_constant(c, reg, const_idx);
        reg
    };

    // Allocate iterator variable
    let iter_reg = alloc_register(c);
    emit_move(c, iter_reg, start_reg);

    // Begin new scope and add loop variable
    begin_scope(c);
    add_local(c, var_name, iter_reg);
    
    // Begin loop
    begin_loop(c);

    // Mark loop start
    let loop_start = c.chunk.code.len();

    // Universal condition check that works for both positive and negative steps:
    // (iter - end) * step <= 0
    // This is equivalent to:
    //   - If step > 0: iter <= end
    //   - If step < 0: iter >= end
    
    // Calculate (iter - end)
    let diff_reg = alloc_register(c);
    emit(c, Instruction::encode_abc(OpCode::Sub, diff_reg, iter_reg, end_reg));
    
    // Calculate (iter - end) * step
    let product_reg = alloc_register(c);
    emit(c, Instruction::encode_abc(OpCode::Mul, product_reg, diff_reg, step_reg));
    
    // Check if product <= 0
    let zero_const = add_constant(c, crate::value::LuaValue::integer(0));
    let zero_reg = alloc_register(c);
    emit_load_constant(c, zero_reg, zero_const);
    
    let cond_reg = alloc_register(c);
    emit(c, Instruction::encode_abc(OpCode::Le, cond_reg, product_reg, zero_reg));
    
    // Test condition: if false, exit loop
    emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
    let end_jump = emit_jump(c, OpCode::Jmp);

    // Compile loop body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Increment iterator: iter = iter + step
    let new_iter_reg = alloc_register(c);
    emit(c, Instruction::encode_abc(OpCode::Add, new_iter_reg, iter_reg, step_reg));
    emit_move(c, iter_reg, new_iter_reg);

    // Jump back to loop start
    let jump_offset = (c.chunk.code.len() - loop_start) as i32 + 1;
    emit(c, Instruction::encode_asbx(OpCode::Jmp, 0, -jump_offset));

    // Patch end jump
    patch_jump(c, end_jump);
    
    // End loop (patches all break statements)
    end_loop(c);

    // End scope
    end_scope(c);

    Ok(())
}

/// Compile generic for loop
fn compile_for_range_stat(c: &mut Compiler, stat: &LuaForRangeStat) -> Result<(), String> {    
    use emmylua_parser::LuaExpr;
    use super::expr::compile_call_expr_with_returns;
    
    // Structure: for <var-list> in <expr-list> do <block> end
    // Full iterator protocol: for var1, var2, ... in iter_func, state, init_val do ... end
    // Also supports: for var1, var2 in pairs(t) do ... end
    
    // Get loop variable names
    let var_names = stat.get_var_name_list()
        .map(|name| name.get_name_text().to_string())
        .collect::<Vec<_>>();
    
    if var_names.is_empty() {
        return Err("for-in loop requires at least one variable".to_string());
    }

    // Get iterator expressions (typically: iterator_func, state, initial_value)
    let iter_exprs = stat.get_expr_list().collect::<Vec<_>>();
    if iter_exprs.is_empty() {
        return Err("for-in loop requires iterator expression".to_string());
    }

    // Check if we have a single call expression (like pairs(t) or ipairs(t))
    // which returns multiple values
    let (iter_func_reg, state_reg, control_var_reg) = if iter_exprs.len() == 1 {
        if let LuaExpr::CallExpr(call_expr) = &iter_exprs[0] {
            // Single call expression - expect it to return (iter_func, state, control_var)
            let base_reg = compile_call_expr_with_returns(c, call_expr, 3)?;
            (base_reg, base_reg + 1, base_reg + 2)
        } else {
            // Single non-call expression - not valid for for-in
            return Err("for-in loop requires iterator function".to_string());
        }
    } else {
        // Multiple expressions: iter_func, state, control_var
        let mut iter_regs = Vec::new();
        for expr in &iter_exprs {
            iter_regs.push(compile_expr(c, expr)?);
        }
        
        let iter_func_reg = if !iter_regs.is_empty() {
            iter_regs[0]
        } else {
            return Err("for-in loop requires iterator function".to_string());
        };
        
        let state_reg = if iter_regs.len() > 1 {
            iter_regs[1]
        } else {
            let reg = alloc_register(c);
            emit_load_nil(c, reg);
            reg
        };
        
        let control_var_reg = if iter_regs.len() > 2 {
            iter_regs[2]
        } else {
            let reg = alloc_register(c);
            emit_load_nil(c, reg);
            reg
        };
        
        (iter_func_reg, state_reg, control_var_reg)
    };

    // Begin scope for loop variables
    begin_scope(c);

    // Allocate registers for loop variables
    let mut var_regs = Vec::new();
    for var_name in &var_names {
        let reg = alloc_register(c);
        add_local(c, var_name.clone(), reg);
        var_regs.push(reg);
    }
    
    // Begin loop
    begin_loop(c);

    // Mark loop start
    let loop_start = c.chunk.code.len();

    // Call iterator function: var1, var2, ... = iter_func(state, control_var)
    // Setup call: place function in a register, followed by arguments
    let call_base = alloc_register(c);
    emit_move(c, call_base, iter_func_reg);
    
    let arg1 = alloc_register(c);
    emit_move(c, arg1, state_reg);
    
    let arg2 = alloc_register(c);
    emit_move(c, arg2, control_var_reg);
    
    // Call with 2 arguments, expect as many return values as we have loop variables
    // OpCode::Call: A = func reg, B = num args + 1, C = num returns + 1
    let num_returns = var_names.len();
    emit(c, Instruction::encode_abc(OpCode::Call, call_base, 3, (num_returns + 1) as u32));
    
    // Move return values to loop variable registers
    for i in 0..var_names.len() {
        if i < var_regs.len() {
            emit_move(c, var_regs[i], call_base + i as u32);
        }
    }
    
    // Update control_var for next iteration
    emit_move(c, control_var_reg, call_base);
    
    // Check if first return value is nil (end of iteration)
    let is_nil_reg = alloc_register(c);
    let nil_const = add_constant(c, LuaValue::Nil);
    let nil_reg = alloc_register(c);
    emit_load_constant(c, nil_reg, nil_const);
    
    emit(c, Instruction::encode_abc(OpCode::Eq, is_nil_reg, var_regs[0], nil_reg));
    
    // If first value is nil, exit loop
    emit(c, Instruction::encode_abc(OpCode::Test, is_nil_reg, 0, 1));
    let end_jump = emit_jump(c, OpCode::Jmp);

    // Compile loop body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Jump back to loop start
    let jump_offset = (c.chunk.code.len() - loop_start) as i32 + 1;
    emit(c, Instruction::encode_asbx(OpCode::Jmp, 0, -jump_offset));

    // Patch end jump
    patch_jump(c, end_jump);
    
    // End loop (patches all break statements)
    end_loop(c);

    // End scope
    end_scope(c);

    Ok(())
}

/// Compile do-end block
fn compile_do_stat(c: &mut Compiler, stat: &LuaDoStat) -> Result<(), String> {
    begin_scope(c);

    if let Some(block) = stat.get_block() {
        compile_block(c, &block)?;
    }

    end_scope(c);

    Ok(())
}

/// Compile break statement
fn compile_break_stat(c: &mut Compiler) -> Result<(), String> {
    emit_break(c)
}
