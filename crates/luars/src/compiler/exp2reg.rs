use super::Compiler;
/// Expression to register conversion functions
/// Mirrors Lua's code generation strategy with expdesc
use super::expdesc::*;
use super::helpers::*;
use crate::lua_value::LuaValue;
use crate::lua_vm::{Instruction, OpCode};

/// Discharge expression variables (convert to concrete value)
/// Lua equivalent: luaK_dischargevars
pub fn discharge_vars(c: &mut Compiler, e: &mut ExpDesc) {
    match e.kind {
        ExpKind::VLocal => {
            // Local variable: convert to VNONRELOC with its register
            e.kind = ExpKind::VNonReloc;
            e.info = e.var.ridx;
        }
        ExpKind::VUpval => {
            // Upvalue: generate GETUPVAL instruction
            let reg = alloc_register(c);
            emit(c, Instruction::encode_abc(OpCode::GetUpval, reg, e.info, 0));
            e.kind = ExpKind::VNonReloc;
            e.info = reg;
        }
        ExpKind::VIndexUp => {
            // Indexed upvalue: generate GETTABUP instruction
            let reg = alloc_register(c);
            emit(
                c,
                Instruction::create_abck(OpCode::GetTabUp, reg, e.ind.t, e.ind.idx, true),
            );
            e.kind = ExpKind::VNonReloc;
            e.info = reg;
        }
        ExpKind::VIndexI => {
            // Integer indexed: generate GETI instruction
            free_register(c, e.ind.t);
            let reg = alloc_register(c);
            emit(
                c,
                Instruction::encode_abc(OpCode::GetI, reg, e.ind.t, e.ind.idx),
            );
            e.kind = ExpKind::VNonReloc;
            e.info = reg;
        }
        ExpKind::VIndexStr => {
            // String indexed: generate GETFIELD instruction
            free_register(c, e.ind.t);
            let reg = alloc_register(c);
            emit(
                c,
                Instruction::create_abck(OpCode::GetField, reg, e.ind.t, e.ind.idx, true),
            );
            e.kind = ExpKind::VNonReloc;
            e.info = reg;
        }
        ExpKind::VIndexed => {
            // General indexed: generate GETTABLE instruction
            free_registers(c, e.ind.t, e.ind.idx);
            let reg = alloc_register(c);
            emit(
                c,
                Instruction::encode_abc(OpCode::GetTable, reg, e.ind.t, e.ind.idx),
            );
            e.kind = ExpKind::VNonReloc;
            e.info = reg;
        }
        ExpKind::VCall | ExpKind::VVararg => {
            // These are already discharged by their generation
            // Just set to VNONRELOC pointing to freereg-1
            if c.freereg > 0 {
                e.kind = ExpKind::VNonReloc;
                e.info = c.freereg - 1;
            }
        }
        _ => {
            // Other kinds don't need discharging
        }
    }
}

/// Ensure expression is in any register
/// Lua equivalent: discharge2anyreg
#[allow(dead_code)]
fn discharge_to_any_reg(c: &mut Compiler, e: &mut ExpDesc) {
    if e.kind != ExpKind::VNonReloc {
        reserve_registers(c, 1);
        discharge_to_reg(c, e, c.freereg - 1);
    }
}

/// Discharge expression to a specific register
/// Lua equivalent: discharge2reg
pub fn discharge_to_reg(c: &mut Compiler, e: &mut ExpDesc, reg: u32) {
    discharge_vars(c, e);

    match e.kind {
        ExpKind::VNil => {
            emit(c, Instruction::encode_abc(OpCode::LoadNil, reg, 0, 0));
        }
        ExpKind::VFalse => {
            emit(c, Instruction::encode_abc(OpCode::LoadFalse, reg, 0, 0));
        }
        ExpKind::VTrue => {
            emit(c, Instruction::encode_abc(OpCode::LoadTrue, reg, 0, 0));
        }
        ExpKind::VK => {
            // Load constant from constant table
            emit_loadk(c, reg, e.info);
        }
        ExpKind::VKInt => {
            // Load integer immediate if in range, otherwise use constant table
            if e.ival >= -(1 << 23) && e.ival < (1 << 23) {
                emit(
                    c,
                    Instruction::encode_asbx(OpCode::LoadI, reg, e.ival as i32),
                );
            } else {
                let const_idx = add_constant_dedup(c, LuaValue::integer(e.ival));
                emit_loadk(c, reg, const_idx);
            }
        }
        ExpKind::VKFlt => {
            // Load float from constant table
            let const_idx = add_constant_dedup(c, LuaValue::number(e.nval));
            emit_loadk(c, reg, const_idx);
        }
        ExpKind::VKStr => {
            // Load string from constant table
            emit_loadk(c, reg, e.info);
        }
        ExpKind::VNonReloc => {
            if e.info != reg {
                emit_move(c, reg, e.info);
            }
        }
        ExpKind::VReloc => {
            // Relocatable expression: patch instruction to use target register
            let pc = e.info as usize;
            if pc < c.chunk.code.len() {
                // Patch the A field of the instruction
                let mut instr = c.chunk.code[pc];
                Instruction::set_a(&mut instr, reg);
                c.chunk.code[pc] = instr;
            }
        }
        _ => {
            // Should not happen if discharge_vars was called
        }
    }

    e.kind = ExpKind::VNonReloc;
    e.info = reg;
}

/// Compile expression to any available register
/// Lua equivalent: luaK_exp2anyreg
pub fn exp_to_any_reg(c: &mut Compiler, e: &mut ExpDesc) -> u32 {
    discharge_vars(c, e);

    if e.kind == ExpKind::VNonReloc {
        // Already in a register
        // Check if the register is NOT a local variable (i.e., it's a temp register)
        // If it's a local (info < nactvar), we MUST NOT reuse it - allocate a new register
        let nactvar = nvarstack(c);
        if e.info >= nactvar {
            // It's a temp register, can return directly
            return e.info;
        }
        // Fall through: it's a local variable, need to copy to new register
    }

    // Need to allocate a new register
    reserve_registers(c, 1);
    discharge_to_reg(c, e, c.freereg - 1);
    e.info
}

/// Compile expression to next available register
/// Lua equivalent: luaK_exp2nextreg
pub fn exp_to_next_reg(c: &mut Compiler, e: &mut ExpDesc) {
    discharge_vars(c, e);
    free_exp(c, e);
    reserve_registers(c, 1);
    exp_to_reg(c, e, c.freereg - 1);
}

/// Compile expression to a specific register
/// Lua equivalent: exp2reg
pub fn exp_to_reg(c: &mut Compiler, e: &mut ExpDesc, reg: u32) {
    discharge_to_reg(c, e, reg);

    if e.kind == ExpKind::VJmp {
        // TODO: Handle jump expressions (for boolean operators)
        // concat_jump_lists(c, &mut e.t, e.info);
    }

    if e.has_jumps() {
        // TODO: Patch jump lists
        // patch_list_to_here(c, e.f);
        // patch_list_to_here(c, e.t);
    }

    e.f = -1;
    e.t = -1;
    e.kind = ExpKind::VNonReloc;
    e.info = reg;
}

/// Free expression if it's in a temporary register
/// Lua equivalent: freeexp
pub fn free_exp(c: &mut Compiler, e: &ExpDesc) {
    if e.kind == ExpKind::VNonReloc {
        free_register(c, e.info);
    }
}

/// Free two expressions
/// Lua equivalent: freeexps
#[allow(dead_code)]
pub fn free_exps(c: &mut Compiler, e1: &ExpDesc, e2: &ExpDesc) {
    let r1 = if e1.kind == ExpKind::VNonReloc {
        e1.info as i32
    } else {
        -1
    };
    let r2 = if e2.kind == ExpKind::VNonReloc {
        e2.info as i32
    } else {
        -1
    };

    if r1 >= 0 && r2 >= 0 {
        free_registers(c, r1 as u32, r2 as u32);
    } else if r1 >= 0 {
        free_register(c, r1 as u32);
    } else if r2 >= 0 {
        free_register(c, r2 as u32);
    }
}

/// Check if expression has jump lists
impl ExpDesc {
    pub fn has_jumps(&self) -> bool {
        self.t != -1 || self.f != -1
    }
}

/// Ensure expression is in a register or upvalue (Aligned with luaK_exp2anyregup)
/// If expression is not VUPVAL or has jumps, convert it to a register
pub fn exp_to_any_reg_up(c: &mut Compiler, e: &mut ExpDesc) {
    if e.kind != ExpKind::VUpval || e.has_jumps() {
        exp_to_any_reg(c, e);
    }
}

/// Store value from expression to a variable
/// Lua equivalent: luaK_storevar
#[allow(dead_code)]
pub fn store_var(c: &mut Compiler, var: &ExpDesc, ex: &mut ExpDesc) {
    match var.kind {
        ExpKind::VLocal => {
            free_exp(c, ex);
            exp_to_reg(c, ex, var.var.ridx);
        }
        ExpKind::VUpval => {
            let e = exp_to_any_reg(c, ex);
            emit(c, Instruction::encode_abc(OpCode::SetUpval, e, var.info, 0));
            free_exp(c, ex);
        }
        ExpKind::VIndexUp => {
            code_abrk(c, OpCode::SetTabUp, var.ind.t, var.ind.idx, ex);
            free_exp(c, ex);
        }
        ExpKind::VIndexI => {
            code_abrk(c, OpCode::SetI, var.ind.t, var.ind.idx, ex);
            free_exp(c, ex);
        }
        ExpKind::VIndexStr => {
            code_abrk(c, OpCode::SetField, var.ind.t, var.ind.idx, ex);
            free_exp(c, ex);
        }
        ExpKind::VIndexed => {
            code_abrk(c, OpCode::SetTable, var.ind.t, var.ind.idx, ex);
            free_exp(c, ex);
        }
        _ => {
            // Should not happen
        }
    }
}

/// Code ABRxK instruction (with RK operand)
/// Lua equivalent: codeABRK
#[allow(dead_code)]
fn code_abrk(c: &mut Compiler, op: OpCode, a: u32, b: u32, ec: &mut ExpDesc) {
    let k = exp_to_rk(c, ec);
    emit(c, Instruction::create_abck(op, a, b, ec.info, k));
}

/// Convert expression to RK operand (register or constant)
/// Lua equivalent: exp2RK
pub fn exp_to_rk(c: &mut Compiler, e: &mut ExpDesc) -> bool {
    match e.kind {
        ExpKind::VTrue | ExpKind::VFalse | ExpKind::VNil => {
            // OFFICIAL LUA: These constants must be added to constant table for RK encoding
            // lcode.c exp2RK always calls boolF/boolT/nilK which add to constant table
            let value = if e.kind == ExpKind::VTrue {
                LuaValue::boolean(true)
            } else if e.kind == ExpKind::VFalse {
                LuaValue::boolean(false)
            } else {
                LuaValue::nil()
            };
            let const_idx = add_constant_dedup(c, value);
            if const_idx <= Instruction::MAX_C {
                e.info = const_idx;
                e.kind = ExpKind::VK;
                return true;
            }
            // If constant table is full, discharge to register
            exp_to_any_reg(c, e);
            false
        }
        ExpKind::VKInt => {
            // Try to fit integer in constant table
            let const_idx = add_constant_dedup(c, LuaValue::integer(e.ival));
            if const_idx <= Instruction::MAX_C {
                e.info = const_idx;
                e.kind = ExpKind::VK;
                return true;
            }
            // Fall through to allocate register
            exp_to_any_reg(c, e);
            false
        }
        ExpKind::VKFlt => {
            let const_idx = add_constant_dedup(c, LuaValue::number(e.nval));
            if const_idx <= Instruction::MAX_C {
                e.info = const_idx;
                e.kind = ExpKind::VK;
                return true;
            }
            exp_to_any_reg(c, e);
            false
        }
        ExpKind::VK => {
            if e.info <= Instruction::MAX_C {
                return true;
            }
            exp_to_any_reg(c, e);
            false
        }
        _ => {
            exp_to_any_reg(c, e);
            false
        }
    }
}
