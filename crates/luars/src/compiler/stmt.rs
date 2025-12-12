// Statement compilation (对齐lparser.c的statement parsing)
use super::*;
use super::helpers;
use emmylua_parser::*;

/// Compile a single statement (对齐statement)
pub(crate) fn statement(c: &mut Compiler, stmt: &LuaStat) -> Result<(), String> {
    c.save_line_info(stmt.get_range());

    match stmt {
        LuaStat::LocalStat(local) => compile_local_stat(c, local),
        LuaStat::ReturnStat(ret) => compile_return_stat(c, ret),
        LuaStat::BreakStat(_) => compile_break_stat(c),
        LuaStat::IfStat(if_node) => compile_if_stat(c, if_node),
        LuaStat::WhileStat(while_node) => compile_while_stat(c, while_node),
        LuaStat::RepeatStat(repeat_node) => compile_repeat_stat(c, repeat_node),
        LuaStat::ForStat(for_node) => compile_for_stat(c, for_node),
        LuaStat::DoStat(do_node) => compile_do_stat(c, do_node),
        LuaStat::FuncStat(func_node) => compile_func_stat(c, func_node),
        LuaStat::AssignStat(assign_node) => compile_assign_stat(c, assign_node),
        LuaStat::LabelStat(_) => {
            // TODO: Implement label
            Ok(())
        }
        LuaStat::GotoStat(_) => {
            // TODO: Implement goto
            Ok(())
        }
        _ => Ok(()), // Empty statement
    }
}

/// Compile local variable declaration (对齐localstat)
fn compile_local_stat(_c: &mut Compiler, _local: &LuaLocalStat) -> Result<(), String> {
    // TODO: Implement local variable declaration
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
        let mut e = super::expr::expr(c, &exprs[0])?;
        
        // Check if it's a multi-return expression (call or vararg)
        if matches!(e.kind, super::expdesc::ExpKind::VCall | super::expdesc::ExpKind::VVararg) {
            super::exp2reg::set_returns(c, &mut e, -1); // Return all values
            nret = -1; // LUA_MULTRET
        } else {
            super::exp2reg::exp2anyreg(c, &mut e);
            nret = 1;
        }
    } else {
        // Multiple return values
        for (i, expr) in exprs.iter().enumerate() {
            let mut e = super::expr::expr(c, expr)?;
            if i == exprs.len() - 1 {
                // Last expression might return multiple values
                if matches!(e.kind, super::expdesc::ExpKind::VCall | super::expdesc::ExpKind::VVararg) {
                    super::exp2reg::set_returns(c, &mut e, -1);
                    nret = -1; // LUA_MULTRET
                } else {
                    super::exp2reg::exp2nextreg(c, &mut e);
                    nret = exprs.len() as i32;
                }
            } else {
                super::exp2reg::exp2nextreg(c, &mut e);
            }
        }
        if nret != -1 {
            nret = exprs.len() as i32;
        }
    }
    
    helpers::ret(c, first, nret);
    Ok(())
}

/// Compile break statement (对齐breakstat)
fn compile_break_stat(_c: &mut Compiler) -> Result<(), String> {
    // TODO: Implement break
    Ok(())
}

/// Compile if statement (对齐ifstat)
fn compile_if_stat(_c: &mut Compiler, _if_stat: &LuaIfStat) -> Result<(), String> {
    // TODO: Implement if
    Ok(())
}

/// Compile while statement (对齐whilestat)
fn compile_while_stat(_c: &mut Compiler, _while_stat: &LuaWhileStat) -> Result<(), String> {
    // TODO: Implement while
    Ok(())
}

/// Compile repeat statement (对齐repeatstat)
fn compile_repeat_stat(_c: &mut Compiler, _repeat: &LuaRepeatStat) -> Result<(), String> {
    // TODO: Implement repeat
    Ok(())
}

/// Compile for statement (对齐forstat)
fn compile_for_stat(_c: &mut Compiler, _for_stat: &LuaForStat) -> Result<(), String> {
    // TODO: Implement for
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
fn compile_func_stat(_c: &mut Compiler, _func: &LuaFuncStat) -> Result<(), String> {
    // TODO: Implement function
    Ok(())
}

/// Compile assignment statement (对齐assignment/restassign)
fn compile_assign_stat(_c: &mut Compiler, _assign: &LuaAssignStat) -> Result<(), String> {
    // TODO: Implement assignment
    Ok(())
}
