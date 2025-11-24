// Statement compilation

use super::expr::{
    compile_call_expr, compile_call_expr_with_returns_and_dest, compile_expr, compile_expr_to,
    compile_var_expr,
};
use super::{Compiler, Local, helpers::*};
use crate::compiler::compile_block;
use crate::compiler::expr::{compile_call_expr_with_returns, compile_closure_expr, compile_closure_expr_to};
use crate::lua_value::LuaValue;
use crate::lua_vm::{Instruction, OpCode};
use emmylua_parser::{
    BinaryOperator, LuaAssignStat, LuaBlock, LuaCallExprStat, LuaDoStat, LuaExpr, LuaForRangeStat,
    LuaForStat, LuaFuncStat, LuaGotoStat, LuaIfStat, LuaLabelStat, LuaLiteralToken, LuaLocalStat,
    LuaRepeatStat, LuaReturnStat, LuaStat, LuaVarExpr, LuaWhileStat,
};

/// Check if a block contains only a single unconditional jump statement (break/return only)
/// Note: goto is NOT optimized by luac, so we don't include it here
#[allow(dead_code)]
fn is_single_jump_block(block: &LuaBlock) -> bool {
    let stats: Vec<_> = block.get_stats().collect();
    if stats.len() != 1 {
        return false;
    }
    matches!(stats[0], LuaStat::BreakStat(_) | LuaStat::ReturnStat(_))
}

/// Try to compile binary expression as immediate comparison for control flow
/// Returns Some(register) if successful (comparison instruction emitted)
/// The emitted instruction skips next instruction if comparison result matches `invert`
/// invert=false: skip if FALSE (normal if-then), invert=true: skip if TRUE (optimized break/goto/return)
fn try_compile_immediate_comparison(
    c: &mut Compiler,
    expr: &LuaExpr,
    invert: bool,
) -> Result<Option<u32>, String> {
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
                    // Lua 5.4 immediate comparisons use signed sB field (8 bits): range [-128, 127]
                    // But encoded as unsigned in instruction, so range is [0, 255] with wraparound
                    if int_val >= -128 && int_val <= 127 {
                        // Compile left operand
                        let left_reg = compile_expr(c, &left)?;

                        // Encode immediate value with OFFSET_SB = 128 for signed B field
                        let imm = ((int_val + 128) & 0xFF) as u32;

                        // Emit immediate comparison
                        // C parameter controls skip behavior:
                        //   C=0: skip next if FALSE (normal if-then: true executes then-block)
                        //   C=1: skip next if TRUE (inverted: true skips the jump, false executes jump)
                        let c_param = if invert { 1 } else { 0 };

                        match op_kind {
                            BinaryOperator::OpLt => {
                                emit(
                                    c,
                                    Instruction::encode_abc(OpCode::LtI, left_reg, imm, c_param),
                                );
                                return Ok(Some(left_reg));
                            }
                            BinaryOperator::OpLe => {
                                emit(
                                    c,
                                    Instruction::encode_abc(OpCode::LeI, left_reg, imm, c_param),
                                );
                                return Ok(Some(left_reg));
                            }
                            BinaryOperator::OpGt => {
                                emit(
                                    c,
                                    Instruction::encode_abc(OpCode::GtI, left_reg, imm, c_param),
                                );
                                return Ok(Some(left_reg));
                            }
                            BinaryOperator::OpGe => {
                                emit(
                                    c,
                                    Instruction::encode_abc(OpCode::GeI, left_reg, imm, c_param),
                                );
                                return Ok(Some(left_reg));
                            }
                            BinaryOperator::OpEq => {
                                emit(
                                    c,
                                    Instruction::encode_abc(OpCode::EqI, left_reg, imm, c_param),
                                );
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
    let result = match stat {
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
    };

    // After each statement, reset freereg to active local variables
    // This matches Lua's: fs->freereg = luaY_nvarstack(fs);
    if result.is_ok() {
        reset_freereg(c);
    }

    result
}

/// Compile local variable declaration
fn compile_local_stat(c: &mut Compiler, stat: &LuaLocalStat) -> Result<(), String> {
    use super::expr::{compile_expr_to, compile_call_expr_with_returns_and_dest};
    use emmylua_parser::LuaExpr;

    let names: Vec<_> = stat.get_local_name_list().collect();
    let exprs: Vec<_> = stat.get_value_exprs().collect();

    // CRITICAL FIX: Pre-allocate registers for local variables
    // This ensures expressions compile into the correct target registers
    // Example: `local a = f()` should place f's result directly into a's register
    let base_reg = c.freereg;
    let num_vars = names.len();
    
    // Pre-allocate registers for all local variables
    for _ in 0..num_vars {
        alloc_register(c);
    }
    
    // Now compile init expressions into the pre-allocated registers
    let mut regs = Vec::new();

    if !exprs.is_empty() {
        // Compile all expressions except the last one
        for (i, expr) in exprs.iter().take(exprs.len().saturating_sub(1)).enumerate() {
            let target_reg = base_reg + i as u32;
            // Compile expression with target register specified
            let result_reg = compile_expr_to(c, expr, Some(target_reg))?;
            regs.push(result_reg);
        }

        // Handle the last expression specially if we need more values
        if let Some(last_expr) = exprs.last() {
            let remaining_vars = num_vars.saturating_sub(regs.len());
            let target_base = base_reg + regs.len() as u32;

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
                // Varargs expansion: generate VarArg instruction into pre-allocated registers
                // VarArg instruction: R(target_base)..R(target_base+remaining_vars-1) = ...
                // B = remaining_vars + 1 (or 0 for all)
                let b_value = if remaining_vars == 1 {
                    2
                } else {
                    (remaining_vars + 1) as u32
                };
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Vararg, target_base, b_value, 0),
                );

                // Add all registers
                for i in 0..remaining_vars {
                    regs.push(target_base + i as u32);
                }
            }
            // Check if last expression is a function call (which might return multiple values)
            else if let LuaExpr::CallExpr(call_expr) = last_expr {
                if remaining_vars > 1 {
                    // Multi-return call: compile with dest = target_base
                    let result_base = compile_call_expr_with_returns_and_dest(c, call_expr, remaining_vars, Some(target_base))?;

                    // Add all return registers
                    for i in 0..remaining_vars {
                        regs.push(result_base + i as u32);
                    }
                    
                    // Define locals and return
                    for (i, name) in names.iter().enumerate() {
                        if let Some(name_token) = name.get_name_token() {
                            let name_text = name_token.get_name_text().to_string();
                            let mut is_const = false;
                            let mut is_to_be_closed = false;
                            if let Some(attr_token) = name.get_attrib() {
                                is_const = attr_token.is_const();
                                is_to_be_closed = attr_token.is_close();
                            }
                            add_local_with_attrs(c, name_text, regs[i], is_const, is_to_be_closed);
                        }
                    }
                    return Ok(());
                } else {
                    // Single value: compile with dest = target_base
                    let result_reg = compile_expr_to(c, last_expr, Some(target_base))?;
                    regs.push(result_reg);
                }
            } else {
                // Non-call expression: compile with dest = target_base
                let result_reg = compile_expr_to(c, last_expr, Some(target_base))?;
                regs.push(result_reg);
            }
        }
    }

    // Fill missing values with nil (batch optimization)
    if regs.len() < names.len() {
        // Use pre-allocated registers instead of allocating new ones
        let first_nil_reg = base_reg + regs.len() as u32;
        let nil_count = names.len() - regs.len();

        // Emit single LOADNIL instruction for all nil values
        // Format: LOADNIL A B - loads nil into R(A)..R(A+B)
        emit(
            c,
            Instruction::encode_abc(OpCode::LoadNil, first_nil_reg, (nil_count - 1) as u32, 0),
        );

        // Add all nil registers to regs
        for i in 0..nil_count {
            regs.push(first_nil_reg + i as u32);
        }
    }

    // Define locals with attributes
    for (i, name) in names.iter().enumerate() {
        // Get name text from LocalName node
        if let Some(name_token) = name.get_name_token() {
            let name_text = name_token.get_name_text().to_string();

            // Parse attributes: <const> or <close>
            let mut is_const = false;
            let mut is_to_be_closed = false;

            // Check if LocalName has an attribute
            if let Some(attr_token) = name.get_attrib() {
                is_const = attr_token.is_const();
                is_to_be_closed = attr_token.is_close();
            }

            add_local_with_attrs(c, name_text, regs[i], is_const, is_to_be_closed);
        }
    }

    Ok(())
}

/// Compile assignment statement
fn compile_assign_stat(c: &mut Compiler, stat: &LuaAssignStat) -> Result<(), String> {
    use super::exp2reg::exp_to_next_reg;
    use super::expr::{compile_expr_desc, compile_expr_to};
    use emmylua_parser::LuaIndexKey;

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
                // Check if trying to assign to a const variable
                if local.is_const {
                    return Err(format!("attempt to assign to const variable '{}'", name));
                }
                // Local variable - compile expression directly to its register
                let _result_reg = compile_expr_to(c, &exprs[0], Some(local.register))?;
                return Ok(());
            }
            
            // Check if it's an upvalue assignment
            if let Some(upvalue_index) = resolve_upvalue_from_chain(c, &name) {
                // Compile expression to a temporary register
                let value_reg = compile_expr(c, &exprs[0])?;
                // Emit SETUPVAL to copy value to upvalue
                emit(c, Instruction::encode_abc(OpCode::SetUpval, value_reg, upvalue_index as u32, 0));
                // Note: We don't free the register here to maintain compatibility
                // Standard Lua may reuse it, but we need more careful analysis
                return Ok(());
            }

            // OPTIMIZATION: global = constant -> SETTABUP with k=1
            if resolve_upvalue_from_chain(c, &name).is_none() {
                // It's a global - add key to constants first (IMPORTANT: luac adds key before value!)
                let lua_str = create_string_value(c, &name);
                let key_idx = add_constant_dedup(c, lua_str);

                // Then check if value is constant
                if let Some(const_idx) = try_expr_as_constant(c, &exprs[0]) {
                    if key_idx <= Instruction::MAX_B && const_idx <= Instruction::MAX_C {
                        emit(
                            c,
                            Instruction::create_abck(OpCode::SetTabUp, 0, key_idx, const_idx, true),
                        );
                        return Ok(());
                    }
                }
            }
        }

        // OPTIMIZATION: For table.field = constant, use SetField with RK
        if let LuaVarExpr::IndexExpr(index_expr) = &vars[0] {
            if let Some(LuaIndexKey::Name(name_token)) = index_expr.get_index_key() {
                // Try to compile value as constant
                if let Some(const_idx) = try_expr_as_constant(c, &exprs[0]) {
                    // Compile table expression
                    let prefix_expr = index_expr
                        .get_prefix_expr()
                        .ok_or("Index expression missing table")?;
                    let table_reg = compile_expr(c, &prefix_expr)?;

                    // Get field name as constant
                    let field_name = name_token.get_name_text().to_string();
                    let lua_str = create_string_value(c, &field_name);
                    let key_idx = add_constant_dedup(c, lua_str);

                    // Emit SetField with k=1 (value is constant)
                    if const_idx <= Instruction::MAX_C && key_idx <= Instruction::MAX_B {
                        emit(
                            c,
                            Instruction::create_abck(
                                OpCode::SetField,
                                table_reg,
                                key_idx,
                                const_idx,
                                true, // k=1: C is constant index
                            ),
                        );
                        return Ok(());
                    }
                }
            }
        }
    }

    // Multi-assignment: use luac's reverse-order strategy
    // Strategy:
    // 1. Get target registers for all variables
    // 2. Compile expressions to temps, but last value can go to its target directly
    // 3. Emit moves in reverse order to avoid conflicts

    let mut target_regs = Vec::new();

    // Collect target registers for local variables and check const
    let mut all_locals = true;
    for var in vars.iter() {
        if let LuaVarExpr::NameExpr(name_expr) = var {
            let name = name_expr.get_name_text().unwrap_or("".to_string());
            if let Some(local) = resolve_local(c, &name) {
                // Check if trying to assign to a const variable
                if local.is_const {
                    return Err(format!("attempt to assign to const variable '{}'", name));
                }
                target_regs.push(Some(local.register));
                continue;
            }
        }
        target_regs.push(None);
        all_locals = false;
    }

    // Compile expressions
    let mut val_regs = Vec::new();

    for (i, expr) in exprs.iter().enumerate() {
        let is_last = i == exprs.len() - 1;

        // If this is the last expression and it's a call, request multiple returns
        if is_last && matches!(expr, LuaExpr::CallExpr(_)) {
            let remaining_vars = vars.len().saturating_sub(val_regs.len());
            if remaining_vars > 0 {
                if let LuaExpr::CallExpr(call_expr) = expr {
                    let base_reg = compile_call_expr_with_returns(c, call_expr, remaining_vars)?;
                    for j in 0..remaining_vars {
                        val_regs.push(base_reg + j as u32);
                    }
                    break;
                }
            }
        }

        // NEW: Use ExpDesc system to compile expressions
        // This is THE KEY FIX that eliminates extra register allocation!
        let reg = if is_last && all_locals && val_regs.len() + 1 == vars.len() {
            // Last value can go directly to its target
            if let Some(target_reg) = target_regs[val_regs.len()] {
                // CRITICAL: compile_expr_to may NOT honor dest if it's unsafe!
                // Always use the returned register value
                let result_reg = compile_expr_to(c, expr, Some(target_reg))?;
                // If result ended up in different register, move it
                if result_reg != target_reg {
                    emit_move(c, target_reg, result_reg);
                    target_reg
                } else {
                    result_reg
                }
            } else {
                compile_expr(c, expr)?
            }
        } else {
            // CRITICAL OPTIMIZATION: Use exp_to_next_reg instead of manual allocation!
            // This ensures we allocate exactly one register per expression
            let mut e = compile_expr_desc(c, expr)?;
            exp_to_next_reg(c, &mut e);
            e.get_register().ok_or("Expression has no register")?
        };
        val_regs.push(reg);
    }

    // Fill missing values with nil
    if val_regs.len() < vars.len() {
        let nil_count = vars.len() - val_regs.len();
        let first_nil_reg = alloc_register(c);

        // Allocate remaining registers
        for _ in 1..nil_count {
            alloc_register(c);
        }

        // Emit LOADNIL (batch)
        if nil_count == 1 {
            emit(
                c,
                Instruction::encode_abc(OpCode::LoadNil, first_nil_reg, 0, 0),
            );
        } else {
            emit(
                c,
                Instruction::encode_abc(OpCode::LoadNil, first_nil_reg, (nil_count - 1) as u32, 0),
            );
        }

        for i in 0..nil_count {
            val_regs.push(first_nil_reg + i as u32);
        }
    }

    // Emit assignments in REVERSE order (luac optimization)
    if all_locals && vars.len() > 1 {
        // All local variables: use reverse-order moves
        for i in (0..vars.len()).rev() {
            if let Some(target_reg) = target_regs[i] {
                if val_regs[i] != target_reg {
                    emit_move(c, target_reg, val_regs[i]);
                }
            }
        }
    } else {
        // Not all locals: compile normally with RK optimization for globals
        for (i, var) in vars.iter().enumerate() {
            // Special case: global variable assignment with constant value
            if let LuaVarExpr::NameExpr(name_expr) = var {
                let name = name_expr.get_name_text().unwrap_or("".to_string());

                // Check if it's NOT a local (i.e., it's a global)
                if resolve_local(c, &name).is_none()
                    && resolve_upvalue_from_chain(c, &name).is_none()
                {
                    // It's a global - try to use RK optimization
                    // Check if value_reg contains a recently loaded constant
                    if i < exprs.len() {
                        if let Some(const_idx) = try_expr_as_constant(c, &exprs[i]) {
                            // Use SETTABUP with k=1 (both key and value are constants)
                            let lua_str = create_string_value(c, &name);
                            let key_idx = add_constant_dedup(c, lua_str);

                            if key_idx <= Instruction::MAX_B && const_idx <= Instruction::MAX_C {
                                emit(
                                    c,
                                    Instruction::create_abck(
                                        OpCode::SetTabUp,
                                        0,
                                        key_idx,
                                        const_idx,
                                        true,
                                    ),
                                );
                                continue;
                            }
                        }
                    }
                }
            }

            compile_var_expr(c, var, val_regs[i])?;
        }
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
        // return (no values) - use Return0 optimization
        emit(c, Instruction::encode_abc(OpCode::Return0, 0, 0, 0));
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
            let mut last_is_vararg_all_out = false;
            for (i, arg) in args.iter().enumerate() {
                let target_reg = reserved_regs[i + 1];
                let is_last_arg = i == args.len() - 1;

                // Check if last argument is ... (vararg)
                if is_last_arg {
                    if let LuaExpr::LiteralExpr(lit) = arg {
                        if matches!(lit.get_literal(), Some(emmylua_parser::LuaLiteralToken::Dots(_))) {
                            // Vararg as last argument: use "all out" mode
                            emit(c, Instruction::encode_abc(OpCode::Vararg, target_reg, 0, 0));
                            last_is_vararg_all_out = true;
                            continue;
                        }
                    }
                }

                let arg_reg = compile_expr(c, &arg)?;
                if arg_reg != target_reg {
                    emit_move(c, target_reg, arg_reg);
                }
            }

            // Emit TailCall instruction
            // A = function register, B = num_args + 1 (or 0 if last arg is vararg "all out")
            let num_args = args.len();
            let b_param = if last_is_vararg_all_out {
                0 // B=0: all in (variable number of args from vararg)
            } else {
                (num_args + 1) as u32
            };
            emit(
                c,
                Instruction::encode_abc(OpCode::TailCall, base_reg, b_param, 0),
            );

            return Ok(());
        }
    }

    // Check if last expression is varargs (...) or function call - these can return multiple values
    let last_is_multret = if let Some(last_expr) = exprs.last() {
        matches!(last_expr, LuaExpr::CallExpr(_))
            || matches!(last_expr, LuaExpr::LiteralExpr(lit) if matches!(
                lit.get_literal(),
                Some(emmylua_parser::LuaLiteralToken::Dots(_))
            ))
    } else {
        false
    };

    // Compile expressions to consecutive registers for return
    // Allocate new registers to avoid corrupting local variables
    let num_exprs = exprs.len();
    let base_reg = c.freereg; // Start from the first free register
    
    // Handle last expression specially if it's varargs or call
    if last_is_multret && num_exprs > 0 {
        let last_expr = exprs.last().unwrap();

        // First, compile all expressions except the last
        for (i, expr) in exprs.iter().take(num_exprs - 1).enumerate() {
            alloc_register(c);
            let target_reg = base_reg + i as u32;
            let src_reg = compile_expr(c, expr)?;
            if src_reg != target_reg {
                emit_move(c, target_reg, src_reg);
            }
        }

        if let LuaExpr::LiteralExpr(lit) = last_expr {
            if matches!(
                lit.get_literal(),
                Some(emmylua_parser::LuaLiteralToken::Dots(_))
            ) {
                // Varargs: emit VARARG with B=0 (all out)
                let last_target_reg = base_reg + (num_exprs - 1) as u32;
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Vararg, last_target_reg, 0, 0),
                );
                // Return with B=0 (all out)
                emit(
                    c,
                    Instruction::create_abck(OpCode::Return, base_reg, 0, 0, true),
                );
                return Ok(());
            }
        } else if let LuaExpr::CallExpr(call_expr) = last_expr {
            // Call expression: compile with "all out" mode
            let last_target_reg = base_reg + (num_exprs - 1) as u32;
            compile_call_expr_with_returns_and_dest(c, call_expr, 0, Some(last_target_reg))?;
            // Return with B=0 (all out)
            emit(
                c,
                Instruction::create_abck(OpCode::Return, base_reg, 0, 0, true),
            );
            return Ok(());
        }
    }

    // Normal return with fixed number of values
    // Reserve registers and compile expressions
    for i in 0..num_exprs {
        alloc_register(c);
        let target_reg = base_reg + i as u32;
        let src_reg = compile_expr(c, &exprs[i])?;
        if src_reg != target_reg {
            emit_move(c, target_reg, src_reg);
        }
    }
    
    // Use optimized Return0/Return1 when possible
    if num_exprs == 1 {
        // return single_value - use Return1 optimization
        emit(c, Instruction::encode_abc(OpCode::Return1, base_reg, 0, 0));
    } else {
        // Return instruction: OpCode::Return, A = base_reg, B = num_values + 1, k = 1
        emit(
            c,
            Instruction::create_abck(OpCode::Return, base_reg, (num_exprs + 1) as u32, 0, true),
        );
    }

    Ok(())
}

/// Compile if statement
fn compile_if_stat(c: &mut Compiler, stat: &LuaIfStat) -> Result<(), String> {
    // Structure: if <condition> then <block> [elseif <condition> then <block>]* [else <block>] end
    let mut end_jumps = Vec::new();

    // Check if there are elseif or else clauses
    let elseif_clauses = stat.get_else_if_clause_list().collect::<Vec<_>>();
    let has_else = stat.get_else_clause().is_some();
    let has_branches = !elseif_clauses.is_empty() || has_else;

    // Main if clause
    if let Some(cond) = stat.get_condition_expr() {
        // Check if then-block contains only a single jump (break/goto/return)
        // If so, invert comparison to optimize away the jump instruction
        // BUT: Only if there are no else/elseif clauses (otherwise we need to compile them)
        // NOTE: For break statements, we DON'T invert because the JMP IS the break itself
        let then_body = stat.get_block();
        #[allow(unused_variables)]
        let is_single_break = then_body.as_ref().map_or(false, |b| {
            let stats: Vec<_> = b.get_stats().collect();
            stats.len() == 1 && matches!(stats[0], LuaStat::BreakStat(_))
        });
        
        // Only invert for return statements, not for break
        // DISABLED: This optimization assumes the if statement is at the end of the block
        // But in cases like "if cond then return end; <more code>", we need normal mode
        // TODO: Re-enable this optimization only when if statement is the last statement in the block
        let invert = false;
        /*
        let invert = !has_branches
            && !is_single_break
            && then_body
                .as_ref()
                .map_or(false, |b| is_single_jump_block(b));
        */

        // Try immediate comparison optimization (like GTI, LEI, etc.)
        let next_jump = if let Some(_) = try_compile_immediate_comparison(c, &cond, invert)? {
            // Immediate comparison emitted
            if invert {
                // Inverted mode: comparison skips then-block if condition is TRUE
                // When condition is FALSE, execute then-block directly (no JMP needed)
                // This optimization is only used when then-block is a single jump (return/break)
                // and there are no elseif/else branches
                
                // Compile then block directly
                if let Some(body) = then_body {
                    compile_block(c, &body)?;
                }

                // No elseif/else for inverted single-jump blocks
                return Ok(());
            } else {
                // Normal mode: comparison skips next instruction if FALSE
                // So we emit a JMP to skip the then-block when condition is false
                emit_jump(c, OpCode::Jmp)
            }
        } else {
            // Standard path: compile expression + Test
            let cond_reg = compile_expr(c, &cond)?;
            let test_c = if invert { 1 } else { 0 };
            emit(
                c,
                Instruction::encode_abc(OpCode::Test, cond_reg, 0, test_c),
            );

            if invert {
                // Same inverted optimization logic for Test instruction
                let jump_pos = emit_jump(c, OpCode::Jmp);
                if let Some(body) = then_body {
                    let stats: Vec<_> = body.get_stats().collect();
                    if stats.len() == 1 {
                        match &stats[0] {
                            LuaStat::BreakStat(_) => {
                                c.loop_stack.last_mut().unwrap().break_jumps.push(jump_pos);
                            }
                            LuaStat::ReturnStat(ret_stat) => {
                                compile_return_stat(c, ret_stat)?;
                            }
                            _ => unreachable!(
                                "is_single_jump_block should only return true for break/return"
                            ),
                        }
                    }
                }
                return Ok(());
            }

            emit_jump(c, OpCode::Jmp)
        };

        // Compile then block (only in normal mode, already handled in inverted mode)
        if let Some(body) = then_body {
            compile_block(c, &body)?;
        }

        // Only add jump to end if there are other branches
        if has_branches {
            end_jumps.push(emit_jump(c, OpCode::Jmp));
        }

        // Patch jump to next clause (elseif or else)
        patch_jump(c, next_jump);
    }

    // Handle elseif clauses
    for elseif_clause in elseif_clauses {
        if let Some(cond) = elseif_clause.get_condition_expr() {
            // Try immediate comparison optimization (elseif always uses normal mode)
            let next_jump = if let Some(_) = try_compile_immediate_comparison(c, &cond, false)? {
                emit_jump(c, OpCode::Jmp)
            } else {
                let cond_reg = compile_expr(c, &cond)?;
                emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
                emit_jump(c, OpCode::Jmp)
            };

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

    let cond = stat
        .get_condition_expr()
        .ok_or("while statement missing condition")?;

    // OPTIMIZATION: while true -> infinite loop (no condition check)
    let is_infinite_loop = if let LuaExpr::LiteralExpr(lit) = &cond {
        if let Some(LuaLiteralToken::Bool(b)) = lit.get_literal() {
            b.is_true()
        } else {
            false
        }
    } else {
        false
    };

    let end_jump = if is_infinite_loop {
        // Infinite loop: no condition check, no exit jump
        // Just compile body and jump back
        None
    } else if let Some(_imm_reg) = try_compile_immediate_comparison(c, &cond, false)? {
        // OPTIMIZATION: immediate comparison (e.g., i < 10)
        Some(emit_jump(c, OpCode::Jmp))
    } else {
        // Standard path: compile expression + Test
        let cond_reg = compile_expr(c, &cond)?;
        emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
        Some(emit_jump(c, OpCode::Jmp))
    };

    // Compile body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Jump back to loop start
    let jump_offset = (c.chunk.code.len() - loop_start) as i32 + 1;
    emit(c, Instruction::create_sj(OpCode::Jmp, -jump_offset));

    // Patch end jump (if exists)
    if let Some(jump_pos) = end_jump {
        patch_jump(c, jump_pos);
    }

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
        if let Some(_) = try_compile_immediate_comparison(c, &cond_expr, false)? {
            // Immediate comparison skips if FALSE, so Jmp executes when condition is FALSE
            // This is correct for repeat-until (continue when false)
            let jump_offset = loop_start as i32 - (c.chunk.code.len() as i32 + 1);
            emit(c, Instruction::create_sj(OpCode::Jmp, jump_offset));
        } else {
            // Standard path
            let cond_reg = compile_expr(c, &cond_expr)?;
            emit(c, Instruction::encode_abc(OpCode::Test, cond_reg, 0, 0));
            let jump_offset = loop_start as i32 - (c.chunk.code.len() as i32 + 1);
            emit(c, Instruction::create_sj(OpCode::Jmp, jump_offset));
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
        // Use LOADI for immediate integer 1
        emit(c, Instruction::encode_asbx(OpCode::LoadI, step_reg, 1));
    }

    // Create hidden local variables for the internal control state
    // These prevent the registers from being reused by function calls
    add_local(c, "(for state)".to_string(), base_reg);
    add_local(c, "(for state)".to_string(), limit_reg);
    add_local(c, "(for state)".to_string(), step_reg);

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
    // Bx is the backward jump distance. Since PC is incremented before dispatch,
    // when FORLOOP executes, PC is already forloop_pc+1. To jump back to loop_body_start:
    // (forloop_pc + 1) - Bx = loop_body_start => Bx = forloop_pc + 1 - loop_body_start
    let forloop_offset = forloop_pc + 1 - loop_body_start;
    emit(
        c,
        Instruction::encode_abx(OpCode::ForLoop, base_reg, forloop_offset as u32),
    );

    // Patch FORPREP to jump to FORLOOP (not body)
    // Use unsigned Bx for forward jump distance
    let prep_jump = (forloop_pc as i32) - (forprep_pc as i32) - 1;
    c.chunk.code[forprep_pc] = Instruction::encode_abx(OpCode::ForPrep, base_reg, prep_jump as u32);

    end_loop(c);
    end_scope(c);

    // Free the 4 loop control registers (base, limit, step, var)
    c.freereg = base_reg;

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

    // Update control_var for next iteration (use first return value, which is the new index)
    if !var_regs.is_empty() {
        emit_move(c, control_var_reg, var_regs[0]);
    }

    // Check if first return value is nil (end of iteration)
    let nil_const = add_constant(c, LuaValue::nil());

    // Use EQK to compare with constant nil (k=1: skip if NOT equal)
    // When i==nil, is_equal=TRUE, TRUE!=1 is FALSE, don't skip, execute JMP (exit)
    // When i!=nil, is_equal=FALSE, FALSE!=1 is TRUE, skip JMP (continue loop)
    emit(
        c,
        Instruction::create_abck(OpCode::EqK, var_regs[0], nil_const, 0, true),
    );
    
    // If equal (i.e., first value is nil), exit loop
    let end_jump = emit_jump(c, OpCode::Jmp);

    // Compile loop body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Jump back to loop start
    let jump_offset = (c.chunk.code.len() - loop_start) as i32 + 1;
    emit(c, Instruction::create_sj(OpCode::Jmp, -jump_offset));

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
    let func_reg = c.freereg;
    c.freereg += 1;
    c.nactvar += 1; // Increment active variable count

    c.scope_chain.borrow_mut().locals.push(Local {
        name: func_name.clone(),
        depth: c.scope_depth,
        register: func_reg,
        is_const: false,
        is_to_be_closed: false,
    });
    c.chunk.locals.push(func_name);

    // Compile the closure - use dest=Some(func_reg) to ensure it goes to the correct register
    let closure_reg = compile_closure_expr_to(c, &closure, Some(func_reg), false)?;
    
    // Sanity check: closure should be compiled to func_reg
    debug_assert_eq!(closure_reg, func_reg, "Closure should be compiled to func_reg");

    Ok(())
}
