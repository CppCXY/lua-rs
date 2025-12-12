// Expression discharge and register allocation (对齐lcode.c的discharge系列函数)
use super::expdesc::*;
use super::helpers::*;
use super::*;
use crate::lua_vm::{Instruction, OpCode};

/// Discharge variables to their values (对齐luaK_dischargevars)
pub(crate) fn discharge_vars(c: &mut Compiler, e: &mut ExpDesc) {
    match e.kind {
        ExpKind::VLocal => {
            e.info = e.var.ridx;
            e.kind = ExpKind::VNonReloc;
        }
        ExpKind::VUpval => {
            e.info = code_abc(c, OpCode::GetUpval, 0, e.info, 0) as u32;
            e.kind = ExpKind::VReloc;
        }
        ExpKind::VIndexUp => {
            e.info = code_abc(c, OpCode::GetTabUp, 0, e.ind.t, e.ind.idx) as u32;
            e.kind = ExpKind::VReloc;
        }
        ExpKind::VIndexI => {
            free_reg(c, e.ind.t);
            e.info = code_abc(c, OpCode::GetI, 0, e.ind.t, e.ind.idx) as u32;
            e.kind = ExpKind::VReloc;
        }
        ExpKind::VIndexStr => {
            free_reg(c, e.ind.t);
            e.info = code_abc(c, OpCode::GetField, 0, e.ind.t, e.ind.idx) as u32;
            e.kind = ExpKind::VReloc;
        }
        ExpKind::VIndexed => {
            free_reg(c, e.ind.t);
            free_reg(c, e.ind.idx);
            e.info = code_abc(c, OpCode::GetTable, 0, e.ind.t, e.ind.idx) as u32;
            e.kind = ExpKind::VReloc;
        }
        ExpKind::VCall | ExpKind::VVararg => {
            set_one_ret(c, e);
        }
        _ => {}
    }
}

/// Discharge expression to a specific register (对齐discharge2reg)
fn discharge2reg(c: &mut Compiler, e: &mut ExpDesc, reg: u32) {
    discharge_vars(c, e);
    match e.kind {
        ExpKind::VNil => nil(c, reg, 1),
        ExpKind::VFalse => { code_abc(c, OpCode::LoadFalse, reg, 0, 0); }
        ExpKind::VTrue => { code_abc(c, OpCode::LoadTrue, reg, 0, 0); }
        ExpKind::VK => code_loadk(c, reg, e.info),
        ExpKind::VKInt => code_int(c, reg, e.ival),
        ExpKind::VKFlt => code_float(c, reg, e.nval),
        ExpKind::VReloc => {
            let pc = e.info as usize;
            let mut instr = c.chunk.code[pc];
            Instruction::set_a(&mut instr, reg);
            c.chunk.code[pc] = instr;
        }
        ExpKind::VNonReloc => {
            if reg != e.info {
                code_abc(c, OpCode::Move, reg, e.info, 0);
            }
        }
        ExpKind::VJmp => return,
        _ => {}
    }
    e.info = reg;
    e.kind = ExpKind::VNonReloc;
}

/// Discharge to any register (对齐discharge2anyreg)
fn discharge2anyreg(c: &mut Compiler, e: &mut ExpDesc) {
    if e.kind != ExpKind::VNonReloc {
        reserve_regs(c, 1);
        discharge2reg(c, e, c.freereg - 1);
    }
}

/// Ensure expression is in next available register (对齐luaK_exp2nextreg)
pub(crate) fn exp2nextreg(c: &mut Compiler, e: &mut ExpDesc) {
    discharge_vars(c, e);
    free_exp(c, e);
    reserve_regs(c, 1);
    exp2reg(c, e, c.freereg - 1);
}

/// Ensure expression is in some register (对齐luaK_exp2anyreg)
pub(crate) fn exp2anyreg(c: &mut Compiler, e: &mut ExpDesc) -> u32 {
    discharge_vars(c, e);
    if e.kind == ExpKind::VNonReloc {
        if !has_jumps(e) {
            return e.info;
        }
        if e.info >= nvarstack(c) {
            exp2reg(c, e, e.info);
            return e.info;
        }
    }
    exp2nextreg(c, e);
    e.info
}

/// Ensure expression value is in register or upvalue (对齐luaK_exp2anyregup)
pub(crate) fn exp2anyregup(c: &mut Compiler, e: &mut ExpDesc) {
    if e.kind != ExpKind::VUpval || has_jumps(e) {
        exp2anyreg(c, e);
    }
}

/// Ensure expression is discharged (对齐luaK_exp2val)
pub(crate) fn exp2val(c: &mut Compiler, e: &mut ExpDesc) {
    if has_jumps(e) {
        exp2anyreg(c, e);
    } else {
        discharge_vars(c, e);
    }
}

/// Full exp2reg with jump handling (对齐exp2reg)
pub(crate) fn exp2reg(c: &mut Compiler, e: &mut ExpDesc, reg: u32) {
    discharge2reg(c, e, reg);
    if e.kind == ExpKind::VJmp {
        concat(c, &mut e.t, e.info as i32);
    }
    if has_jumps(e) {
        // TODO: Handle jump lists for boolean expressions
    }
    e.f = NO_JUMP;
    e.t = NO_JUMP;
    e.info = reg;
    e.kind = ExpKind::VNonReloc;
}

/// Set expression to return one result (对齐luaK_setoneret)
pub(crate) fn set_one_ret(c: &mut Compiler, e: &mut ExpDesc) {
    if e.kind == ExpKind::VCall {
        let pc = e.info as usize;
        let instr = c.chunk.code[pc];
        e.kind = ExpKind::VNonReloc;
        e.info = Instruction::get_a(instr);
    } else if e.kind == ExpKind::VVararg {
        let pc = e.info as usize;
        let mut instr = c.chunk.code[pc];
        Instruction::set_c(&mut instr, 2);
        c.chunk.code[pc] = instr;
        e.kind = ExpKind::VReloc;
    }
}

/// Set expression to return multiple results (对齐luaK_setreturns)
pub(crate) fn set_returns(c: &mut Compiler, e: &mut ExpDesc, nresults: i32) {
    if e.kind == ExpKind::VCall {
        let pc = e.info as usize;
        let mut instr = c.chunk.code[pc];
        Instruction::set_c(&mut instr, (nresults + 1) as u32);
        c.chunk.code[pc] = instr;
    } else if e.kind == ExpKind::VVararg {
        let pc = e.info as usize;
        let mut instr = c.chunk.code[pc];
        Instruction::set_c(&mut instr, (nresults + 1) as u32);
        Instruction::set_a(&mut instr, c.freereg);
        c.chunk.code[pc] = instr;
        reserve_regs(c, 1);
    }
}

/// Check if expression has jumps
fn has_jumps(e: &ExpDesc) -> bool {
    e.t != NO_JUMP || e.f != NO_JUMP
}

/// Generate jump if expression is false (对齐luaK_goiffalse)
pub(crate) fn goiffalse(c: &mut Compiler, e: &mut ExpDesc) -> i32 {
    discharge_vars(c, e);
    match e.kind {
        ExpKind::VJmp => {
            // Already a jump - negate condition
            negate_condition(c, e);
            e.f
        }
        ExpKind::VNil | ExpKind::VFalse => {
            // Always false - no jump needed
            NO_JUMP
        }
        _ => {
            // Generate test and jump
            discharge2anyreg(c, e);
            free_exp(c, e);
            let jmp = jump_on_cond(c, e.info, false);
            jmp as i32
        }
    }
}

/// Generate jump if expression is true (对齐luaK_goiftrue)
pub(crate) fn goiftrue(c: &mut Compiler, e: &mut ExpDesc) -> i32 {
    discharge_vars(c, e);
    match e.kind {
        ExpKind::VJmp => {
            // Already a jump - keep condition
            e.t
        }
        ExpKind::VNil | ExpKind::VFalse => {
            // Always false - jump unconditionally
            jump(c) as i32
        }
        ExpKind::VTrue | ExpKind::VK | ExpKind::VKFlt | ExpKind::VKInt | ExpKind::VKStr => {
            // Always true - no jump
            NO_JUMP
        }
        _ => {
            // Generate test and jump
            discharge2anyreg(c, e);
            free_exp(c, e);
            let jmp = jump_on_cond(c, e.info, true);
            jmp as i32
        }
    }
}

/// Negate condition of jump (对齐negatecondition)
fn negate_condition(_c: &mut Compiler, e: &mut ExpDesc) {
    // Swap true and false jump lists
    let temp = e.t;
    e.t = e.f;
    e.f = temp;
}

/// Generate conditional jump (对齐jumponcond)
fn jump_on_cond(c: &mut Compiler, reg: u32, cond: bool) -> usize {
    if cond {
        code_abc(c, OpCode::Test, reg, 0, 1)
    } else {
        code_abc(c, OpCode::Test, reg, 0, 0)
    }
}

/// Free register used by expression (对齐freeexp)
pub(crate) fn free_exp(c: &mut Compiler, e: &ExpDesc) {
    if e.kind == ExpKind::VNonReloc {
        free_reg(c, e.info);
    }
}

/// Load constant into register (对齐luaK_codek)
fn code_loadk(c: &mut Compiler, reg: u32, k: u32) {
    if k <= 0x3FFFF {
        code_abx(c, OpCode::LoadK, reg, k);
    } else {
        code_abx(c, OpCode::LoadKX, reg, 0);
        code_extra_arg(c, k);
    }
}

/// Emit EXTRAARG instruction
fn code_extra_arg(c: &mut Compiler, a: u32) {
    let instr = Instruction::create_ax(OpCode::ExtraArg, a);
    code(c, instr);
}

/// Load integer into register (对齐luaK_int)
pub(crate) fn code_int(c: &mut Compiler, reg: u32, i: i64) {
    if i >= -0x1FFFF && i <= 0x1FFFF {
        code_asbx(c, OpCode::LoadI, reg, i as i32);
    } else {
        let k = int_k(c, i);
        code_loadk(c, reg, k);
    }
}

/// Load float into register (对齐luaK_float)
fn code_float(c: &mut Compiler, reg: u32, f: f64) {
    let fi = f as i64;
    if (fi as f64) == f && fi >= -0x1FFFF && fi <= 0x1FFFF {
        code_asbx(c, OpCode::LoadF, reg, fi as i32);
    } else {
        let k = number_k(c, f);
        code_loadk(c, reg, k);
    }
}

/// Store expression value into variable (对齐luaK_storevar)
pub(crate) fn store_var(c: &mut Compiler, var: &ExpDesc, ex: &mut ExpDesc) {
    use super::expdesc::ExpKind;
    
    match var.kind {
        ExpKind::VLocal => {
            // Store to local variable
            free_exp(c, ex);
            exp2reg(c, ex, var.info as u32);
        }
        ExpKind::VUpval => {
            // Store to upvalue
            let e = exp2anyreg(c, ex);
            code_abc(c, OpCode::SetUpval, e, var.info, 0);
        }
        ExpKind::VIndexUp => {
            // Store to indexed upvalue: upval[k] = v
            // Used for global variable assignment like _ENV[x] = v
            let e = exp2anyreg(c, ex);
            code_abck(c, OpCode::SetTabUp, var.ind.t, e, var.ind.idx, true);
            free_exp(c, ex);
        }
        ExpKind::VIndexed => {
            // Store to table: t[k] = v
            // TODO: Implement proper indexed store with SETTABLE, SETI, SETFIELD variants
            // For now, use generic SETTABLE
            let val = exp2anyreg(c, ex);
            code_abc(c, OpCode::SetTable, var.ind.t, var.ind.idx, val);
            free_exp(c, ex);
        }
        _ => {
            // Invalid variable kind for store
            panic!("Invalid variable kind for store: {:?}", var.kind);
        }
    }
}

/// Create indexed expression from table and key (对齐 luaK_indexed)
/// 根据 key 的类型选择合适的索引方式
pub(crate) fn indexed(c: &mut Compiler, t: &mut ExpDesc, k: &mut ExpDesc) {
    // t 必须已经是寄存器或 upvalue
    debug_assert!(
        matches!(t.kind, ExpKind::VNonReloc | ExpKind::VLocal | ExpKind::VUpval | ExpKind::VIndexUp)
    );
    
    // 根据 key 的类型选择索引方式
    if let Some(idx) = valid_op(k) {
        // Key 可以作为 RK 操作数（寄存器或常量）
        let op = if t.kind == ExpKind::VUpval {
            ExpKind::VIndexUp // upvalue[k]
        } else {
            ExpKind::VIndexed // t[k]
        };
        
        // CRITICAL: 先设置ind字段，再调用exp2anyreg
        t.kind = op;
        t.ind.idx = idx;
        t.ind.t = if op == ExpKind::VIndexUp { t.info } else { exp2anyreg(c, t) };
    } else if k.kind == ExpKind::VKStr {
        // 字符串常量索引
        let op = if t.kind == ExpKind::VUpval {
            ExpKind::VIndexUp
        } else {
            ExpKind::VIndexStr
        };
        
        // CRITICAL: 先保存k.info，再调用exp2anyreg（它会触发discharge_vars读取ind）
        let key_idx = k.info;
        t.kind = op;
        t.ind.idx = key_idx; // 必须在exp2anyreg之前设置！
        t.ind.t = if op == ExpKind::VIndexUp { t.info } else { exp2anyreg(c, t) };
    } else if k.kind == ExpKind::VKInt && fits_as_offset(k.ival) {
        // 整数索引（在范围内）
        let op = if t.kind == ExpKind::VUpval {
            ExpKind::VIndexUp
        } else {
            ExpKind::VIndexI
        };
        
        // CRITICAL: 先设置ind字段，再调用exp2anyreg
        t.kind = op;
        t.ind.idx = k.ival as u32;
        t.ind.t = if op == ExpKind::VIndexUp { t.info } else { exp2anyreg(c, t) };
    } else {
        // 通用索引：需要把 key 放到寄存器
        t.kind = ExpKind::VIndexed;
        t.ind.t = exp2anyreg(c, t);
        t.ind.idx = exp2anyreg(c, k);
    }
}

/// Check if integer fits as an offset (Lua 使用 8 位或更多位)
fn fits_as_offset(n: i64) -> bool {
    n >= 0 && n < 256
}

/// Check if expression is valid as RK operand and return its index
fn valid_op(e: &ExpDesc) -> Option<u32> {
    match e.kind {
        ExpKind::VK | ExpKind::VKInt | ExpKind::VKFlt => Some(e.info),
        _ => None,
    }
}

/// Discharge expression to any register (对齐 luaK_exp2anyreg)
pub(crate) fn discharge_2any_reg(c: &mut Compiler, e: &mut ExpDesc) {
    discharge_vars(c, e);
    if e.kind != ExpKind::VNonReloc {
        reserve_regs(c, 1);
        discharge2reg(c, e, c.freereg - 1);
    }
}
