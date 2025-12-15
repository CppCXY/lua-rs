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
            // 注意：idx可能是RK编码的常量（高位标志0x100），不应该释放
            // 参考lcode.c:732-738，官方只调用freexp(e)，当e是VIndexed时不做任何事
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

/// Try to convert expression to K (constant in table) (对齐luaK_exp2K)
/// Returns true if successfully converted to K, false otherwise
pub(crate) fn exp2k(c: &mut Compiler, e: &mut ExpDesc) -> bool {
    if !has_jumps(e) {
        match e.kind {
            ExpKind::VNil => {
                e.info = super::helpers::nil_k(c);
                e.kind = ExpKind::VK;
                return true;
            }
            ExpKind::VTrue => {
                e.info = super::helpers::bool_k(c, true);
                e.kind = ExpKind::VK;
                return true;
            }
            ExpKind::VFalse => {
                e.info = super::helpers::bool_k(c, false);
                e.kind = ExpKind::VK;
                return true;
            }
            ExpKind::VKInt => {
                e.info = super::helpers::int_k(c, e.ival);
                e.kind = ExpKind::VK;
                return true;
            }
            ExpKind::VKFlt => {
                e.info = super::helpers::number_k(c, e.nval);
                e.kind = ExpKind::VK;
                return true;
            }
            ExpKind::VKStr => {
                // Already a string constant, just ensure it's in K form
                e.kind = ExpKind::VK;
                return true;
            }
            ExpKind::VK => {
                // Already a K expression
                return true;
            }
            _ => {}
        }
    }
    false
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
        let base_reg = Instruction::get_a(instr);
        Instruction::set_c(&mut instr, (nresults + 1) as u32);
        c.chunk.code[pc] = instr;
        
        // 当丢弃返回值时(nresults==0)，将freereg重置为call的基寄存器（对齐Lua C）
        // 参考lcode.c中luaK_setreturns: if (nresults == 0) fs->freereg = base(e);
        if nresults == 0 {
            c.freereg = base_reg;
        }
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
pub(crate) fn has_jumps(e: &ExpDesc) -> bool {
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
            // Generate test and jump（对齐Lua C中的luaK_goiffalse）
            discharge2anyreg(c, e);
            free_exp(c, e);
            // TEST指令：if not R then skip next
            jump_on_cond(c, e.info, false);
            // JMP指令：跳转到目标位置（稍后patch）
            let jmp = jump(c);
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
            // Generate test and jump（对齐Lua C中的luaK_goiftrue）
            discharge2anyreg(c, e);
            free_exp(c, e);
            // TEST指令：if R then skip next
            jump_on_cond(c, e.info, true);
            // JMP指令：跳转到目标位置（稍后patch）
            let jmp = jump(c);
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
            // 参考lcode.c:1409-1412: luaK_storevar for VLOCAL
            // 使用var.var.ridx而不是var.info（info未被设置）
            free_exp(c, ex);
            exp2reg(c, ex, var.var.ridx);
        }
        ExpKind::VUpval => {
            // Store to upvalue
            let e = exp2anyreg(c, ex);
            code_abc(c, OpCode::SetUpval, e, var.info, 0);
        }
        ExpKind::VIndexUp => {
            // Store to indexed upvalue: upval[k] = v
            // Used for global variable assignment like _ENV[x] = v
            // SETTABUP A B C k: UpValue[A][K[B]] := RK(C)
            let e = exp2anyreg(c, ex);
            code_abck(c, OpCode::SetTabUp, var.ind.t, var.ind.idx, e, true);
            free_exp(c, ex);
        }
        ExpKind::VIndexed => {
            // Store to table: t[k] = v (对齐luac SETTABLE)
            let val = exp2anyreg(c, ex);
            code_abc(c, OpCode::SetTable, var.ind.t, var.ind.idx, val);
            free_exp(c, ex);
        }
        ExpKind::VIndexStr => {
            // Store to table with string key: t.field = v (对齐luac SETFIELD)
            let val = exp2anyreg(c, ex);
            code_abc(c, OpCode::SetField, var.ind.t, var.ind.idx, val);
            free_exp(c, ex);
        }
        ExpKind::VIndexI => {
            // Store to table with integer key: t[i] = v (对齐luac SETI)
            let val = exp2anyreg(c, ex);
            code_abc(c, OpCode::SetI, var.ind.t, var.ind.idx, val);
            free_exp(c, ex);
        }
        ExpKind::VNonReloc | ExpKind::VReloc => {
            // If variable was discharged to a register, this is an error
            // This should not happen - indexed expressions should not be discharged before store
            panic!("Cannot store to discharged indexed variable: {:?}", var.kind);
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
    // 参考lcode.c:1281-1282: if (k->k == VKSTR) str2K(fs, k);
    if k.kind == ExpKind::VKStr {
        // String constant - already in correct form
    }
    
    // t 必须已经是寄存器或 upvalue
    debug_assert!(
        matches!(t.kind, ExpKind::VNonReloc | ExpKind::VLocal | ExpKind::VUpval)
    );
    
    // 参考lcode.c:1285-1286: upvalue indexed by non 'Kstr' needs register
    if t.kind == ExpKind::VUpval && k.kind != ExpKind::VKStr {
        exp2anyreg(c, t);
    }
    
    // 根据 key 的类型选择索引方式
    if let Some(idx) = valid_op(k) {
        // Key 可以作为 RK 操作数（寄存器或常量）
        // 对于常量，需要进行RK编码（加上0x100标志）
        let rk_idx = if k.kind == ExpKind::VK {
            idx | 0x100  // RK编码：常量索引+0x100
        } else {
            idx  // 寄存器直接使用
        };
        
        let op = if t.kind == ExpKind::VUpval {
            ExpKind::VIndexUp // upvalue[k]
        } else {
            ExpKind::VIndexed // t[k]
        };
        
        // CRITICAL: 先exp2anyreg获取t的寄存器，再设置kind
        let t_reg = if op == ExpKind::VIndexUp { t.info } else { exp2anyreg(c, t) };
        t.kind = op;
        t.ind.idx = rk_idx;
        t.ind.t = t_reg;
    } else if k.kind == ExpKind::VKStr {
        // 字符串常量索引
        // 参考lcode.c:1297-1299
        let op = if t.kind == ExpKind::VUpval {
            ExpKind::VIndexUp
        } else {
            ExpKind::VIndexStr
        };
        
        // 参考lcode.c:1296: t->u.ind.t = (t->k == VLOCAL) ? t->u.var.ridx: t->u.info;
        let t_reg = if t.kind == ExpKind::VLocal {
            t.var.ridx
        } else if t.kind == ExpKind::VUpval {
            t.info
        } else {
            t.info // VNonReloc
        };
        let key_idx = k.info;
        t.kind = op;
        t.ind.idx = key_idx;
        t.ind.t = t_reg;
    } else if k.kind == ExpKind::VKInt && fits_as_offset(k.ival) {
        // 整数索引（在范围内）
        let op = if t.kind == ExpKind::VUpval {
            ExpKind::VIndexUp
        } else {
            ExpKind::VIndexI
        };
        
        // CRITICAL: 先exp2anyreg获取t的寄存器，再设置kind
        let t_reg = if op == ExpKind::VIndexUp { t.info } else { exp2anyreg(c, t) };
        t.kind = op;
        t.ind.idx = k.ival as u32;
        t.ind.t = t_reg;
    } else {
        // 通用索引：需要把 key 放到寄存器（对齐 Lua C 实现）
        // CRITICAL: 必须先 exp2anyreg 获取寄存器，再设置 kind
        let t_reg = exp2anyreg(c, t);
        let k_reg = exp2anyreg(c, k);
        t.kind = ExpKind::VIndexed;
        t.ind.t = t_reg;
        t.ind.idx = k_reg;
    }
}

/// Check if integer fits as an offset (Lua 使用 8 位或更多位)
fn fits_as_offset(n: i64) -> bool {
    n >= 0 && n < 256
}

/// Check if expression is valid as RK operand and return its index
fn valid_op(e: &ExpDesc) -> Option<u32> {
    match e.kind {
        ExpKind::VK => Some(e.info), // 常量池索引
        // VKInt 和 VKFlt 不应该走这个路径，它们需要特殊处理
        // 因为整数和浮点数存储在 ival/nval，不是 info
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
