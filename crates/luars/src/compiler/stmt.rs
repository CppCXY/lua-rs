// Statement compilation

use super::assign::compile_assign_stat_new;
use super::exp2reg::{discharge_vars, exp_to_any_reg};
use super::expdesc::{ExpDesc, ExpKind};
use super::expr::{
    compile_call_expr, compile_call_expr_with_returns_and_dest, compile_expr, compile_expr_desc,
    compile_expr_to, compile_var_expr,
};
use super::{Compiler, Local, helpers::*};
use crate::compiler::compile_block;
use crate::compiler::expr::compile_closure_expr_to;
use crate::lua_vm::{Instruction, OpCode};
use emmylua_parser::{
    BinaryOperator, LuaAstNode, LuaBlock, LuaCallExprStat, LuaDoStat, LuaExpr, LuaForRangeStat,
    LuaForStat, LuaFuncStat, LuaGotoStat, LuaIfStat, LuaLabelStat, LuaLiteralToken, LuaLocalStat,
    LuaRepeatStat, LuaReturnStat, LuaStat, LuaVarExpr, LuaWhileStat,
};

/// Check if an expression is a vararg (...) literal
fn is_vararg_expr(expr: &LuaExpr) -> bool {
    if let LuaExpr::LiteralExpr(lit) = expr {
        matches!(lit.get_literal(), Some(LuaLiteralToken::Dots(_)))
    } else {
        false
    }
}

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

/// Try to compile binary expression as register comparison for control flow
/// This handles comparisons between two registers (e.g., i < n where n is a variable)
/// Returns true if successful (comparison + JMP emitted)
/// The emitted instructions skip the JMP if condition is TRUE (continue loop)
fn try_compile_register_comparison(
    c: &mut Compiler,
    expr: &LuaExpr,
    invert: bool,
) -> Result<bool, String> {
    // Only handle binary comparison expressions
    if let LuaExpr::BinaryExpr(bin_expr) = expr {
        let (left, right) = bin_expr.get_exprs().ok_or("error")?;
        let op = bin_expr.get_op_token().ok_or("error")?;
        let op_kind = op.get_op();

        // Check if this is a comparison operator
        let (opcode, swap) = match op_kind {
            BinaryOperator::OpLt => (OpCode::Lt, false),
            BinaryOperator::OpLe => (OpCode::Le, false),
            BinaryOperator::OpGt => (OpCode::Lt, true), // a > b == b < a
            BinaryOperator::OpGe => (OpCode::Le, true), // a >= b == b <= a
            _ => return Ok(false),
        };

        // Compile both operands
        let left_reg = compile_expr(c, &left)?;
        let right_reg = compile_expr(c, &right)?;

        // Emit comparison instruction
        // k=0: skip next if FALSE (we want to continue if TRUE, so FALSE means exit)
        // For while: if (i < n) is TRUE, continue loop (skip JMP), else execute JMP to exit
        let k = if invert { 1 } else { 0 };
        let (a, b) = if swap {
            (right_reg, left_reg)
        } else {
            (left_reg, right_reg)
        };
        emit(c, Instruction::encode_abc(opcode, a, b, k));

        return Ok(true);
    }

    Ok(false)
}

/// Compile any statement
pub fn compile_stat(c: &mut Compiler, stat: &LuaStat) -> Result<(), String> {
    c.save_line_info(stat.get_range());

    let result = match stat {
        LuaStat::LocalStat(s) => compile_local_stat(c, s),
        // Use NEW assignment logic (aligned with official luaK_storevar)
        LuaStat::AssignStat(s) => compile_assign_stat_new(c, s),
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
    use super::expr::{compile_call_expr_with_returns_and_dest, compile_expr_to};
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
                // VARARG A C: R(A), ..., R(A+C-2) = vararg
                // C = remaining_vars + 1 (or 0 for all)
                let c_value = if remaining_vars == 1 {
                    2
                } else {
                    (remaining_vars + 1) as u32
                };
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Vararg, target_base, 0, c_value),
                );

                // Add all registers
                for i in 0..remaining_vars {
                    regs.push(target_base + i as u32);
                }
            }
            // Check if last expression is a function call (which might return multiple values)
            else if let LuaExpr::CallExpr(call_expr) = last_expr {
                if remaining_vars > 1 {
                    // Multi-return call: pass target_base as dest so results go directly there
                    // OFFICIAL LUA: funcargs() in lparser.c passes expdesc with VLocal/VNonReloc
                    // which tells luaK_storevar/discharge to use that register as base
                    let result_base = compile_call_expr_with_returns_and_dest(
                        c,
                        call_expr,
                        remaining_vars,
                        Some(target_base), // Pass dest to compile results directly into target registers
                    )?;

                    // Verify results are in target registers (should be guaranteed)
                    debug_assert_eq!(result_base, target_base, "Call should place results in target registers");
                    
                    // Add all result registers
                    for i in 0..remaining_vars {
                        regs.push(target_base + i as u32);
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

    // Check if last expression is varargs (...) or function call - these can return multiple values
    // 官方策略：先编译普通return，然后检测是否为tailcall并修改指令
    // (lparser.c L1824-1827: 检测VCALL && nret==1，修改CALL为TAILCALL)
    let last_is_multret = if let Some(last_expr) = exprs.last() {
        matches!(last_expr, LuaExpr::CallExpr(_)) || is_vararg_expr(last_expr)
    } else {
        false
    };

    // Compile expressions to consecutive registers for return
    // 官方策略L1817: first = luaY_nvarstack(fs)
    // 使用活跃变量的寄存器数作为起点，而非freereg
    let num_exprs = exprs.len();
    let first = nvarstack(c); // Official: use nvarstack as starting register

    // Handle last expression specially if it's varargs or call
    if last_is_multret && num_exprs > 0 {
        let last_expr = exprs.last().unwrap();

        // First, compile all expressions except the last directly to target registers
        for (i, expr) in exprs.iter().take(num_exprs - 1).enumerate() {
            let target_reg = first + i as u32;

            // Try to compile expression directly to target register
            let src_reg = compile_expr_to(c, expr, Some(target_reg))?;

            // If expression couldn't be placed in target, emit a MOVE
            if src_reg != target_reg {
                emit_move(c, target_reg, src_reg);
            }

            if target_reg >= c.freereg {
                c.freereg = target_reg + 1;
            }
        }

        if let LuaExpr::LiteralExpr(lit) = last_expr {
            if matches!(
                lit.get_literal(),
                Some(emmylua_parser::LuaLiteralToken::Dots(_))
            ) {
                // Varargs: emit VARARG with B=0 (all out)
                let last_target_reg = first + (num_exprs - 1) as u32;
                emit(
                    c,
                    Instruction::encode_abc(OpCode::Vararg, last_target_reg, 0, 0),
                );
                // Return with B=0 (all out)
                emit(
                    c,
                    Instruction::create_abck(OpCode::Return, first, 0, 0, true),
                );
                return Ok(());
            }
        } else if let LuaExpr::CallExpr(call_expr) = last_expr {
            // Call expression: compile with "all out" mode
            let last_target_reg = first + (num_exprs - 1) as u32;
            compile_call_expr_with_returns_and_dest(
                c,
                call_expr,
                usize::MAX,
                Some(last_target_reg),
            )?;
            // Return with B=0 (all out)
            // NOTE: k flag initially false, will be set by finish_function if needclose=true
            emit(
                c,
                Instruction::create_abck(OpCode::Return, first, 0, 0, false),
            );

            // Tail call optimization (官方lparser.c L1824-1827)
            // If this is a single call expression return, convert CALL to TAILCALL
            if num_exprs == 1 && c.chunk.code.len() >= 2 {
                let call_pc = c.chunk.code.len() - 2; // CALL is before RETURN
                let call_inst_raw = c.chunk.code[call_pc];
                let call_opcode = Instruction::get_opcode(call_inst_raw);

                if call_opcode == OpCode::Call {
                    let call_a = Instruction::get_a(call_inst_raw);
                    if call_a == first {
                        // Patch CALL to TAILCALL
                        let b = Instruction::get_b(call_inst_raw);
                        c.chunk.code[call_pc] =
                            Instruction::encode_abc(OpCode::TailCall, call_a, b, 0);
                        // RETURN already has B=0 (all out), which is correct for TAILCALL
                    }
                }
            }

            return Ok(());
        }
    }

    // Normal return with fixed number of values
    // 官方策略：
    // - 单返回值L1832: first = luaK_exp2anyreg (可复用原寄存器)
    // - 多返回值L1834: luaK_exp2nextreg (必须连续)
    if num_exprs == 1 {
        // 单返回值优化：不传dest，让表达式使用原寄存器
        // 官方L1832: first = luaK_exp2anyreg(fs, &e);
        let actual_reg = compile_expr_to(c, &exprs[0], None)?;

        // return single_value - use Return1 optimization
        // B = nret + 1 = 2, 使用actual_reg直接返回（无需MOVE）
        emit(
            c,
            Instruction::encode_abc(OpCode::Return1, actual_reg, 2, 0),
        );

        // Tail call optimization for single return (官方lparser.c L1824-1827)
        // Check if the single expression is a CallExpr
        let is_single_call = matches!(&exprs[0], LuaExpr::CallExpr(_));
        if is_single_call && c.chunk.code.len() >= 2 {
            let call_pc = c.chunk.code.len() - 2; // CALL is before RETURN1
            let call_inst_raw = c.chunk.code[call_pc];
            let call_opcode = Instruction::get_opcode(call_inst_raw);

            if call_opcode == OpCode::Call {
                // Verify that CALL's A register matches the return register
                let call_a = Instruction::get_a(call_inst_raw);
                if call_a == actual_reg {
                    // Patch CALL to TAILCALL
                    let b = Instruction::get_b(call_inst_raw);
                    c.chunk.code[call_pc] = Instruction::encode_abc(OpCode::TailCall, call_a, b, 0);

                    // Change RETURN1 to RETURN with B=0 (all out)
                    let return_pc = c.chunk.code.len() - 1;
                    c.chunk.code[return_pc] =
                        Instruction::create_abck(OpCode::Return, call_a, 0, 0, false);
                }
            }
        }
    } else if num_exprs == 0 {
        // No return values - use Return0
        emit(c, Instruction::encode_abc(OpCode::Return0, first, 0, 0));
    } else {
        // 多返回值：必须编译到连续寄存器
        // 官方L1834: luaK_exp2nextreg(fs, &e);
        for i in 0..num_exprs {
            let target_reg = first + i as u32;

            // Try to compile expression directly to target register
            let src_reg = compile_expr_to(c, &exprs[i], Some(target_reg))?;

            // If expression couldn't be placed in target, emit a MOVE
            if src_reg != target_reg {
                emit_move(c, target_reg, src_reg);
            }

            // Update freereg to account for this register
            if target_reg >= c.freereg {
                c.freereg = target_reg + 1;
            }
        }

        // Return instruction: OpCode::Return, A = first, B = num_values + 1
        // NOTE: k flag initially false, will be set by finish_function if needclose=true
        emit(
            c,
            Instruction::create_abck(OpCode::Return, first, (num_exprs + 1) as u32, 0, false),
        );
    }

    Ok(())
}

/// Convert ExpDesc to boolean condition with TEST instruction
/// Returns the jump position to patch (jump if condition is FALSE)
/// This function handles the jump lists in ExpDesc (t and f fields)
/// Aligned with official Lua's approach: TEST directly on the source register
fn exp_to_condition(c: &mut Compiler, e: &mut ExpDesc) -> usize {
    discharge_vars(c, e);

    // Determine if TEST should be inverted
    // e.f == -2: marker for inverted simple expression (from NOT)
    // e.f == -1: normal expression (not inverted)
    // e.f >= 0: has actual jump list (from AND/OR/NOT with jumps)
    let test_c = if e.f == -2 || (e.f != -1 && e.f >= 0) {
        1
    } else {
        0
    };

    // Standard case: emit TEST instruction
    match e.kind {
        ExpKind::VNil | ExpKind::VFalse => {
            // Always false - emit unconditional jump
            return emit_jump(c, OpCode::Jmp);
        }
        ExpKind::VTrue | ExpKind::VK | ExpKind::VKInt | ExpKind::VKFlt => {
            // Always true - no jump needed (will be optimized away by patch_jump)
            return emit_jump(c, OpCode::Jmp);
        }
        ExpKind::VLocal => {
            // Local variable: TEST directly on the variable's register
            // NO MOVE needed! This is key for matching official Lua bytecode
            let reg = e.var.ridx;
            emit(c, Instruction::create_abck(OpCode::Test, reg, 0, 0, test_c != 0));
            return emit_jump(c, OpCode::Jmp);
        }
        ExpKind::VNonReloc => {
            // Already in a register: TEST directly
            let reg = e.info;
            emit(c, Instruction::create_abck(OpCode::Test, reg, 0, 0, test_c != 0));
            reset_freereg(c);
            return emit_jump(c, OpCode::Jmp);
        }
        _ => {
            // Other cases: need to put in a register first
            let reg = exp_to_any_reg(c, e);
            emit(c, Instruction::create_abck(OpCode::Test, reg, 0, 0, test_c != 0));
            reset_freereg(c);
            return emit_jump(c, OpCode::Jmp);
        }
    }
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
            // Standard path: compile expression as ExpDesc for boolean optimization
            let mut cond_desc = compile_expr_desc(c, &cond)?;

            // exp_to_condition will handle NOT optimization (swapped jump lists)
            let next_jump = exp_to_condition(c, &mut cond_desc);

            if invert {
                // Inverted mode (currently disabled)
                // Would need different handling here
                unreachable!("Inverted mode is currently disabled");
            }

            next_jump
        };

        // Compile then block
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
                // Use ExpDesc compilation for boolean optimization
                let mut cond_desc = compile_expr_desc(c, &cond)?;
                exp_to_condition(c, &mut cond_desc)
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

    // Begin scope for the loop body (to track locals for CLOSE)
    begin_scope(c);

    // Enter block as loop (for goto/label handling)
    enterblock(c, true);

    // Begin loop - record first_reg for break CLOSE
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

    // Record freereg before condition compilation
    // After condition is tested, we must reset freereg so the condition's
    // temporary register can be reused by the loop body. This is critical
    // for GC: if we don't reset, the condition value stays on the register
    // stack and becomes a GC root, preventing weak table values from being collected.
    let freereg_before_cond = c.freereg;

    let end_jump = if is_infinite_loop {
        // Infinite loop: no condition check, no exit jump
        // Just compile body and jump back
        None
    } else if let Some(_imm_reg) = try_compile_immediate_comparison(c, &cond, false)? {
        // OPTIMIZATION: immediate comparison with constant (e.g., i < 10)
        Some(emit_jump(c, OpCode::Jmp))
    } else if try_compile_register_comparison(c, &cond, false)? {
        // OPTIMIZATION: register comparison (e.g., i < n)
        Some(emit_jump(c, OpCode::Jmp))
    } else {
        // Standard path: use ExpDesc for boolean optimization
        let mut cond_desc = compile_expr_desc(c, &cond)?;
        Some(exp_to_condition(c, &mut cond_desc))
    };

    // Reset freereg after condition test to release temporary registers
    // This matches official Lua's behavior: freeexp(fs, e) after condition
    c.freereg = freereg_before_cond;

    // Compile body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Before jumping back, emit CLOSE if any local in the loop body was captured
    {
        let loop_info = c.loop_stack.last().unwrap();
        let loop_scope_depth = loop_info.scope_depth;
        let first_reg = loop_info.first_local_register;

        let scope = c.scope_chain.borrow();
        let mut min_close_reg: Option<u32> = None;
        for local in scope.locals.iter().rev() {
            if local.depth < loop_scope_depth {
                break;
            }
            if local.needs_close && local.register >= first_reg {
                min_close_reg = Some(match min_close_reg {
                    None => local.register,
                    Some(min_reg) => min_reg.min(local.register),
                });
            }
        }
        drop(scope);
        if let Some(reg) = min_close_reg {
            emit(c, Instruction::encode_abc(OpCode::Close, reg, 0, 0));
        }
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

    // Leave block (for goto/label handling)
    leaveblock(c);

    // End scope
    end_scope(c);

    Ok(())
}

/// Compile repeat-until loop
fn compile_repeat_stat(c: &mut Compiler, stat: &LuaRepeatStat) -> Result<(), String> {
    // Structure: repeat <block> until <condition>

    // Enter block as loop (for goto/label handling)
    enterblock(c, true);

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
        } else if try_compile_register_comparison(c, &cond_expr, false)? {
            // OPTIMIZATION: register comparison (e.g., i >= n)
            let jump_offset = loop_start as i32 - (c.chunk.code.len() as i32 + 1);
            emit(c, Instruction::create_sj(OpCode::Jmp, jump_offset));
        } else {
            // Standard path
            let cond_reg = compile_expr(c, &cond_expr)?;
            emit(c, Instruction::create_abck(OpCode::Test, cond_reg, 0, 0, false));
            let jump_offset = loop_start as i32 - (c.chunk.code.len() as i32 + 1);
            emit(c, Instruction::create_sj(OpCode::Jmp, jump_offset));
        }
    }

    // End loop (patches all break statements)
    end_loop(c);

    // Leave block (for goto/label handling)
    leaveblock(c);

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

    // Enter block as loop (for goto/label handling)
    enterblock(c, true);

    // Begin loop with var_reg as first register, so break can close it
    // Using var_reg instead of c.freereg because var_reg is the loop variable
    begin_loop_with_register(c, var_reg);

    // The loop variable is at R(base+3)
    add_local(c, var_name, var_reg);

    // Loop body starts here
    let loop_body_start = c.chunk.code.len();

    // Compile loop body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Before FORLOOP, emit CLOSE if any local in the loop body was captured
    // Find the minimum register of captured locals in the current scope (loop body)
    {
        let scope = c.scope_chain.borrow();
        let mut min_close_reg: Option<u32> = None;
        for local in scope.locals.iter().rev() {
            if local.depth < c.scope_depth {
                break; // Only check current scope (loop body)
            }
            if local.depth == c.scope_depth && local.needs_close {
                min_close_reg = Some(match min_close_reg {
                    None => local.register,
                    Some(min_reg) => min_reg.min(local.register),
                });
            }
        }
        drop(scope);
        if let Some(reg) = min_close_reg {
            emit(c, Instruction::encode_abc(OpCode::Close, reg, 0, 0));
        }
    }

    // FORLOOP comes AFTER the body (and CLOSE if needed)
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

    // Leave block (for goto/label handling)
    leaveblock(c);

    end_scope(c);

    // Free the 4 loop control registers (base, limit, step, var)
    // and remove the 3 hidden state variables from the local scope
    c.freereg = base_reg;

    // Remove the 3 state variables from locals (they were added before begin_scope)
    // Find and remove them by checking their registers
    let mut removed = 0;
    c.scope_chain.borrow_mut().locals.retain(|l| {
        if l.register >= base_reg && l.register < base_reg + 3 && l.name == "(for state)" {
            if !l.is_const {
                removed += 1;
            }
            false
        } else {
            true
        }
    });
    c.nactvar = c.nactvar.saturating_sub(removed);

    Ok(())
}

/// Compile generic for loop using TFORPREP/TFORCALL/TFORLOOP instructions
///
/// Lua 5.4 for-in register layout:
/// R[A]   = iter_func (f)
/// R[A+1] = state (s)
/// R[A+2] = control variable (var, updated by TFORLOOP)
/// R[A+3] = to-be-closed variable (copy of state, for cleanup)
/// R[A+4] = first loop variable (var1)
/// R[A+5] = second loop variable (var2)
/// ...
///
/// Instruction sequence:
/// TFORPREP A Bx   -> sets up R[A+3] = R[A+1], jumps forward to TFORCALL
/// (loop body)
/// TFORCALL A C    -> R[A+4], ..., R[A+3+C] := R[A](R[A+1], R[A+2])
/// TFORLOOP A Bx   -> if R[A+4] ~= nil then { R[A+2]=R[A+4]; pc -= Bx }
fn compile_for_range_stat(c: &mut Compiler, stat: &LuaForRangeStat) -> Result<(), String> {
    use super::expr::compile_call_expr_with_returns;
    use emmylua_parser::LuaExpr;

    // Get loop variable names
    let var_names = stat
        .get_var_name_list()
        .map(|name| name.get_name_text().to_string())
        .collect::<Vec<_>>();

    if var_names.is_empty() {
        return Err("for-in loop requires at least one variable".to_string());
    }

    // Get iterator expressions
    let iter_exprs = stat.get_expr_list().collect::<Vec<_>>();
    if iter_exprs.is_empty() {
        return Err("for-in loop requires iterator expression".to_string());
    }

    // FIRST: Compile iterator expressions BEFORE allocating the for-in block
    // This prevents the call results from overlapping with loop variables
    let base = c.freereg;

    // Compile iterator expressions to get (iter_func, state, control_var, to-be-closed) at base
    // Lua 5.4 for-in needs 4 control slots: iterator, state, control, closing value
    if iter_exprs.len() == 1 {
        if let LuaExpr::CallExpr(call_expr) = &iter_exprs[0] {
            // Single call expression - returns (iter_func, state, control_var, closing)
            // Compile directly to base register, expecting 4 return values
            let result_reg = compile_call_expr_with_returns(c, call_expr, 4)?;
            // Move results to base if not already there
            if result_reg != base {
                emit_move(c, base, result_reg);
                emit_move(c, base + 1, result_reg + 1);
                emit_move(c, base + 2, result_reg + 2);
                emit_move(c, base + 3, result_reg + 3);
            }
        } else {
            // Single non-call expression - use as iterator function, state and control are nil
            let func_reg = compile_expr(c, &iter_exprs[0])?;
            emit_move(c, base, func_reg);
            emit_load_nil(c, base + 1);
            emit_load_nil(c, base + 2);
            emit_load_nil(c, base + 3);
        }
    } else {
        // Multiple expressions: iter_func, state, control_var, closing_value
        for (i, expr) in iter_exprs.iter().enumerate().take(4) {
            let reg = compile_expr(c, expr)?;
            if reg != base + i as u32 {
                emit_move(c, base + i as u32, reg);
            }
        }
        // Fill missing with nil (up to 4 control slots)
        for i in iter_exprs.len()..4 {
            emit_load_nil(c, base + i as u32);
        }
    }

    // NOW set freereg to allocate the control block properly
    // The first 3 registers (iter_func, state, control) are already at base
    // We need to mark them as used and allocate the to-be-closed slot
    c.freereg = base + 4; // base + 3 slots already used + 1 for to-be-closed

    // Begin scope for loop variables
    begin_scope(c);

    // Enter block as loop (for goto/label handling)
    enterblock(c, true);

    // Register the iterator's hidden variables as internal locals
    add_local(c, "(for state)".to_string(), base);
    add_local(c, "(for state)".to_string(), base + 1);
    add_local(c, "(for state)".to_string(), base + 2);
    add_local(c, "(for state)".to_string(), base + 3);

    // Allocate registers for loop variables (starting at base+4)
    for var_name in &var_names {
        let reg = alloc_register(c);
        add_local(c, var_name.clone(), reg);
    }

    // Number of loop variables (C parameter for TFORCALL)
    let num_vars = var_names.len();

    // Emit TFORPREP: creates to-be-closed and jumps forward to TFORCALL
    let tforprep_pc = c.chunk.code.len();
    emit(c, Instruction::encode_abx(OpCode::TForPrep, base, 0)); // Will patch later

    begin_loop(c);

    // Loop body starts here
    let loop_body_start = c.chunk.code.len();

    // Compile loop body
    if let Some(body) = stat.get_block() {
        compile_block(c, &body)?;
    }

    // Ensure max_stack_size is large enough to protect vararg area
    // When a function call happens inside the loop, the called function may use
    // registers beyond max_stack_size. If vararg is stored at max_stack_size,
    // it can be overwritten. Add extra space to prevent this.
    // This is a workaround - the proper fix would be in VARARGPREP to use
    // a larger offset or track the maximum call depth.
    let safe_stack_size = c.freereg as usize + 4; // Add 4 extra slots for safety
    if safe_stack_size > c.chunk.max_stack_size {
        c.chunk.max_stack_size = safe_stack_size;
    }

    // TFORCALL comes after the loop body
    let tforcall_pc = c.chunk.code.len();
    // TFORCALL A C: R[A+4], ..., R[A+3+C] := R[A](R[A+1], R[A+2])
    emit(
        c,
        Instruction::encode_abc(OpCode::TForCall, base, 0, num_vars as u32),
    );

    // TFORLOOP: if R[A+4] ~= nil then { R[A+2]=R[A+4]; pc -= Bx }
    let tforloop_pc = c.chunk.code.len();
    // Jump back to loop body start
    // When TFORLOOP executes, PC is at tforloop_pc. After execution, PC increments.
    // If continuing: PC = PC - Bx (before increment), then PC++.
    // So we need: (tforloop_pc - Bx + 1) = loop_body_start
    // Bx = tforloop_pc + 1 - loop_body_start
    let tforloop_jump = tforloop_pc + 1 - loop_body_start;
    emit(
        c,
        Instruction::encode_abx(OpCode::TForLoop, base, tforloop_jump as u32),
    );

    // Patch TFORPREP to jump to TFORCALL
    // TFORPREP jumps forward by Bx
    let tforprep_jump = tforcall_pc - tforprep_pc - 1;
    c.chunk.code[tforprep_pc] =
        Instruction::encode_abx(OpCode::TForPrep, base, tforprep_jump as u32);

    end_loop(c);

    // Leave block (for goto/label handling)
    leaveblock(c);

    end_scope(c);

    // Free the loop control registers
    c.freereg = base;

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

    let func_name = func_name_var_expr.get_text();
    // Compile the closure to get function value
    let func_reg = compile_closure_expr_to(c, &closure, None, is_colon, Some(func_name.clone()))?;

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
        needs_close: false,
    });
    c.chunk.locals.push(func_name.clone());

    // Compile the closure - use dest=Some(func_reg) to ensure it goes to the correct register
    let closure_reg = compile_closure_expr_to(c, &closure, Some(func_reg), false, Some(func_name))?;

    // Sanity check: closure should be compiled to func_reg
    debug_assert_eq!(
        closure_reg, func_reg,
        "Closure should be compiled to func_reg"
    );

    Ok(())
}
