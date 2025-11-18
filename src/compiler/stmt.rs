// Statement compilation

use super::expr::{compile_call_expr, compile_expr, compile_expr_to, compile_var_expr};
use super::{Compiler, Local, helpers::*};
use crate::compiler::compile_block;
use crate::compiler::expr::{compile_call_expr_with_returns, compile_closure_expr};
use crate::lua_value::LuaValue;
use crate::opcode::{Instruction, OpCode};
use emmylua_parser::{
    LuaAssignStat, LuaCallExprStat, LuaDoStat, LuaExpr, LuaForRangeStat, LuaForStat, LuaFuncStat,
    LuaGotoStat, LuaIfStat, LuaLabelStat, LuaLocalStat, LuaRepeatStat, LuaReturnStat, LuaStat,
    LuaVarExpr, LuaWhileStat, LuaBinaryExpr, BinaryOperator, LuaLiteralExpr, LuaLiteralToken,
};

/// Try to compile binary expression as immediate comparison for control flow
/// Returns Some(register) if successful (comparison instruction emitted)
/// The emitted instruction skips next instruction if comparison is FALSE
fn try_compile_immediate_comparison(c: &mut Compiler, expr: &LuaExpr) -> Result<Option<u32>, String> {
    // Only handle binary comparison expressions
    if let LuaExpr::BinaryExpr(bin_expr) = expr {
        let (left, right) = bin_expr.get_exprs().ok_or("error")?;
        let op = bin_expr.get_op_token().ok_or("error")?;
        let op_kind = op.get_op();
        
        // Check if right operand is small integer constant
        if let LuaExpr::LiteralExpr(lit) = &right {
            if let Some(LuaLiteralToken::Number(num)) = lit.get_literal() {
                if !num.is_float() {
                    let int_val = num.get_int_value();
                    // Use signed 9-bit immediate: range [-256, 255]
                    if int_val >= -256 && int_val <= 255 {
                        // Compile left operand
                        let left_reg = compile_expr(c, &left)?;
                        
                        // Encode immediate value (9 bits)
                        let imm = if int_val < 0 {
                            (int_val + 512) as u32
                        } else {
                            int_val as u32
                        };
                        
                        // Emit immediate comparison (skips next if FALSE)
                        match op_kind {
                            BinaryOperator::OpLt => {
                                emit(c, Instruction::encode_abc(OpCode::LtI, left_reg, imm, 1));
                                return Ok(Some(left_reg));
                            }
                            BinaryOperator::OpLe => {
                                emit(c, Instruction::encode_abc(OpCode::LeI, left_reg, imm, 1));
                                return Ok(Some(left_reg));
                            }
                            BinaryOperator::OpGt => {
                                emit(c, Instruction::encode_abc(OpCode::GtI, left_reg, imm, 1));
                                return Ok(Some(left_reg));
                            }
                            BinaryOperator::OpGe => {
                                emit(c, Instruction::encode_abc(OpCode::GeI, left_reg, imm, 1));
                                return Ok(Some(left_reg));
                            }
                            BinaryOperator::OpEq => {
                                emit(c, Instruction::encode_abc(OpCode::EqI, left_reg, imm, 1));
                                return Ok(Some(left_reg));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    
    Ok(None)
}

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
        LuaStat::GotoStat(s) => compile_goto_stat(c, s),
        LuaStat::LabelStat(s) => compile_label_stat(c, s),
        LuaStat::FuncStat(s) => compile_function_stat(c, s),
        LuaStat::LocalFuncStat(s) => compile_local_function_stat(c, s),
        _ => Ok(()), // Other statements not yet implemented
    }
}

/// Compile local variable declaration
fn compile_local_stat(c: &mut Compiler, stat: &LuaLocalStat) -> Result<(), String> {
    use emmylua_parser::LuaExpr;

    let names: Vec<_> = stat.get_local_name_list().collect();
    let exprs: Vec<_> = stat.get_value_exprs().collect();

    // Compile init expressions
    let mut regs = Vec::new();

    if !exprs.is_empty() {
        // Compile all expressions except the last one
        for expr in exprs.iter().take(exprs.len().saturating_sub(1)) {
            // Allocate a new register for each variable
            let dest_reg = alloc_register(c);
            let src_reg = compile_expr(c, expr)?;
            if src_reg != dest_reg {
                emit_move(c, dest_reg, src_reg);
            }
            regs.push(dest_reg);
        }

        // Handle the last expression specially if we need more values
        if let Some(last_expr) = exprs.last() {
            let remaining_vars = names.len().saturating_sub(regs.len());

            // Check if last expression is ... (varargs) which should expand
            let is_dots = if let LuaExpr::LiteralExpr(lit_expr) = last_expr {
                matches!(
                    lit_expr.get_literal(),
                    Some(emmylua_parser::LuaLiteralToken::Dots(_))
                )
            } else {
                false
            };

            if is_dots && remaining_vars > 0 {
                // Varargs expansion: generate VarArg instruction with B=0 (all varargs)
                // or B=remaining_vars+1 (specific number)
                let base_reg = alloc_register(c);

                // Allocate registers for all remaining variables
                for _i in 1..remaining_vars {
                    alloc_register(c);
                }

                // VarArg instruction: R(base_reg)..R(base_reg+remaining_vars-1) = ...
                // B = remaining_vars + 1 (or 0 for all)
                let b_value = if remaining_vars == 1 {
                    2
                } else {
                    (remaining_vars + 1) as u32
                };
                emit(
                    c,
                    Instruction::encode_abc(OpCode::VarArg, base_reg, b_value, 0),
                );

                // Add all registers
                for i in 0..remaining_vars {
                    regs.push(base_reg + i as u32);
                }
            }
            // Check if last expression is a function call (which might return multiple values)
            else if let LuaExpr::CallExpr(call_expr) = last_expr {
                if remaining_vars > 1 {
                    // Use compile_call_expr_with_returns to handle multi-return
                    let base_reg = compile_call_expr_with_returns(c, call_expr, remaining_vars)?;

                    // Ensure next_register is past all return registers
                    while c.next_register < base_reg + remaining_vars as u32 {
                        alloc_register(c);
                    }

                    // Add all return registers
                    for i in 0..remaining_vars {
                        regs.push(base_reg + i as u32);
                    }
                } else {
                    // Single value needed
                    let dest_reg = alloc_register(c);
                    let src_reg = compile_expr(c, last_expr)?;
                    if src_reg != dest_reg {
                        emit_move(c, dest_reg, src_reg);
                    }
                    regs.push(dest_reg);
                }
            } else {
                // Non-call expression
                let dest_reg = alloc_register(c);
                let src_reg = compile_expr(c, last_expr)?;
                if src_reg != dest_reg {
                    emit_move(c, dest_reg, src_reg);
                }
                regs.push(dest_reg);
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
    use super::expr::compile_expr_to;
    
    // Get vars and expressions from children
    let (vars, exprs) = stat.get_var_and_expr_list();

    if vars.is_empty() {
        return Ok(());
    }

    // OPTIMIZATION: For single local variable assignment, compile directly to target register
    if vars.len() == 1 && exprs.len() == 1 {
        if let LuaVarExpr::NameExpr(name_expr) = &vars[0] {
            let name = name_expr.get_name_text().unwrap_or("".to_string());
            if let Some(local) = resolve_local(c, &name) {
                // Local variable - compile expression directly to its register
                compile_expr_to(c, &exprs[0], Some(local.register))?;
                return Ok(());
            }
        }
    }

    // Standard path: compile expressions
    let mut val_regs = Vec::new();

    for (i, expr) in exprs.iter().enumerate() {
        let is_last = i == exprs.len() - 1;

        // If this is the last expression and it's a call, request multiple returns
        if is_last && matches!(expr, LuaExpr::CallExpr(_)) {
            let remaining_vars = vars.len().saturating_sub(val_regs.len());
            if remaining_vars > 0 {
                if let LuaExpr::CallExpr(call_expr) = expr {
                    let base_reg = compile_call_expr_with_returns(c, call_expr, remaining_vars)?;
                    // Collect all return values
                    for j in 0..remaining_vars {
                        val_regs.push(base_reg + j as u32);
                    }
                    break;
                }
            }
        }

        // Regular expression
        let reg = compile_expr(c, expr)?;
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
        return Ok(());
    }

    // Tail call optimization: if return has a single call expression, use TailCall
    if exprs.len() == 1 {
        if let LuaExpr::CallExpr(call_expr) = &exprs[0] {
            // This is a tail call: return func(...)
            // Get function being called
            let func_expr = if let Some(prefix) = call_expr.get_prefix_expr() {
                prefix
            } else {
                return Err("Tail call missing function expression".to_string());
            };

            // Get arguments first to know how many we have
            let args = if let Some(args_list) = call_expr.get_args_list() {
                args_list.get_args().collect::<Vec<_>>()
            } else {
                Vec::new()
            };

            // Reserve all registers we'll need (func + args)
            let num_total = 1 + args.len();
            let mut reserved_regs = Vec::new();
            for _ in 0..num_total {
                reserved_regs.push(alloc_register(c));
            }
            let base_reg = reserved_regs[0];

            // Compile function to the first reserved register
            let func_reg = compile_expr(c, &func_expr)?;
            if func_reg != base_reg {
                emit_move(c, base_reg, func_reg);
            }

            // Compile arguments to consecutive registers after function
            for (i, arg) in args.iter().enumerate() {
                let target_reg = reserved_regs[i + 1];
                let arg_reg = compile_expr(c, &arg)?;
                if arg_reg != target_reg {
                    emit_move(c, target_reg, arg_reg);
                }
            }

            // Emit TailCall instruction
            // A = function register, B = num_args + 1
            let num_args = args.len();
            emit(
                c,
                Instruction::encode_abc(OpCode::TailCall, base_reg, (num_args + 1) as u32, 0),
            );

            return Ok(());
        }
    }

    // Allocate consecutive registers for all return values
    let base_reg = alloc_register(c);
    let num_exprs = exprs.len();

    // Reserve registers for all return values
    for _ in 1..num_exprs {
        alloc_register(c);
    }

    // Compile all expressions into consecutive registers
    for (i, expr) in exprs.iter().enumerate() {
        let target_reg = base_reg + i as u32;
        let src_reg = compile_expr(c, expr)?;
        if src_reg != target_reg {
            emit_move(c, target_reg, src_reg);
        }
    }

    // Emit return: B = num_values + 1
    // Return instruction: OpCode::Return, A = base_reg, B = num_values + 1
    emit(
        c,
        Instruction::encode_abc(OpCode::Return, base_reg, (num_exprs + 1) as u32, 0),
    );

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

    // OPTIMIZATION: Try to compile condition as immediate comparison (skip Test instruction)
    let cond = stat
        .get_condition_expr()
        .ok_or("while statement missing condition")?;
    
    // Check if condition is immediate comparison pattern: var < constant
    let end_jump = if let Some(_imm_reg) = try_compile_immediate_comparison(c, &cond)? {
        // Success! Generated LtI/LeI/GtI etc that skips next instruction if FALSE
        // Now emit jump to exit when comparison fails
        emit_jump(c, OpCode::Jmp)
    } else {
        // Standard path: compile expression + Test
        let cond_reg = compile_expr(c, &cond)?;
        emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
        emit_jump(c, OpCode::Jmp)
    };

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
        // OPTIMIZATION: Try immediate comparison (skip Test)
        if let Some(_) = try_compile_immediate_comparison(c, &cond_expr)? {
            // Immediate comparison skips if FALSE, so Jmp executes when condition is FALSE
            // This is correct for repeat-until (continue when false)
            let jump_offset = loop_start as i32 - (c.chunk.code.len() as i32 + 1);
            emit(c, Instruction::encode_asbx(OpCode::Jmp, 0, jump_offset));
        } else {
            // Standard path
            let cond_reg = compile_expr(c, &cond_expr)?;
            emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
            let jump_offset = loop_start as i32 - (c.chunk.code.len() as i32 + 1);
            emit(c, Instruction::encode_asbx(OpCode::Jmp, 0, jump_offset));
        }
    }

    // End loop (patches all break statements)
    end_loop(c);

    Ok(())
}

/// Compile numeric for loop
fn compile_for_stat(c: &mut Compiler, stat: &LuaForStat) -> Result<(), String> {
    // Structure: for <var> = <start>, <end> [, <step>] do <block> end
    // Use efficient FORPREP/FORLOOP instructions like Lua

    // Get loop variable name
    let var_name = stat
        .get_var_name()
        .ok_or("for loop missing variable name")?
        .get_name_text()
        .to_string();

    // Get start, end, step expressions
    let exprs = stat.get_iter_expr().collect::<Vec<_>>();
    if exprs.len() < 2 {
        return Err("for loop requires at least start and end expressions".to_string());
    }

    // Allocate registers for loop control variables in sequence
    // R(base) = index, R(base+1) = limit, R(base+2) = step, R(base+3) = loop var
    let base_reg = alloc_register(c);
    let limit_reg = alloc_register(c);
    let step_reg = alloc_register(c);
    let var_reg = alloc_register(c);

    // Compile expressions DIRECTLY to target registers - avoid intermediate registers
    let _ = compile_expr_to(c, &exprs[0], Some(base_reg))?;
    let _ = compile_expr_to(c, &exprs[1], Some(limit_reg))?;

    // Compile step expression (default 1)
    if exprs.len() >= 3 {
        let _ = compile_expr_to(c, &exprs[2], Some(step_reg))?;
    } else {
        let const_idx = add_constant(c, LuaValue::integer(1));
        emit_load_constant(c, step_reg, const_idx);
    }

    // Emit FORPREP: R(base) -= R(step); jump to FORLOOP (not loop body)
    let forprep_pc = c.chunk.code.len();
    emit(c, Instruction::encode_asbx(OpCode::ForPrep, base_reg, 0)); // Will patch later

    // Begin new scope for loop body
    begin_scope(c);

    // The loop variable is at R(base+3)
    add_local(c, var_name, var_reg);
    begin_loop(c);

    // Loop body starts here
    let loop_body_start = c.chunk.code.len();

    // Compile loop body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // FORLOOP comes AFTER the body
    let forloop_pc = c.chunk.code.len();
    // Emit FORLOOP: increments index, checks condition, copies to var, jumps back to body
    let forloop_offset = (loop_body_start as i32) - (forloop_pc as i32) - 1;
    emit(
        c,
        Instruction::encode_asbx(OpCode::ForLoop, base_reg, forloop_offset),
    );

    // Patch FORPREP to jump to FORLOOP (not body)
    let prep_jump = (forloop_pc as i32) - (forprep_pc as i32) - 1;
    c.chunk.code[forprep_pc] = Instruction::encode_asbx(OpCode::ForPrep, base_reg, prep_jump);

    end_loop(c);
    end_scope(c);

    Ok(())
}

/// Compile generic for loop
fn compile_for_range_stat(c: &mut Compiler, stat: &LuaForRangeStat) -> Result<(), String> {
    use super::expr::compile_call_expr_with_returns;
    use emmylua_parser::LuaExpr;

    // Structure: for <var-list> in <expr-list> do <block> end
    // Full iterator protocol: for var1, var2, ... in iter_func, state, init_val do ... end
    // Also supports: for var1, var2 in pairs(t) do ... end

    // Get loop variable names
    let var_names = stat
        .get_var_name_list()
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
    emit(
        c,
        Instruction::encode_abc(OpCode::Call, call_base, 3, (num_returns + 1) as u32),
    );

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
    let nil_const = add_constant(c, LuaValue::nil());
    let nil_reg = alloc_register(c);
    emit_load_constant(c, nil_reg, nil_const);

    emit(
        c,
        Instruction::encode_abc(OpCode::Eq, is_nil_reg, var_regs[0], nil_reg),
    );

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

fn compile_goto_stat(c: &mut Compiler, stat: &LuaGotoStat) -> Result<(), String> {
    let label_name = stat
        .get_label_name_token()
        .ok_or("goto statement missing label name")?
        .get_name_text()
        .to_string();

    emit_goto(c, label_name)?;
    Ok(())
}

fn compile_label_stat(c: &mut Compiler, stat: &LuaLabelStat) -> Result<(), String> {
    let label_name = stat
        .get_label_name_token()
        .ok_or("label statement missing label name")?
        .get_name_text()
        .to_string();

    define_label(c, label_name)?;
    Ok(())
}

fn compile_function_stat(c: &mut Compiler, stat: &LuaFuncStat) -> Result<(), String> {
    let func_name_var_expr = stat
        .get_func_name()
        .ok_or("function statement missing function name")?;

    let closure = stat
        .get_closure()
        .ok_or("function statement missing function body")?;

    let is_colon = if let LuaVarExpr::IndexExpr(index_expr) = &func_name_var_expr {
        index_expr
            .get_index_token()
            .ok_or("Missing index token")?
            .is_colon()
    } else {
        false
    };
    // Compile the closure to get function value
    let func_reg = compile_closure_expr(c, &closure, is_colon)?;

    compile_var_expr(c, &func_name_var_expr, func_reg)?;

    Ok(())
}

fn compile_local_function_stat(
    c: &mut Compiler,
    stat: &emmylua_parser::LuaLocalFuncStat,
) -> Result<(), String> {
    let local_name = stat
        .get_local_name()
        .ok_or("local function statement missing function name")?;
    let func_name = local_name
        .get_name_token()
        .ok_or("local function statement missing function name token")?
        .get_name_text()
        .to_string();

    let closure = stat
        .get_closure()
        .ok_or("local function statement missing function body")?;

    // Declare the local variable first (for recursion support)
    let func_reg = c.next_register;
    c.next_register += 1;

    c.scope_chain.borrow_mut().locals.push(Local {
        name: func_name.clone(),
        depth: c.scope_depth,
        register: func_reg,
    });
    c.chunk.locals.push(func_name);

    // Save and restore next_register to compile closure into func_reg
    let saved_next = c.next_register;
    c.next_register = func_reg;

    // Compile the closure
    let closure_reg = compile_closure_expr(c, &closure, false)?;

    // Restore next_register (should be func_reg + 1)
    c.next_register = saved_next.max(closure_reg + 1);

    // Move closure to the local variable register if different
    if closure_reg != func_reg {
        let move_instr = Instruction::encode_abc(OpCode::Move, func_reg, closure_reg, 0);
        c.chunk.code.push(move_instr);
    }

    Ok(())
}
