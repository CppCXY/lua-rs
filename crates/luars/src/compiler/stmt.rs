// Statement compilation (对齐lparser.c的statement parsing)
use super::helpers;
use super::*;
use emmylua_parser::*;

/// Compile a single statement (对齐statement)
pub(crate) fn statement(c: &mut Compiler, stmt: &LuaStat) -> Result<(), String> {
    c.save_line_info(stmt.get_range());

    match stmt {
        LuaStat::LocalStat(local) => compile_local_stat(c, local),
        LuaStat::LocalFuncStat(local_func) => compile_local_func_stat(c, local_func),
        LuaStat::ReturnStat(ret) => compile_return_stat(c, ret),
        LuaStat::BreakStat(_) => compile_break_stat(c),
        LuaStat::IfStat(if_node) => compile_if_stat(c, if_node),
        LuaStat::WhileStat(while_node) => compile_while_stat(c, while_node),
        LuaStat::RepeatStat(repeat_node) => compile_repeat_stat(c, repeat_node),
        LuaStat::ForStat(for_node) => compile_numeric_for_stat(c, for_node),
        LuaStat::ForRangeStat(for_range_node) => compile_generic_for_stat(c, for_range_node),
        LuaStat::DoStat(do_node) => compile_do_stat(c, do_node),
        LuaStat::FuncStat(func_node) => compile_func_stat(c, func_node),
        LuaStat::AssignStat(assign_node) => compile_assign_stat(c, assign_node),
        LuaStat::LabelStat(label_node) => compile_label_stat(c, label_node),
        LuaStat::GotoStat(goto_node) => compile_goto_stat(c, goto_node),
        LuaStat::CallExprStat(expr_stat) => compile_expr_stat(c, expr_stat),
        LuaStat::EmptyStat(_) => Ok(()), // Empty statement is explicitly handled
        _ => Err(format!("Unimplemented statement type: {:?}", stmt)),
    }
}

/// Adjust assignment - align nvars variables to nexps expressions (对齐adjust_assign)
/// This handles the case where the number of variables doesn't match the number of expressions
pub(crate) fn adjust_assign(c: &mut Compiler, nvars: i32, nexps: i32, e: &mut expdesc::ExpDesc) {
    use exp2reg;
    use expdesc::ExpKind;

    let needed = nvars - nexps; // extra values needed

    // 参考lcode.c:1343-1360 (adjust_assign)
    // Check if last expression has multiple returns (call or vararg)
    if matches!(e.kind, ExpKind::VCall | ExpKind::VVararg) {
        let mut extra = needed + 1; // discount last expression itself
        if extra < 0 {
            extra = 0;
        }
        exp2reg::set_returns(c, e, extra); // last exp. provides the difference
    } else {
        // 参考lparser.c:492-495
        // explist返回时最后一个表达式还未discharge，这里discharge它
        if e.kind != ExpKind::VVoid {
            // at least one expression?
            exp2reg::exp2nextreg(c, e); // close last expression
        }
        if needed > 0 {
            // missing values?
            helpers::nil(c, c.freereg, needed as u32); // complete with nils
        }
    }

    if needed > 0 {
        helpers::reserve_regs(c, needed as u32); // registers for extra values
    } else {
        // adding 'needed' is actually a subtraction
        c.freereg = (c.freereg as i32 + needed) as u32; // remove extra values
    }
}

/// Compile local variable declaration (对齐localstat)
fn compile_local_stat(c: &mut Compiler, local_stat: &LuaLocalStat) -> Result<(), String> {
    use var::{adjustlocalvars, new_localvar};
    use {exp2reg, expr};

    let mut nvars: i32 = 0;
    let mut toclose: i32 = -1; // index of to-be-closed variable (if any)
    let local_defs = local_stat.get_local_name_list();
    // Parse variable names and attributes
    // local name1 [<attr>], name2 [<attr>], ... [= explist]

    for name_def in local_defs {
        let name = name_def
            .get_name_token()
            .unwrap()
            .get_name_text()
            .to_string();

        if name.is_empty() {
            return Err("expected variable name".to_string());
        }

        // Create local variable
        let vidx = new_localvar(c, name)?;

        // Check for attribute (<const> or <close>)
        let (is_const, is_to_be_closed) = {
            let attrib = name_def.get_attrib();
            let mut is_const = false;
            let mut is_to_be_closed = false;
            if let Some(attr) = attrib {
                if attr.is_const() {
                    is_const = true;
                }
                if attr.is_close() {
                    is_to_be_closed = true;
                }
            }
            (is_const, is_to_be_closed)
        };

        // Set attributes in the local variable
        {
            let mut scope = c.scope_chain.borrow_mut();
            if let Some(local) = scope.locals.get_mut(vidx) {
                local.is_const = is_const;
                local.is_to_be_closed = is_to_be_closed;
            }
        }

        // Check for to-be-closed
        if is_to_be_closed {
            if toclose != -1 {
                return Err("multiple to-be-closed variables in local list".to_string());
            }
            // Calculate current nactvar equivalent (number of active locals)
            let scope = c.scope_chain.borrow();
            let nactvar = scope.locals.iter().filter(|l| !l.is_const).count();
            toclose = nactvar as i32 + nvars;
        }

        nvars += 1;
    }

    // Parse initialization expressions if present
    let exprs: Vec<_> = local_stat.get_value_exprs().collect();
    let nexps = exprs.len() as i32;

    if nexps == 0 {
        // No initialization - all variables get nil
        // Special case: const without initializer is compile-time constant nil
        adjustlocalvars(c, nvars as usize);
    } else if nvars == nexps {
        // Equal number of variables and expressions
        // Check if last variable is const and can be compile-time constant
        let scope = c.scope_chain.borrow();
        let vidx = scope.locals.len() - 1;
        let is_last_const = scope.locals.get(vidx).map(|l| l.is_const).unwrap_or(false);
        drop(scope);

        if is_last_const && nexps > 0 {
            // Try to evaluate as compile-time constant
            let mut e = expr::expr(c, &exprs[(nexps - 1) as usize])?;

            // For now, only simple constants can be compile-time (TODO: add luaK_exp2const)
            // Activate all variables
            adjustlocalvars(c, nvars as usize);

            // TODO: Implement compile-time constant optimization
            // This requires luaK_exp2const equivalent
            adjust_assign(c, nvars, nexps, &mut e);
        } else {
            // Compile all expressions (参考lparser.c:1011 explist + lparser.c:1747 localstat)
            // explist会返回最后一个表达式未discharge，adjust_assign会处理它
            let mut last_e = expr::expr(c, &exprs[0])?;
            // 对于后续表达式：先discharge前一个到nextreg，再编译当前的
            for ex in exprs.iter().skip(1) {
                exp2reg::exp2nextreg(c, &mut last_e);
                last_e = expr::expr(c, ex)?;
            }
            // nvars == nexps时，adjust_assign会discharge最后一个表达式（needed=0）
            // 然后adjust freereg（fs->freereg += needed，实际不变）
            adjust_assign(c, nvars, nexps, &mut last_e);
            adjustlocalvars(c, nvars as usize);
        }
    } else {
        // Different number of variables and expressions - use adjust_assign
        // 参考lparser.c:1011 (explist) 和 lparser.c:1747 (localstat)
        let mut last_e = if nexps > 0 {
            // 对齐Lua C: expr直接填充v，不需要先初始化为void
            let mut e = expr::expr(c, &exprs[0])?;
            // 对于后续表达式：先discharge前一个到nextreg，再编译当前的
            for ex in exprs.iter().skip(1) {
                exp2reg::exp2nextreg(c, &mut e);
                e = expr::expr(c, ex)?;
            }
            e
        } else {
            // 对齐Lua C: else { e.k = VVOID; nexps = 0; }
            expdesc::ExpDesc::new_void()
        };

        adjust_assign(c, nvars, nexps, &mut last_e);
        adjustlocalvars(c, nvars as usize);
    }

    // Handle to-be-closed variable
    if toclose != -1 {
        // Mark the variable as to-be-closed (OP_TBC)
        let level = var::reglevel(c, toclose as usize);
        helpers::code_abc(c, OpCode::Tbc, level, 0, 0);
    }

    Ok(())
}

/// Compile return statement (对齐retstat)
fn compile_return_stat(c: &mut Compiler, ret: &LuaReturnStat) -> Result<(), String> {
    let first = helpers::nvarstack(c);
    let mut nret: i32 = 0;

    // Get return expressions and collect them
    let exprs: Vec<_> = ret.get_expr_list().collect();

    if exprs.is_empty() {
        // No return values
        nret = 0;
    } else if exprs.len() == 1 {
        // Single return value
        let mut e = expr::expr(c, &exprs[0])?;

        // Check if it's a multi-return expression (call or vararg)
        if matches!(e.kind, expdesc::ExpKind::VCall | expdesc::ExpKind::VVararg) {
            exp2reg::set_returns(c, &mut e, -1); // Return all values
            nret = -1; // LUA_MULTRET
        } else {
            exp2reg::exp2anyreg(c, &mut e);
            nret = 1;
        }
    } else {
        // Multiple return values
        for (i, expr) in exprs.iter().enumerate() {
            let mut e = expr::expr(c, expr)?;
            if i == exprs.len() - 1 {
                // Last expression might return multiple values
                if matches!(e.kind, expdesc::ExpKind::VCall | expdesc::ExpKind::VVararg) {
                    exp2reg::set_returns(c, &mut e, -1);
                    nret = -1; // LUA_MULTRET
                } else {
                    exp2reg::exp2nextreg(c, &mut e);
                    nret = exprs.len() as i32;
                }
            } else {
                exp2reg::exp2nextreg(c, &mut e);
            }
        }
        if nret != -1 {
            nret = exprs.len() as i32;
        }
    }

    helpers::ret(c, first, nret);
    Ok(())
}

/// Compile expression statement (对齐exprstat)
/// This handles function calls and other expressions used as statements
fn compile_expr_stat(c: &mut Compiler, expr_stat: &LuaCallExprStat) -> Result<(), String> {
    // Get the expression
    if let Some(expr) = expr_stat.get_call_expr() {
        let mut v = expr::expr(c, &LuaExpr::CallExpr(expr))?;

        // Expression statements must be function calls or assignments
        // For calls, we need to set the result count to 0 (discard results)
        if matches!(v.kind, expdesc::ExpKind::VCall) {
            exp2reg::set_returns(c, &mut v, 0); // Discard all return values
        } else {
            // Other expressions as statements are generally no-ops
            // but we still discharge them in case they have side effects
            exp2reg::discharge_vars(c, &mut v);
        }
    }
    Ok(())
}

/// Compile break statement (对齐breakstat)
fn compile_break_stat(c: &mut Compiler) -> Result<(), String> {
    // Break is semantically equivalent to "goto break" (对齐luac breakstat)
    if c.loop_stack.is_empty() {
        return Err("break statement not inside a loop".to_string());
    }

    // Get loop info to check if we need to close variables
    let loop_idx = c.loop_stack.len() - 1;
    let first_local = c.loop_stack[loop_idx].first_local_register as usize;

    // Emit CLOSE for variables that need closing when exiting loop (对齐luac)
    if c.nactvar > first_local {
        // Check if any variables need closing
        let scope = c.scope_chain.borrow();
        let mut needs_close = false;
        for i in first_local..c.nactvar {
            if i < scope.locals.len() && scope.locals[i].is_to_be_closed {
                needs_close = true;
                break;
            }
        }
        drop(scope);

        if needs_close {
            helpers::code_abc(c, OpCode::Close, first_local as u32, 0, 0);
        }
    }

    // Create a jump instruction that will be patched later when we leave the loop
    let pc = helpers::jump(c);

    // Add jump to the current loop's break list
    c.loop_stack[loop_idx].break_jumps.push(pc);

    Ok(())
}

/// Compile if statement (对齐ifstat)
fn compile_if_stat(c: &mut Compiler, if_stat: &LuaIfStat) -> Result<(), String> {
    // ifstat -> IF cond THEN block {ELSEIF cond THEN block} [ELSE block] END
    let mut escapelist = helpers::NO_JUMP;

    // Compile main if condition and block
    if let Some(ref cond) = if_stat.get_condition_expr() {
        let mut v = expr::expr(c, cond)?;
        let jf = exp2reg::goiffalse(c, &mut v);

        enter_block(c, false)?;
        if let Some(ref block) = if_stat.get_block() {
            compile_statlist(c, block)?;
        }
        leave_block(c)?;

        // If there are else/elseif clauses, jump over them
        if if_stat.get_else_clause().is_some() || if_stat.get_else_if_clause_list().next().is_some()
        {
            let jmp = helpers::jump(c) as i32;
            helpers::concat(c, &mut escapelist, jmp);
        }

        helpers::patch_to_here(c, jf);
    }

    // Compile elseif clauses
    for elseif in if_stat.get_else_if_clause_list() {
        if let Some(ref cond) = elseif.get_condition_expr() {
            let mut v = expr::expr(c, cond)?;
            let jf = exp2reg::goiffalse(c, &mut v);

            enter_block(c, false)?;
            if let Some(ref block) = elseif.get_block() {
                compile_statlist(c, block)?;
            }
            leave_block(c)?;

            // Jump over remaining elseif/else
            if if_stat.get_else_clause().is_some() {
                let jmp = helpers::jump(c) as i32;
                helpers::concat(c, &mut escapelist, jmp);
            }

            helpers::patch_to_here(c, jf);
        }
    }

    // Compile else clause
    if let Some(ref else_clause) = if_stat.get_else_clause() {
        enter_block(c, false)?;
        if let Some(ref block) = else_clause.get_block() {
            compile_statlist(c, block)?;
        }
        leave_block(c)?;
    }

    // Patch all escape jumps to here (end of if statement)
    helpers::patch_to_here(c, escapelist);

    Ok(())
}

/// Compile while statement (对齐whilestat)
fn compile_while_stat(c: &mut Compiler, while_stat: &LuaWhileStat) -> Result<(), String> {
    // whilestat -> WHILE cond DO block END
    let whileinit = helpers::get_label(c);

    // Compile condition
    let cond_expr = while_stat
        .get_condition_expr()
        .ok_or("while statement missing condition")?;
    let mut v = expr::expr(c, &cond_expr)?;

    // Generate conditional jump (jump if false)
    let condexit = exp2reg::goiffalse(c, &mut v);

    // Enter loop block
    enter_block(c, true)?;

    // Setup loop info for break statements
    c.loop_stack.push(LoopInfo {
        break_jumps: Vec::new(),
        scope_depth: c.scope_depth,
        first_local_register: helpers::nvarstack(c),
    });

    // Compile loop body
    if let Some(ref block) = while_stat.get_block() {
        compile_statlist(c, block)?;
    }

    // Jump back to condition
    helpers::jump_to(c, whileinit);

    // Leave block (this will handle break jumps automatically)
    leave_block(c)?;

    // Patch condition exit to jump here (after loop)
    helpers::patch_to_here(c, condexit);

    Ok(())
}

/// Compile repeat statement (对齐repeatstat)
fn compile_repeat_stat(c: &mut Compiler, repeat_stat: &LuaRepeatStat) -> Result<(), String> {
    // repeatstat -> REPEAT block UNTIL cond
    let repeat_init = helpers::get_label(c);

    // Enter loop block
    enter_block(c, true)?;

    // Setup loop info for break statements
    c.loop_stack.push(LoopInfo {
        break_jumps: Vec::new(),
        scope_depth: c.scope_depth,
        first_local_register: helpers::nvarstack(c),
    });

    // Enter inner scope block (for condition variables)
    enter_block(c, false)?;

    // Compile loop body
    if let Some(ref block) = repeat_stat.get_block() {
        compile_statlist(c, block)?;
    }

    // Compile condition (can see variables declared in loop body)
    let cond_expr = repeat_stat
        .get_condition_expr()
        .ok_or("repeat statement missing condition")?;
    let mut v = expr::expr(c, &cond_expr)?;
    let condexit = exp2reg::goiftrue(c, &mut v);

    // Leave inner scope
    leave_block(c)?;

    // Check if we need to close upvalues
    // TODO: Handle upvalue closing properly when block.upval is true

    // Jump back to start if condition is false
    helpers::patch_list(c, condexit, repeat_init);

    // Leave loop block (this will handle break jumps automatically)
    leave_block(c)?;

    Ok(())
}

/// Compile generic for statement (对齐forlist)
/// for var1, var2, ... in exp1, exp2, exp3 do block end
fn compile_generic_for_stat(
    c: &mut Compiler,
    for_range_stat: &LuaForRangeStat,
) -> Result<(), String> {
    use var::{adjustlocalvars, new_localvar};

    // 参考lparser.c:1624: enterblock(fs, &bl, 1) - 外层block，用于整个for语句
    enter_block(c, true)?; // isloop=true

    // Setup loop info for break statements (在外层block)
    c.loop_stack.push(LoopInfo {
        break_jumps: Vec::new(),
        scope_depth: c.scope_depth,
        first_local_register: helpers::nvarstack(c),
    });

    // forlist -> NAME {,NAME} IN explist DO block
    // 参考lparser.c:1591 forlist
    let base = c.freereg;

    // Create 4 control variables: gen, state, control, toclose
    // 参考lparser.c:1599-1602
    new_localvar(c, "(for state)".to_string())?;
    new_localvar(c, "(for state)".to_string())?;
    new_localvar(c, "(for state)".to_string())?;
    new_localvar(c, "(for state)".to_string())?;

    // Parse user variables: var1, var2, ...
    // 参考lparser.c:1604-1608
    let var_names: Vec<_> = for_range_stat.get_var_name_list().collect();
    if var_names.is_empty() {
        return Err("generic for missing variable name".to_string());
    }

    let mut nvars = 4; // 4 control variables
    for var_name in var_names {
        new_localvar(c, var_name.get_name_text().to_string())?;
        nvars += 1;
    }

    // Compile iterator expressions: explist
    // 参考lparser.c:1611: adjust_assign(ls, 4, explist(ls, &e), &e)
    let iter_exprs: Vec<_> = for_range_stat.get_expr_list().collect();
    if iter_exprs.is_empty() {
        return Err("generic for missing iterator expression".to_string());
    }

    let mut last_e = expr::expr(c, &iter_exprs[0])?;
    let nexps = iter_exprs.len() as i32;

    for iter_expr in iter_exprs.iter().skip(1) {
        exp2reg::exp2nextreg(c, &mut last_e);
        last_e = expr::expr(c, iter_expr)?;
    }

    // Adjust to exactly 4 values (gen, state, control, toclose)
    adjust_assign(c, 4, nexps, &mut last_e);

    // Activate loop control variables (4 control vars)
    // 参考lparser.c:1612
    adjustlocalvars(c, 4);

    // marktobeclosed(fs) - 标记最后一个控制变量需要关闭
    // 参考lparser.c:1613
    helpers::marktobeclosed(c);

    // luaK_checkstack(fs, 3) - 确保有3个额外寄存器用于调用生成器
    // 参考lparser.c:1614
    helpers::check_stack(c, 3);

    // Generate TFORPREP instruction - prepare for generic for
    let prep = helpers::code_abx(c, OpCode::TForPrep, base, 0);

    // Setup loop block and activate user variables
    // 参考lparser.c:1615: forbody(ls, base, line, nvars - 4, 1)
    enter_block(c, false)?;
    adjustlocalvars(c, nvars - 4); // Activate user variables (总变量数 - 4个控制变量)
    helpers::reserve_regs(c, (nvars - 4) as u32);

    // Compile loop body
    if let Some(ref block) = for_range_stat.get_block() {
        compile_statlist(c, block)?;
    }

    // Leave inner block (isloop=false, so won't pop loop_stack)
    leave_block(c)?;

    // Fix TFORPREP to jump to after TFORLOOP
    helpers::fix_for_jump(c, prep, helpers::get_label(c), false);

    // Generate TFORCALL instruction - call iterator
    // C参数是用户变量数量
    helpers::code_abc(c, OpCode::TForCall, base, 0, (nvars - 4) as u32);

    // Generate TFORLOOP instruction - check result and loop back
    let endfor = helpers::code_abx(c, OpCode::TForLoop, base, 0);

    // Fix TFORLOOP to jump back to right after TFORPREP
    helpers::fix_for_jump(c, endfor, prep + 1, true);

    // 参考lparser.c:1633: leaveblock(fs) - 离开外层block
    // Outer block is isloop=true, so leave_block will pop loop_stack and patch breaks
    leave_block(c)?;

    Ok(())
}

/// Compile numeric for statement (对齐fornum)
/// for v = e1, e2 [, e3] do block end
fn compile_numeric_for_stat(c: &mut Compiler, for_stat: &LuaForStat) -> Result<(), String> {
    use var::{adjustlocalvars, new_localvar};

    // 参考lparser.c:1624: enterblock(fs, &bl, 1) - 外层block，用于整个for语句
    enter_block(c, true)?; // isloop=true

    // Setup loop info for break statements (在外层block)
    c.loop_stack.push(LoopInfo {
        break_jumps: Vec::new(),
        scope_depth: c.scope_depth,
        first_local_register: helpers::nvarstack(c),
    });

    // fornum -> NAME = exp1,exp1[,exp1] DO block
    // 参考lparser.c:1568 fornum
    let base = c.freereg;

    // Create 3 internal control variables
    // 参考lparser.c:1572-1574（注意：源码中重复了3次"(for state)"）
    new_localvar(c, "(for state)".to_string())?;
    new_localvar(c, "(for state)".to_string())?;
    new_localvar(c, "(for state)".to_string())?;

    // Get loop variable name
    // 参考lparser.c:1575
    let var_name = for_stat
        .get_var_name()
        .ok_or("numeric for missing variable name")?;
    new_localvar(c, var_name.get_name_text().to_string())?;

    // Compile expressions: initial, limit, [step]
    // 参考lparser.c:1577-1585
    let exprs: Vec<_> = for_stat.get_iter_expr().collect();
    if exprs.len() < 2 {
        return Err("numeric for requires at least start and end values".to_string());
    }

    // Compile start expression (exp1)
    let mut v = expr::expr(c, &exprs[0])?;
    exp2reg::exp2nextreg(c, &mut v);

    // Compile limit expression (exp1)
    let mut v = expr::expr(c, &exprs[1])?;
    exp2reg::exp2nextreg(c, &mut v);

    // Compile step expression (exp1), default 1 if not provided
    if exprs.len() >= 3 {
        let mut v = expr::expr(c, &exprs[2])?;
        exp2reg::exp2nextreg(c, &mut v);
    } else {
        // 参考lparser.c:1583: luaK_int(fs, fs->freereg, 1)
        exp2reg::code_int(c, c.freereg, 1);
        helpers::reserve_regs(c, 1);
    }

    // Activate control variables
    // 参考lparser.c:1586
    adjustlocalvars(c, 3);

    // Generate FORPREP instruction - initialize loop and skip if empty
    // 参考lparser.c:1587: forbody(ls, base, line, 1, 0)
    let prep = helpers::code_abx(c, OpCode::ForPrep, base, 0);

    // Enter inner loop block
    enter_block(c, false)?;
    adjustlocalvars(c, 1); // activate loop variable (nvars=1 for numeric for)
    helpers::reserve_regs(c, 1);

    // Compile loop body
    if let Some(ref block) = for_stat.get_block() {
        compile_statlist(c, block)?;
    }

    // Leave inner block (isloop=false, so won't pop loop_stack)
    leave_block(c)?;

    // Fix FORPREP to jump to after FORLOOP if loop is empty
    helpers::fix_for_jump(c, prep, helpers::get_label(c), false);

    // Generate FORLOOP instruction - increment and jump back if not done
    let endfor = helpers::code_abx(c, OpCode::ForLoop, base, 0);

    // Fix FORLOOP to jump back to right after FORPREP
    helpers::fix_for_jump(c, endfor, prep + 1, true);

    // 参考lparser.c:1633: leaveblock(fs) - 离开外层block  
    // Outer block is isloop=true, so leave_block will pop loop_stack and patch breaks
    leave_block(c)?;

    Ok(())
}

/// Compile do statement (对齐block in DO statement)
fn compile_do_stat(c: &mut Compiler, do_stat: &LuaDoStat) -> Result<(), String> {
    if let Some(ref block) = do_stat.get_block() {
        enter_block(c, false)?;
        compile_block(c, block)?;
        leave_block(c)?;
    }
    Ok(())
}

/// Compile function statement (对齐funcstat)
/// 编译 funcname 中的索引表达式（对齐 funcname 中的 fieldsel 调用链）
/// 处理 t.a.b 这样的嵌套结构
fn compile_func_name_index(
    c: &mut Compiler,
    index_expr: &LuaIndexExpr,
) -> Result<expdesc::ExpDesc, String> {
    // 递归处理前缀
    let mut v = expdesc::ExpDesc::new_void();

    if let Some(prefix) = index_expr.get_prefix_expr() {
        match prefix {
            LuaExpr::NameExpr(name_expr) => {
                let name = name_expr
                    .get_name_token()
                    .ok_or("function name prefix missing token")?
                    .get_name_text()
                    .to_string();
                var::singlevar(c, &name, &mut v)?;
            }
            LuaExpr::IndexExpr(inner_index) => {
                // 递归处理
                v = compile_func_name_index(c, &inner_index)?;
            }
            _ => return Err("Invalid function name prefix".to_string()),
        }
    }

    // 获取当前字段
    if let Some(index_token) = index_expr.get_index_token() {
        if index_token.is_left_bracket() {
            return Err("function name cannot use [] syntax".to_string());
        }

        if let Some(key) = index_expr.get_index_key() {
            let field_name = match key {
                LuaIndexKey::Name(name_token) => name_token.get_name_text().to_string(),
                _ => return Err("function name field must be a name".to_string()),
            };

            // 创建字段访问（对齐 fieldsel）
            let k_idx = helpers::string_k(c, field_name);
            let mut k = expdesc::ExpDesc::new_k(k_idx);
            exp2reg::exp2anyregup(c, &mut v);
            exp2reg::indexed(c, &mut v, &mut k);
        }
    }

    Ok(v)
}

/// function funcname body
fn compile_func_stat(c: &mut Compiler, func: &LuaFuncStat) -> Result<(), String> {
    // funcstat -> FUNCTION funcname body

    // Get function name (this is a variable expression)
    let func_name = func
        .get_func_name()
        .ok_or("function statement missing name")?;

    // Parse function name into a variable descriptor
    // ismethod 标记是否为方法定义（使用冒号）
    let mut v = expdesc::ExpDesc::new_void();
    let mut ismethod = false;

    match func_name {
        LuaVarExpr::NameExpr(name_expr) => {
            // Simple name: function foo() end
            let name = name_expr
                .get_name_token()
                .ok_or("function name missing token")?
                .get_name_text()
                .to_string();
            var::singlevar(c, &name, &mut v)?;
        }
        LuaVarExpr::IndexExpr(index_expr) => {
            // Table field: function t.foo() end or function t:foo() end
            // 对齐 funcname: NAME {fieldsel} [':' NAME]
            // funcname 只支持点号和最后的冒号，不支持 []

            // 检测最外层是否为冒号（对齐 funcname 中最后的 ':' 检测）
            if let Some(index_token) = index_expr.get_index_token() {
                if index_token.is_colon() {
                    ismethod = true;
                }
            }

            // 获取前缀表达式（必须是一个名字或者另一个索引）
            let prefix = index_expr
                .get_prefix_expr()
                .ok_or("function name missing prefix")?;

            // 递归处理前缀（可能是 t 或者 t.a.b）
            match prefix {
                LuaExpr::NameExpr(name_expr) => {
                    let name = name_expr
                        .get_name_token()
                        .ok_or("function name prefix missing token")?
                        .get_name_text()
                        .to_string();
                    var::singlevar(c, &name, &mut v)?;
                }
                LuaExpr::IndexExpr(inner_index) => {
                    // 递归处理嵌套的索引（如 t.a.b）
                    // 这里需要递归地构建索引链
                    v = compile_func_name_index(c, &inner_index)?;
                }
                _ => return Err("Invalid function name prefix".to_string()),
            }

            // 获取当前这一层的索引（字段名）
            if let Some(index_token) = index_expr.get_index_token() {
                if index_token.is_left_bracket() {
                    return Err("function name cannot use [] syntax".to_string());
                }

                // 点号或冒号后面必须跟一个名字
                if let Some(key) = index_expr.get_index_key() {
                    let field_name = match key {
                        LuaIndexKey::Name(name_token) => name_token.get_name_text().to_string(),
                        _ => return Err("function name field must be a name".to_string()),
                    };

                    // 创建字段访问（对齐 fieldsel）
                    let k_idx = helpers::string_k(c, field_name);
                    let mut k = expdesc::ExpDesc::new_k(k_idx);
                    exp2reg::exp2anyregup(c, &mut v);
                    exp2reg::indexed(c, &mut v, &mut k);
                }
            }
        }
    }

    // Compile function body with ismethod flag
    // 参考 lparser.c 的 body 函数：if (ismethod) new_localvarliteral(ls, "self");
    let closure = func
        .get_closure()
        .ok_or("function statement missing body")?;

    // 调用compile_closure_expr传递ismethod参数（对齐lparser.c的body调用）
    let mut b = expr::compile_closure_expr(c, &closure, ismethod)?;

    // Store function in the variable
    // TODO: Check readonly variables
    exp2reg::store_var(c, &v, &mut b);

    Ok(())
}

/// Compile local function statement (对齐localfunc)
/// local function name() body end
fn compile_local_func_stat(c: &mut Compiler, local_func: &LuaLocalFuncStat) -> Result<(), String> {
    use var::{adjustlocalvars, new_localvar};

    // Get function name
    let local_name = local_func
        .get_local_name()
        .ok_or("local function missing name")?;
    let name = local_name
        .get_name_token()
        .ok_or("local function name missing token")?
        .get_name_text()
        .to_string();

    // Create local variable but don't activate yet (对齐luac localfunc)
    // The variable will be activated after the function is compiled
    new_localvar(c, name)?;

    // Compile function body (this will reserve a register for the closure)
    let closure = local_func
        .get_closure()
        .ok_or("local function missing body")?;
    let b = expr::expr(c, &LuaExpr::ClosureExpr(closure))?;

    // Now activate the local variable (对齐luac: adjustlocalvars after body compilation)
    // This makes the variable point to the register where the closure was placed
    adjustlocalvars(c, 1);

    // The function is already in the correct register (from CLOSURE instruction)
    // No need to move it - just ensure freereg is correct
    debug_assert!(matches!(b.kind, expdesc::ExpKind::VNonReloc));

    Ok(())
}

/// Compile assignment statement (对齐assignment/restassign)
/// var1, var2, ... = exp1, exp2, ...
fn compile_assign_stat(c: &mut Compiler, assign: &LuaAssignStat) -> Result<(), String> {
    // assignment -> var {, var} = explist
    // 参考lparser.c:1486-1524 (restassign) 和 lparser.c:1467-1484 (assignment)

    // Get variables and expressions
    let (vars, exprs) = assign.get_var_and_expr_list();

    if vars.is_empty() {
        return Err("assignment statement missing variables".to_string());
    }

    let nvars = vars.len() as i32;
    let nexps = exprs.len() as i32;

    // Parse all left-hand side variables
    let mut var_descs: Vec<expdesc::ExpDesc> = Vec::new();
    for var_expr in &vars {
        let mut v = expdesc::ExpDesc::new_void();
        match var_expr {
            LuaVarExpr::NameExpr(name_expr) => {
                let name = name_expr
                    .get_name_token()
                    .ok_or("variable name missing token")?
                    .get_name_text()
                    .to_string();
                var::singlevar(c, &name, &mut v)?;
            }
            LuaVarExpr::IndexExpr(index_expr) => {
                // Table indexing: t[k] or t.k (对齐luac suffixedexp)
                v = expr::compile_index_expr(c, index_expr)?;
            }
        }
        var_descs.push(v.clone());
    }

    // 参考lparser.c:1111-1131 (explist): 解析表达式列表
    // 关键：只有非最后的表达式才exp2nextreg，最后一个交给adjust_assign
    let base_reg = c.freereg;  // 记录表达式开始的寄存器
    
    let mut expr_descs: Vec<expdesc::ExpDesc> = Vec::new();
    if nexps > 0 {
        for (i, expr) in exprs.iter().enumerate() {
            let mut e = expr::expr(c, expr)?;

            if i < nexps as usize - 1 {
                // 参考lparser.c:1127-1130
                exp2reg::exp2nextreg(c, &mut e);
            }

            expr_descs.push(e);
        }
    }

    // Get the last expression
    let mut last_expr = if nexps > 0 {
        expr_descs.pop().unwrap()
    } else {
        expdesc::ExpDesc::new_void()
    };

    // 参考lparser.c:1514-1520
    if nexps != nvars {
        adjust_assign(c, nvars, nexps, &mut last_expr);
    } else {
        // 当nexps == nvars时，使用setoneret: 只关闭最后一个表达式
        // 参考lparser.c:1518-1519
        // 注意：这里不能调用exp2reg，因为它会生成MOVE指令
        // 应该使用exp2nextreg，让值自然分配到下一个寄存器（base_reg）
        use exp2reg;
        exp2reg::exp2nextreg(c, &mut last_expr);
    }

    // 现在执行赋值
    // 参考lparser.c:1467-1484 (assignment函数) 和 lparser.c:1481-1484 (storevartop)
    
    // 关键理解：
    // 1. adjust_assign之后，栈上有nvars个值在base_reg开始的连续寄存器中
    // 2. 官方通过递归restassign，每次只处理一个变量
    // 3. 我们必须直接生成指令，不能调用store_var（它会分配新寄存器并破坏freereg）
    
    // 按顺序赋值给变量
    for (i, var_desc) in var_descs.iter().enumerate() {
        let value_reg = base_reg + i as u32;  // 值所在的寄存器
        
        match var_desc.kind {
            expdesc::ExpKind::VLocal => {
                // 局部变量：生成MOVE指令
                if value_reg != var_desc.var.ridx {
                    use super::helpers;
                    helpers::code_abc(
                        c,
                        crate::lua_vm::OpCode::Move,
                        var_desc.var.ridx,
                        value_reg,
                        0,
                    );
                }
            }
            expdesc::ExpKind::VUpval => {
                // Upvalue：生成SETUPVAL指令
                use super::helpers;
                helpers::code_abc(
                    c,
                    crate::lua_vm::OpCode::SetUpval,
                    value_reg,
                    var_desc.info,
                    0,
                );
            }
            expdesc::ExpKind::VIndexUp => {
                // 索引upvalue：生成SETTABUP指令（用于全局变量）
                // _ENV[key] = value
                // SETTABUP A B C k: UpValue[A][K[B]] := RK(C)
                use super::helpers;
                helpers::code_abck(
                    c,
                    crate::lua_vm::OpCode::SetTabUp,
                    var_desc.ind.t,     // A: upvalue索引
                    var_desc.ind.idx,   // B: key（常量索引）
                    value_reg,          // C: value（RK操作数）
                    true,  // k=true表示B是常量索引
                );
            }
            expdesc::ExpKind::VIndexed => {
                // 表索引：生成SETTABLE指令
                // t[k] = value
                use super::helpers;
                helpers::code_abc(
                    c,
                    crate::lua_vm::OpCode::SetTable,
                    var_desc.ind.t,
                    var_desc.ind.idx,
                    value_reg,
                );
            }
            expdesc::ExpKind::VIndexStr => {
                // 字符串索引：生成SETFIELD指令
                // t.field = value
                use super::helpers;
                helpers::code_abc(
                    c,
                    crate::lua_vm::OpCode::SetField,
                    var_desc.ind.t,
                    var_desc.ind.idx,
                    value_reg,
                );
            }
            expdesc::ExpKind::VIndexI => {
                // 整数索引：生成SETI指令
                // t[i] = value
                use super::helpers;
                helpers::code_abc(
                    c,
                    crate::lua_vm::OpCode::SetI,
                    var_desc.ind.t,
                    var_desc.ind.idx,
                    value_reg,
                );
            }
            _ => {
                return Err(format!(
                    "Invalid assignment target: {:?}",
                    var_desc.kind
                ));
            }
        }
    }
    
    // freereg由compile_statlist在语句结束时统一重置为nvarstack
    // 我们不需要在这里修改freereg

    Ok(())
}

/// Compile label statement (对齐labelstat)
/// ::label::
fn compile_label_stat(c: &mut Compiler, label_stat: &LuaLabelStat) -> Result<(), String> {
    // Get label name
    let name = label_stat
        .get_label_name_token()
        .ok_or("label statement missing name")?
        .get_name_text()
        .to_string();

    // Check for duplicate labels in current block (对齐luac checkrepeated)
    if let Some(block) = &c.block {
        let first_label = block.first_label;
        for i in first_label..c.labels.len() {
            if c.labels[i].name == name {
                return Err(format!("label '{}' already defined", name));
            }
        }
    }

    // Create label at current position
    let pc = helpers::get_label(c);
    let nactvar = c.nactvar;
    c.labels.push(Label {
        name: name.clone(),
        position: pc,
        scope_depth: c.scope_depth,
        nactvar,
    });

    // Resolve pending gotos to this label (对齐luac findgotos)
    if let Some(block) = &c.block {
        let first_goto = block.first_goto;
        let mut i = first_goto;
        while i < c.gotos.len() {
            if c.gotos[i].name == name {
                let goto_nactvar = c.gotos[i].nactvar;

                // Check if goto jumps into scope of any variable (对齐luac)
                if goto_nactvar < nactvar {
                    // Get variable name that would be jumped into
                    let scope = c.scope_chain.borrow();
                    let var_name = if goto_nactvar < scope.locals.len() {
                        scope.locals[goto_nactvar].name.clone()
                    } else {
                        "?".to_string()
                    };
                    drop(scope);

                    return Err(format!(
                        "<goto {}> at line ? jumps into the scope of local '{}'",
                        name, var_name
                    ));
                }

                let goto_info = c.gotos.remove(i);
                // Patch the goto jump to this label
                helpers::patch_list(c, goto_info.jump_position as i32, pc);
            } else {
                i += 1;
            }
        }
    }

    Ok(())
}

/// Compile goto statement (对齐gotostat)
/// goto label
fn compile_goto_stat(c: &mut Compiler, goto_stat: &LuaGotoStat) -> Result<(), String> {
    // Get target label name
    let name = goto_stat
        .get_label_name_token()
        .ok_or("goto statement missing label name")?
        .get_name_text()
        .to_string();

    let nactvar = c.nactvar;
    let close = c.needclose;

    // Try to find the label (for backward jumps) (对齐luac findlabel)
    let mut found_label = None;
    for (idx, label) in c.labels.iter().enumerate() {
        if label.name == name {
            found_label = Some(idx);
            break;
        }
    }

    if let Some(label_idx) = found_label {
        // Extract label info before borrowing c mutably
        let label_nactvar = c.labels[label_idx].nactvar;
        let label_position = c.labels[label_idx].position;

        // Backward jump - check scope (对齐luac)
        if nactvar > label_nactvar {
            // Check for to-be-closed variables
            let scope = c.scope_chain.borrow();
            for i in label_nactvar..nactvar {
                if i < scope.locals.len() && scope.locals[i].is_to_be_closed {
                    return Err(format!(
                        "<goto {}> jumps into scope of to-be-closed variable",
                        name
                    ));
                }
            }
        }

        // Emit CLOSE if needed (对齐luac)
        if nactvar > label_nactvar {
            helpers::code_abc(c, OpCode::Close, label_nactvar as u32, 0, 0);
        }

        // Generate jump instruction
        let jump_pc = helpers::jump(c);
        helpers::patch_list(c, jump_pc as i32, label_position);
    } else {
        // Forward jump - add to pending gotos (对齐luac newgotoentry)
        let jump_pc = helpers::jump(c);
        c.gotos.push(GotoInfo {
            name,
            jump_position: jump_pc,
            scope_depth: c.scope_depth,
            nactvar,
            close,
        });
    }

    Ok(())
}
