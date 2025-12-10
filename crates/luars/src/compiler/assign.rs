// Assignment statement helpers - Aligned with Lua 5.4 lparser.c::restassign
// This module implements the official assignment logic using ExpDesc

use super::Compiler;
use super::expdesc::*;
use super::exp2reg::*;
use super::expr::compile_expr_desc;
use super::helpers::*;
use crate::lua_vm::{Instruction, OpCode};
use emmylua_parser::{LuaExpr, LuaVarExpr, LuaAssignStat};

/// Compile assignment statement (Aligned with Lua 5.4 lparser.c::restassign)
/// Format: <var_list> = <expr_list>
/// Official pattern: compile left-values to ExpDesc, then luaK_storevar generates SET instructions
pub fn compile_assign_stat_new(c: &mut Compiler, stat: &LuaAssignStat) -> Result<(), String> {
    // Get variable list and expression list
    let (vars, exprs) = stat.get_var_and_expr_list();
    
    if vars.is_empty() {
        return Ok(());
    }

    let nvars = vars.len();
    let nexprs = exprs.len();

    // CASE 1: Single variable, single expression (most common)
    if nvars == 1 && nexprs == 1 {
        let var_expr = &vars[0];
        let value_expr = &exprs[0];

        // Compile left-value to ExpDesc (preserves VLOCAL, VINDEXSTR, etc.)
        let var_desc = compile_suffixed_expr_desc(c, var_expr)?;
        
        // Compile value expression to ExpDesc
        let mut value_desc = compile_expr_desc(c, value_expr)?;
        
        // Store using official luaK_storevar pattern
        store_var(c, &var_desc, &mut value_desc)?;
        return Ok(());
    }

    // CASE 2: Multiple variables or expressions
    // Compile all left-values to ExpDesc (NO instruction emission yet)
    let mut var_descs = Vec::with_capacity(nvars);
    for var_expr in &vars {
        let desc = compile_suffixed_expr_desc(c, var_expr)?;
        var_descs.push(desc);
    }

    // Compile all value expressions to sequential registers
    let base_reg = c.freereg;
    
    for (i, expr) in exprs.iter().enumerate() {
        let mut desc = compile_expr_desc(c, expr)?;
        
        if i == nexprs - 1 {
            // Last expression: might produce multiple values
            // Adjust to produce exactly (nvars - i) values
            let needed = nvars - i;
            if needed > 1 {
                // Need multiple values from last expression
                adjust_mult_assign(c, &mut desc, needed)?;
            } else {
                // Just need one value
                exp_to_next_reg(c, &mut desc);
            }
        } else {
            // Not last expression: convert to single register
            exp_to_next_reg(c, &mut desc);
        }
    }
    
    // Adjust if expressions produce fewer values than variables
    if nexprs < nvars {
        adjust_mult_assign_nil(c, nvars - nexprs);
    }
    
    // Store each value to its target variable
    // Values are in sequential registers starting at base_reg
    for (i, var_desc) in var_descs.iter().enumerate() {
        let value_reg = base_reg + i as u32;
        let mut value_desc = ExpDesc::new_nonreloc(value_reg);
        store_var(c, var_desc, &mut value_desc)?;
    }
    
    // Free temporary registers
    c.freereg = base_reg;
    Ok(())
}

/// Store value to variable (Aligned with luaK_storevar in lcode.c)
/// This is the CRITICAL function that switches on ExpDesc kind
pub fn store_var(c: &mut Compiler, var: &ExpDesc, value: &mut ExpDesc) -> Result<(), String> {
    match var.kind {
        ExpKind::VLocal => {
            // Local variable: compile value directly into target register
            let target_reg = var.var.ridx;
            let value_reg = exp_to_any_reg(c, value);
            if value_reg != target_reg {
                emit(c, Instruction::encode_abc(OpCode::Move, target_reg, value_reg, 0));
                free_register(c, value_reg);
            }
        }
        ExpKind::VUpval => {
            // Upvalue: SETUPVAL A B where A=value_reg, B=upvalue_index
            let value_reg = exp_to_any_reg(c, value);
            emit(c, Instruction::encode_abc(OpCode::SetUpval, value_reg, var.info, 0));
            free_register(c, value_reg);
        }
        ExpKind::VIndexUp => {
            // Global variable: SETTABUP A B C where A=_ENV, B=key, C=value
            let value_k = exp_to_rk(c, value);  // Returns bool: true if constant
            let value_rk = value.info;  // After exp_to_rk, info contains the RK value
            emit(c, Instruction::create_abck(OpCode::SetTabUp, var.ind.t, var.ind.idx, value_rk, value_k));
        }
        ExpKind::VIndexI => {
            // Table with integer index: SETI A B C where A=table, B=int_key, C=value
            let value_k = exp_to_rk(c, value);
            let value_rk = value.info;
            emit(c, Instruction::create_abck(OpCode::SetI, var.ind.t, var.ind.idx, value_rk, value_k));
        }
        ExpKind::VIndexStr => {
            // Table with string key: SETFIELD A B C where A=table, B=str_key, C=value
            let value_k = exp_to_rk(c, value);
            let value_rk = value.info;
            emit(c, Instruction::create_abck(OpCode::SetField, var.ind.t, var.ind.idx, value_rk, value_k));
        }
        ExpKind::VIndexed => {
            // Table with general key: SETTABLE A B C where A=table, B=key, C=value
            let value_k = exp_to_rk(c, value);
            let value_rk = value.info;
            emit(c, Instruction::create_abck(OpCode::SetTable, var.ind.t, var.ind.idx, value_rk, value_k));
        }
        _ => {
            return Err(format!("Cannot assign to expression of kind {:?}", var.kind));
        }
    }
    Ok(())
}

/// Compile suffixed expression to ExpDesc (for left-values in assignments)
/// This is like suffixedexp() in lparser.c - returns ExpDesc without generating GET instructions
pub fn compile_suffixed_expr_desc(c: &mut Compiler, expr: &LuaVarExpr) -> Result<ExpDesc, String> {
    match expr {
        LuaVarExpr::NameExpr(name_expr) => {
            // Simple variable reference
            let lua_expr = LuaExpr::NameExpr(name_expr.clone());
            compile_expr_desc(c, &lua_expr)
        }
        LuaVarExpr::IndexExpr(index_expr) => {
            // Table indexing: t[key] or t.field
            let lua_expr = LuaExpr::IndexExpr(index_expr.clone());
            compile_expr_desc(c, &lua_expr)
        }
    }
}

/// Adjust last expression in multiple assignment to produce multiple values
fn adjust_mult_assign(c: &mut Compiler, desc: &mut ExpDesc, nvars: usize) -> Result<(), String> {
    match desc.kind {
        ExpKind::VCall => {
            // Function call: adjust C field to produce nvars results
            // CALL A B C: R(A), ..., R(A+C-2) = R(A)(R(A+1), ..., R(A+B-1))
            // Set C = nvars + 1
            let pc = desc.info as usize;
            Instruction::set_c(&mut c.chunk.code[pc], (nvars + 1) as u32);
            c.freereg += nvars as u32;
        }
        ExpKind::VVararg => {
            // Vararg: adjust C field to produce nvars results
            // VARARG A B C: R(A), ..., R(A+C-2) = vararg
            // Set C = nvars + 1
            let pc = desc.info as usize;
            Instruction::set_c(&mut c.chunk.code[pc], (nvars + 1) as u32);
            c.freereg += nvars as u32;
        }
        _ => {
            // Other expressions: convert to register and fill rest with nil
            exp_to_next_reg(c, desc);
            adjust_mult_assign_nil(c, nvars - 1);
        }
    }
    Ok(())
}

/// Fill registers with nil values (for missing expressions in assignment)
fn adjust_mult_assign_nil(c: &mut Compiler, count: usize) {
    if count > 0 {
        let start_reg = c.freereg;
        // LOADNIL A B: R(A), ..., R(A+B) := nil
        // For count=1, B=0; for count=2, B=1, etc.
        emit(c, Instruction::encode_abc(OpCode::LoadNil, start_reg, (count - 1) as u32, 0));
        c.freereg += count as u32;
    }
}
