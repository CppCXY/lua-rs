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
        LuaStat::ForStat(for_node) => compile_generic_for_stat(c, for_node),
        LuaStat::ForRangeStat(for_range_node) => compile_numeric_for_stat(c, for_range_node),
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
pub(crate) fn adjust_assign(
    c: &mut Compiler,
    nvars: i32,
    nexps: i32,
    e: &mut expdesc::ExpDesc,
) {
    use expdesc::ExpKind;
    use exp2reg;

    let needed = nvars - nexps; // extra values needed

    // Check if last expression has multiple returns (call or vararg)
    if matches!(e.kind, ExpKind::VCall | ExpKind::VVararg) {
        let mut extra = needed + 1; // discount last expression itself
        if extra < 0 {
            extra = 0;
        }
        exp2reg::set_returns(c, e, extra); // last exp. provides the difference
    } else {
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
    use super::var::{adjustlocalvars, new_localvar};
    use super::{exp2reg, expr};

    let mut nvars: i32 = 0;
    let mut toclose: i32 = -1; // index of to-be-closed variable (if any)
    let local_defs = local_stat.get_local_name_list();
    // Parse variable names and attributes
    // local name1 [<attr>], name2 [<attr>], ... [= explist]
    // TODO: emmylua_parser API needs investigation
    // Temporarily stub this out until we know the correct API

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
            // Compile all expressions
            let mut last_e = expdesc::ExpDesc::new_void();
            for (i, ex) in exprs.iter().enumerate() {
                last_e = expr::expr(c, ex)?;
                if i < (nexps - 1) as usize {
                    exp2reg::exp2nextreg(c, &mut last_e);
                }
            }
            adjust_assign(c, nvars, nexps, &mut last_e);
            adjustlocalvars(c, nvars as usize);
        }
    } else {
        // Different number of variables and expressions - use adjust_assign
        let mut last_e = expdesc::ExpDesc::new_void();

        if nexps > 0 {
            for (i, ex) in exprs.iter().enumerate() {
                last_e = expr::expr(c, ex)?;
                if i < (nexps - 1) as usize {
                    exp2reg::exp2nextreg(c, &mut last_e);
                }
            }
        }

        adjust_assign(c, nvars, nexps, &mut last_e);
        adjustlocalvars(c, nvars as usize);
    }

    // Handle to-be-closed variable
    if toclose != -1 {
        // Mark the variable as to-be-closed (OP_TBC)
        let level = super::var::reglevel(c, toclose as usize);
        helpers::code_abc(c, crate::lua_vm::OpCode::Tbc, level, 0, 0);
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
        if matches!(
            e.kind,
            expdesc::ExpKind::VCall | expdesc::ExpKind::VVararg
        ) {
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
                if matches!(
                    e.kind,
                    expdesc::ExpKind::VCall | expdesc::ExpKind::VVararg
                ) {
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
    // Break is semantically equivalent to "goto break"
    // Create a jump instruction that will be patched later when we leave the loop
    let pc = helpers::jump(c);
    
    // Find the innermost loop and add this break to its jump list
    if c.loop_stack.is_empty() {
        return Err("break statement not inside a loop".to_string());
    }
    
    // Add jump to the current loop's break list
    let loop_idx = c.loop_stack.len() - 1;
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
        let jf = super::exp2reg::goiffalse(c, &mut v);
        
        super::enter_block(c, false)?;
        if let Some(ref block) = if_stat.get_block() {
            super::compile_statlist(c, block)?;
        }
        super::leave_block(c)?;
        
        // If there are else/elseif clauses, jump over them
        if if_stat.get_else_clause().is_some() || if_stat.get_else_if_clause_list().next().is_some() {
            let jmp = helpers::jump(c) as i32;
            helpers::concat(c, &mut escapelist, jmp);
        }
        
        helpers::patch_to_here(c, jf);
    }
    
    // Compile elseif clauses
    for elseif in if_stat.get_else_if_clause_list() {
        if let Some(ref cond) = elseif.get_condition_expr() {
            let mut v = expr::expr(c, cond)?;
            let jf = super::exp2reg::goiffalse(c, &mut v);
            
            super::enter_block(c, false)?;
            if let Some(ref block) = elseif.get_block() {
                super::compile_statlist(c, block)?;
            }
            super::leave_block(c)?;
            
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
        super::enter_block(c, false)?;
        if let Some(ref block) = else_clause.get_block() {
            super::compile_statlist(c, block)?;
        }
        super::leave_block(c)?;
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
    let cond_expr = while_stat.get_condition_expr()
        .ok_or("while statement missing condition")?;
    let mut v = expr::expr(c, &cond_expr)?;
    
    // Generate conditional jump (jump if false)
    let condexit = super::exp2reg::goiffalse(c, &mut v);
    
    // Enter loop block
    super::enter_block(c, true)?;
    
    // Setup loop info for break statements
    c.loop_stack.push(LoopInfo {
        break_jumps: Vec::new(),
        scope_depth: c.scope_depth,
        first_local_register: helpers::nvarstack(c),
    });
    
    // Compile loop body
    if let Some(ref block) = while_stat.get_block() {
        super::compile_statlist(c, block)?;
    }
    
    // Jump back to condition
    helpers::jump_to(c, whileinit);
    
    // Leave block and patch breaks
    super::leave_block(c)?;
    
    // Patch all break statements to jump here
    if let Some(loop_info) = c.loop_stack.pop() {
        let here = helpers::get_label(c);
        for break_pc in loop_info.break_jumps {
            helpers::fix_jump(c, break_pc, here);
        }
    }
    
    // Patch condition exit to jump here (after loop)
    helpers::patch_to_here(c, condexit);
    
    Ok(())
}

/// Compile repeat statement (对齐repeatstat)
fn compile_repeat_stat(c: &mut Compiler, repeat_stat: &LuaRepeatStat) -> Result<(), String> {
    // repeatstat -> REPEAT block UNTIL cond
    let repeat_init = helpers::get_label(c);
    
    // Enter loop block
    super::enter_block(c, true)?;
    
    // Setup loop info for break statements  
    c.loop_stack.push(LoopInfo {
        break_jumps: Vec::new(),
        scope_depth: c.scope_depth,
        first_local_register: helpers::nvarstack(c),
    });
    
    // Enter inner scope block (for condition variables)
    super::enter_block(c, false)?;
    
    // Compile loop body
    if let Some(ref block) = repeat_stat.get_block() {
        super::compile_statlist(c, block)?;
    }
    
    // Compile condition (can see variables declared in loop body)
    let cond_expr = repeat_stat.get_condition_expr()
        .ok_or("repeat statement missing condition")?;
    let mut v = expr::expr(c, &cond_expr)?;
    let condexit = super::exp2reg::goiftrue(c, &mut v);
    
    // Leave inner scope
    super::leave_block(c)?;
    
    // Check if we need to close upvalues
    // TODO: Handle upvalue closing properly when block.upval is true
    
    // Jump back to start if condition is false
    helpers::patch_list(c, condexit, repeat_init);
    
    // Leave loop block
    super::leave_block(c)?;
    
    // Patch all break statements
    if let Some(loop_info) = c.loop_stack.pop() {
        let here = helpers::get_label(c);
        for break_pc in loop_info.break_jumps {
            helpers::fix_jump(c, break_pc, here);
        }
    }
    
    Ok(())
}

/// Compile generic for statement (对齐forlist)
/// for var1, var2, ... in exp1, exp2, exp3 do block end
fn compile_generic_for_stat(c: &mut Compiler, for_stat: &LuaForStat) -> Result<(), String> {
    use super::var::{adjustlocalvars, new_localvar};
    
    // forlist -> NAME {,NAME} IN explist DO block
    let _base = c.freereg;
    let nvars = 3; // (iter, state, control) - internal control variables
    
    // Create internal control variables
    new_localvar(c, "(for iterator)".to_string())?;
    new_localvar(c, "(for state)".to_string())?;
    new_localvar(c, "(for control)".to_string())?;
    
    // Parse user variables
    let var_name = for_stat.get_var_name()
        .ok_or("generic for missing variable name")?;
    new_localvar(c, var_name.get_name_text().to_string())?;
    let nvars = nvars + 1;
    
    // TODO: Parse additional variables from iterator if multiple vars
    // let mut nvars = nvars + 1;
    // for name in ... { new_localvar(c, name)?; nvars += 1; }
    
    // Compile iterator expressions
    let iter_exprs: Vec<_> = for_stat.get_iter_expr().collect();
    let nexps = iter_exprs.len() as i32;
    
    // Evaluate iterator expressions
    if nexps == 0 {
        return Err("generic for missing iterator expression".to_string());
    }
    
    for (i, iter_expr) in iter_exprs.iter().enumerate() {
        let mut v = expr::expr(c, iter_expr)?;
        if i == nexps as usize - 1 {
            // Last expression can return multiple values
            exp2reg::set_returns(c, &mut v, -1); // LUA_MULTRET
        } else {
            exp2reg::exp2nextreg(c, &mut v);
        }
    }
    
    // Adjust to exactly 3 values (iterator, state, control)
    let mut e = expdesc::ExpDesc::new_void();
    adjust_assign(c, 3, nexps, &mut e);
    
    // Activate loop control variables (iterator, state, control)
    adjustlocalvars(c, 3);
    helpers::reserve_regs(c, 3);
    let base = (_base) as u32;
    
    // Generate TFORPREP instruction - prepare for generic for
    let prep = helpers::code_abx(c, crate::lua_vm::OpCode::TForPrep, base, 0);
    
    // Setup loop block
    super::enter_block(c, false)?;
    adjustlocalvars(c, nvars - 3); // Activate user variables
    helpers::reserve_regs(c, nvars as u32 - 3);
    
    // Compile loop body
    if let Some(ref block) = for_stat.get_block() {
        super::compile_statlist(c, block)?;
    }
    
    // Leave block
    super::leave_block(c)?;
    
    // Fix TFORPREP to jump to after TFORLOOP
    helpers::fix_for_jump(c, prep, helpers::get_label(c), false);
    
    // Generate TFORCALL instruction - call iterator
    helpers::code_abc(c, crate::lua_vm::OpCode::TForCall, base, 0, (nvars - 3) as u32);
    
    // Generate TFORLOOP instruction - check result and loop back
    let endfor = helpers::code_abx(c, crate::lua_vm::OpCode::TForLoop, base, 0);
    
    // Fix TFORLOOP to jump back to right after TFORPREP
    helpers::fix_for_jump(c, endfor, prep + 1, true);
    
    Ok(())
}

/// Compile numeric for statement (对齐fornum)
/// for v = e1, e2 [, e3] do block end
fn compile_numeric_for_stat(c: &mut Compiler, for_range_stat: &LuaForRangeStat) -> Result<(), String> {
    use super::var::{adjustlocalvars, new_localvar};
    
    // fornum -> NAME = exp1,exp1[,exp1] DO block
    let _base = c.freereg;
    
    // Create internal control variables: (index), (limit), (step)
    new_localvar(c, "(for index)".to_string())?;
    new_localvar(c, "(for limit)".to_string())?;
    new_localvar(c, "(for step)".to_string())?;
    
    // Get loop variable name
    let var_names: Vec<_> = for_range_stat.get_var_name_list().collect();
    if var_names.is_empty() {
        return Err("numeric for missing variable name".to_string());
    }
    let varname = var_names[0].get_name_text().to_string();
    new_localvar(c, varname)?;
    
    // Compile initial value, limit, step
    let exprs: Vec<_> = for_range_stat.get_expr_list().collect();
    if exprs.len() < 2 {
        return Err("numeric for requires at least start and end values".to_string());
    }
    
    // Compile start expression
    let mut v = expr::expr(c, &exprs[0])?;
    exp2reg::exp2nextreg(c, &mut v);
    
    // Compile limit expression
    let mut v = expr::expr(c, &exprs[1])?;
    exp2reg::exp2nextreg(c, &mut v);
    
    // Compile step expression (default 1)
    if exprs.len() >= 3 {
        let mut v = expr::expr(c, &exprs[2])?;
        exp2reg::exp2nextreg(c, &mut v);
    } else {
        exp2reg::code_int(c, c.freereg, 1);
        helpers::reserve_regs(c, 1);
    }
    
    // Activate control variables
    adjustlocalvars(c, 3);
    let base = (_base) as u32; // Store base for FORPREP/FORLOOP
    
    // Generate FORPREP instruction - initialize loop and skip if empty
    let prep = helpers::code_abx(c, crate::lua_vm::OpCode::ForPrep, base, 0);
    
    // Enter loop block
    super::enter_block(c, false)?; // Not a loop block for enterblock (variables already created)
    adjustlocalvars(c, 1); // activate loop variable
    helpers::reserve_regs(c, 1);
    
    // Setup loop info for break statements
    c.loop_stack.push(LoopInfo {
        break_jumps: Vec::new(),
        scope_depth: c.scope_depth,
        first_local_register: helpers::nvarstack(c),
    });
    
    // Compile loop body
    if let Some(ref block) = for_range_stat.get_block() {
        super::compile_statlist(c, block)?;
    }
    
    // Leave block
    super::leave_block(c)?;
    
    // Fix FORPREP to jump to after FORLOOP if loop is empty
    helpers::fix_for_jump(c, prep, helpers::get_label(c), false);
    
    // Generate FORLOOP instruction - increment and jump back if not done
    let endfor = helpers::code_abx(c, crate::lua_vm::OpCode::ForLoop, base, 0);
    
    // Fix FORLOOP to jump back to right after FORPREP
    helpers::fix_for_jump(c, endfor, prep + 1, true);
    
    // Patch break statements
    if let Some(loop_info) = c.loop_stack.pop() {
        let here = helpers::get_label(c);
        for break_pc in loop_info.break_jumps {
            helpers::fix_jump(c, break_pc, here);
        }
    }
    
    Ok(())
}

/// Compile do statement (对齐block in DO statement)
fn compile_do_stat(c: &mut Compiler, do_stat: &LuaDoStat) -> Result<(), String> {
    if let Some(ref block) = do_stat.get_block() {
        super::enter_block(c, false)?;
        super::compile_block(c, block)?;
        super::leave_block(c)?;
    }
    Ok(())
}

/// Compile function statement (对齐funcstat)
/// 编译 funcname 中的索引表达式（对齐 funcname 中的 fieldsel 调用链）
/// 处理 t.a.b 这样的嵌套结构
fn compile_func_name_index(c: &mut Compiler, index_expr: &LuaIndexExpr) -> Result<expdesc::ExpDesc, String> {
    // 递归处理前缀
    let mut v = expdesc::ExpDesc::new_void();
    
    if let Some(prefix) = index_expr.get_prefix_expr() {
        match prefix {
            LuaExpr::NameExpr(name_expr) => {
                let name = name_expr.get_name_token()
                    .ok_or("function name prefix missing token")?  
                    .get_name_text()
                    .to_string();
                super::var::singlevar(c, &name, &mut v)?;
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
                LuaIndexKey::Name(name_token) => {
                    name_token.get_name_text().to_string()
                }
                _ => return Err("function name field must be a name".to_string()),
            };
            
            // 创建字段访问（对齐 fieldsel）
            let k_idx = super::helpers::string_k(c, field_name);
            let mut k = expdesc::ExpDesc::new_k(k_idx);
            super::exp2reg::exp2anyregup(c, &mut v);
            super::exp2reg::indexed(c, &mut v, &mut k);
        }
    }
    
    Ok(v)
}

/// function funcname body
fn compile_func_stat(c: &mut Compiler, func: &LuaFuncStat) -> Result<(), String> {
    // funcstat -> FUNCTION funcname body
    
    // Get function name (this is a variable expression)
    let func_name = func.get_func_name()
        .ok_or("function statement missing name")?;
    
    // Parse function name into a variable descriptor
    // ismethod 标记是否为方法定义（使用冒号）
    let mut v = expdesc::ExpDesc::new_void();
    let mut ismethod = false;
    
    match func_name {
        LuaVarExpr::NameExpr(name_expr) => {
            // Simple name: function foo() end
            let name = name_expr.get_name_token()
                .ok_or("function name missing token")?
                .get_name_text()
                .to_string();
            super::var::singlevar(c, &name, &mut v)?;
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
            let prefix = index_expr.get_prefix_expr()
                .ok_or("function name missing prefix")?;
            
            // 递归处理前缀（可能是 t 或者 t.a.b）
            match prefix {
                LuaExpr::NameExpr(name_expr) => {
                    let name = name_expr.get_name_token()
                        .ok_or("function name prefix missing token")?  
                        .get_name_text()
                        .to_string();
                    super::var::singlevar(c, &name, &mut v)?;
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
                        LuaIndexKey::Name(name_token) => {
                            name_token.get_name_text().to_string()
                        }
                        _ => return Err("function name field must be a name".to_string()),
                    };
                    
                    // 创建字段访问（对齐 fieldsel）
                    let k_idx = super::helpers::string_k(c, field_name);
                    let mut k = expdesc::ExpDesc::new_k(k_idx);
                    super::exp2reg::exp2anyregup(c, &mut v);
                    super::exp2reg::indexed(c, &mut v, &mut k);
                }
            }
        }
    }
    
    // Compile function body with ismethod flag
    // 参考 lparser.c 的 body 函数：if (ismethod) new_localvarliteral(ls, "self");
    let closure = func.get_closure()
        .ok_or("function statement missing body")?;
    
    // 调用compile_closure_expr传递ismethod参数（对齐lparser.c的body调用）
    let mut b = expr::compile_closure_expr(c, &closure, ismethod)?;
    
    // Store function in the variable
    // TODO: Check readonly variables
    super::exp2reg::store_var(c, &v, &mut b);
    
    Ok(())
}

/// Compile local function statement (对齐localfunc)
/// local function name() body end
fn compile_local_func_stat(c: &mut Compiler, local_func: &LuaLocalFuncStat) -> Result<(), String> {
    use super::var::{adjustlocalvars, new_localvar};
    
    // Get function name
    let local_name = local_func.get_local_name()
        .ok_or("local function missing name")?;
    let name = local_name.get_name_token()
        .ok_or("local function name missing token")?
        .get_name_text()
        .to_string();
    
    // Create local variable first (before compiling body)
    new_localvar(c, name)?;
    adjustlocalvars(c, 1);
    
    // Compile function body
    let closure = local_func.get_closure()
        .ok_or("local function missing body")?;
    let mut b = expr::expr(c, &LuaExpr::ClosureExpr(closure))?;
    
    // Store in the local variable (which is the last one created)
    let reg = c.freereg - 1;
    super::exp2reg::exp2reg(c, &mut b, reg);
    
    Ok(())
}

/// Compile assignment statement (对齐assignment/restassign)
/// var1, var2, ... = exp1, exp2, ...
fn compile_assign_stat(c: &mut Compiler, assign: &LuaAssignStat) -> Result<(), String> {
    // assignment -> var {, var} = explist
    
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
                let name = name_expr.get_name_token()
                    .ok_or("variable name missing token")?
                    .get_name_text()
                    .to_string();
                super::var::singlevar(c, &name, &mut v)?;
            }
            LuaVarExpr::IndexExpr(_index_expr) => {
                // Table indexing: t[k] or t.k
                // TODO: Implement proper indexed expression parsing
                return Err("Table indexing in assignment not yet implemented".to_string());
            }
        }
        var_descs.push(v);
    }
    
    // Evaluate right-hand side expressions
    let mut expr_descs: Vec<expdesc::ExpDesc> = Vec::new();
    if nexps > 0 {
        for (i, expr) in exprs.iter().enumerate() {
            let mut e = expr::expr(c, expr)?;
            
            if i < nexps as usize - 1 {
                // Not the last expression - discharge to next register
                exp2reg::exp2nextreg(c, &mut e);
            }
            // Last expression handled below
            
            expr_descs.push(e);
        }
    }
    
    // Get the last expression (or create void if no expressions)
    let mut last_expr = if nexps > 0 {
        expr_descs.pop().unwrap()
    } else {
        expdesc::ExpDesc::new_void()
    };
    
    // Adjust last expression to provide the right number of values
    adjust_assign(c, nvars, nexps, &mut last_expr);
    
    // Now perform the assignments in reverse order
    // This is important for cases like: a, b = b, a
    
    // First, store all values in temporary registers if needed
    // For simplicity, we'll assign from left to right
    // The first nvars-1 variables get values from expr_descs
    // The last variable gets the adjusted last_expr
    
    let mut expr_idx = 0;
    for (i, var_desc) in var_descs.iter().enumerate() {
        if i < nexps as usize - 1 {
            // Use evaluated expression
            let mut e = expr_descs[expr_idx].clone();
            super::exp2reg::store_var(c, var_desc, &mut e);
            expr_idx += 1;
        } else if i == nexps as usize - 1 {
            // Use the last (possibly adjusted) expression
            super::exp2reg::store_var(c, var_desc, &mut last_expr);
        } else {
            // No more expressions - assign nil
            let mut nil_expr = expdesc::ExpDesc::new_void();
            nil_expr.kind = expdesc::ExpKind::VNil;
            super::exp2reg::store_var(c, var_desc, &mut nil_expr);
        }
    }
    
    Ok(())
}

/// Compile label statement (对齐labelstat)
/// ::label::
fn compile_label_stat(c: &mut Compiler, label_stat: &LuaLabelStat) -> Result<(), String> {
    // Get label name
    let name = label_stat.get_label_name_token()
        .ok_or("label statement missing name")?
        .get_name_text()
        .to_string();
    
    // Check for duplicate labels in current function
    for existing in &c.labels {
        if existing.name == name && existing.scope_depth == c.scope_depth {
            return Err(format!("Label '{}' already defined", name));
        }
    }
    
    // Create label at current position
    let pc = helpers::get_label(c);
    c.labels.push(Label {
        name: name.clone(),
        position: pc,
        scope_depth: c.scope_depth,
    });
    
    // Resolve any pending gotos to this label
    let mut i = 0;
    while i < c.gotos.len() {
        if c.gotos[i].name == name {
            let goto_info = c.gotos.remove(i);
            // Patch the goto jump to this label
            helpers::patch_list(c, goto_info.jump_position as i32, pc);
            
            // Check for variable scope issues
            // TODO: Track nactvar in GotoInfo if needed for scope checking
        } else {
            i += 1;
        }
    }
    
    Ok(())
}

/// Compile goto statement (对齐gotostat)
/// goto label
fn compile_goto_stat(c: &mut Compiler, goto_stat: &LuaGotoStat) -> Result<(), String> {
    // Get target label name
    let name = goto_stat.get_label_name_token()
        .ok_or("goto statement missing label name")?
        .get_name_text()
        .to_string();
    
    // Generate jump instruction
    let jump_pc = helpers::jump(c);
    
    // Try to find the label (for backward jumps)
    let mut found = false;
    for label in &c.labels {
        if label.name == name {
            // Backward jump - resolve immediately
            helpers::patch_list(c, jump_pc as i32, label.position);
            
            // Check if we need to close upvalues
            // TODO: Track variable count for proper scope checking
            
            found = true;
            break;
        }
    }
    
    if !found {
        // Forward jump - add to pending gotos
        c.gotos.push(GotoInfo {
            name,
            jump_position: jump_pc,
            scope_depth: c.scope_depth,
        });
    }
    
    Ok(())
}
