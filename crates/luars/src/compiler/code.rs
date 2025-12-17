// Code generation - Port from lcode.c
use crate::compiler::func_state::FuncState;
use crate::lua_vm::{Instruction, OpCode};

// Port of luaK_code from lcode.c
pub fn code_abc(fs: &mut FuncState, op: OpCode, a: u32, b: u32, c: u32) -> usize {
    let mut instr = (op as u32) << Instruction::POS_OP;
    Instruction::set_a(&mut instr, a);
    Instruction::set_b(&mut instr, b);
    Instruction::set_c(&mut instr, c);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.pc += 1;
    pc
}

// Port of luaK_codeABx from lcode.c  
pub fn code_abx(fs: &mut FuncState, op: OpCode, a: u32, bx: u32) -> usize {
    let mut instr = (op as u32) << Instruction::POS_OP;
    Instruction::set_a(&mut instr, a);
    Instruction::set_bx(&mut instr, bx);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.pc += 1;
    pc
}

// Port of luaK_codeAsBx from lcode.c
pub fn code_asbx(fs: &mut FuncState, op: OpCode, a: u32, sbx: i32) -> usize {
    let mut instr = (op as u32) << Instruction::POS_OP;
    Instruction::set_a(&mut instr, a);
    let bx = (sbx + Instruction::OFFSET_SBX) as u32;
    Instruction::set_bx(&mut instr, bx);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.pc += 1;
    pc
}

// Port of luaK_codeABCk from lcode.c
pub fn code_abck(fs: &mut FuncState, op: OpCode, a: u32, b: u32, c: u32, k: bool) -> usize {
    let mut instr = (op as u32) << Instruction::POS_OP;
    Instruction::set_a(&mut instr, a);
    Instruction::set_b(&mut instr, b);
    Instruction::set_c(&mut instr, c);
    Instruction::set_k(&mut instr, k);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.pc += 1;
    pc
}

use crate::compiler::expression::{ExpDesc, ExpKind};

// Port of luaK_ret from lcode.c
pub fn ret(fs: &mut FuncState, first: u8, nret: u8) -> usize {
    code_abc(fs, OpCode::Return, first as u32, (nret + 1) as u32, 0)
}

// Port of luaK_jump from lcode.c
pub fn jump(fs: &mut FuncState) -> usize {
    code_asbx(fs, OpCode::Jmp, 0, -1)
}

// Port of luaK_getlabel from lcode.c
pub fn get_label(fs: &FuncState) -> usize {
    fs.pc
}

// Port of luaK_patchtohere from lcode.c
pub fn patchtohere(fs: &mut FuncState, list: isize) {
    let here = get_label(fs) as isize;
    patchlist(fs, list, here);
}

// Port of luaK_concat from lcode.c
pub fn concat(fs: &mut FuncState, l1: &mut isize, l2: isize) {
    if l2 == -1 {
        return;
    }
    if *l1 == -1 {
        *l1 = l2;
    } else {
        let mut list = *l1;
        let mut next = get_jump(fs, list as usize);
        while next != -1 {
            list = next;
            next = get_jump(fs, list as usize);
        }
        fix_jump(fs, list as usize, l2 as usize);
    }
}

// Port of luaK_patchlist from lcode.c
pub fn patchlist(fs: &mut FuncState, mut list: isize, target: isize) {
    while list != -1 {
        let next = get_jump(fs, list as usize);
        fix_jump(fs, list as usize, target as usize);
        list = next;
    }
}

// Helper: get jump target from instruction
fn get_jump(fs: &FuncState, pc: usize) -> isize {
    if pc >= fs.chunk.code.len() {
        return -1;
    }
    let instr = fs.chunk.code[pc];
    let offset = Instruction::get_sbx(instr) as i32 - Instruction::OFFSET_SBX;
    if offset == -1 {
        -1
    } else {
        (pc as isize) + 1 + (offset as isize)
    }
}

// Helper: patch jump instruction
pub fn fix_jump(fs: &mut FuncState, pc: usize, target: usize) {
    if pc >= fs.chunk.code.len() {
        return;
    }
    let offset = (target as isize) - (pc as isize) - 1;
    let max_sbx = (1 << (Instruction::SIZE_BX - 1)) - 1;
    if offset < -(Instruction::OFFSET_SBX as isize) || offset > max_sbx {
        // Error: jump too long
        return;
    }
    let instr = &mut fs.chunk.code[pc];
    let bx = (offset + Instruction::OFFSET_SBX as isize) as u32;
    Instruction::set_bx(instr, bx);
}

// Port of luaK_exp2nextreg from lcode.c
pub fn exp2nextreg(fs: &mut FuncState, e: &mut ExpDesc) -> u8 {
    discharge_vars(fs, e);
    free_exp(fs, e);
    reserve_regs(fs, 1);
    let reg = fs.freereg - 1;
    exp2reg(fs, e, reg);
    reg
}

// Port of luaK_exp2anyreg from lcode.c
pub fn exp2anyreg(fs: &mut FuncState, e: &mut ExpDesc) -> u8 {
    discharge_vars(fs, e);
    if e.kind == ExpKind::VNONRELOC {
        if !e.has_jumps() {
            return unsafe { e.u.info as u8 };
        }
        if unsafe { e.u.info } >= fs.nactvar as i32 {
            exp2reg(fs, e, unsafe { e.u.info as u8 });
            return unsafe { e.u.info as u8 };
        }
    }
    exp2nextreg(fs, e)
}

// Port of luaK_exp2reg from lcode.c
pub fn exp2reg(fs: &mut FuncState, e: &mut ExpDesc, reg: u8) {
    discharge2reg(fs, e, reg);
    if e.kind == ExpKind::VJMP {
        concat(fs, &mut e.t, unsafe { e.u.info as isize });
    }
    if e.has_jumps() {
        let _p_f = -1;
        let _p_t = -1;
        let _final_label = get_label(fs);
        // patchlist to true/false
        // TODO: Complete jump patching logic
    }
    e.kind = ExpKind::VNONRELOC;
    e.u.info = reg as i32;
}

// Port of luaK_exp2val from lcode.c
// Port of dischargevars from lcode.c
pub fn discharge_vars(fs: &mut FuncState, e: &mut ExpDesc) {
    match e.kind {
        ExpKind::VLOCAL => {
            e.u.info = unsafe { e.u.var.ridx as i32 };
            e.kind = ExpKind::VNONRELOC;
        }
        ExpKind::VUPVAL => {
            e.u.info = code_abc(fs, OpCode::GetUpval, 0, unsafe { e.u.info as u32 }, 0) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXUP => {
            e.u.info = code_abc(fs, OpCode::GetTabUp, 0, unsafe { e.u.ind.t as u32 }, unsafe { e.u.ind.idx as u32 }) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXI => {
            free_reg(fs, unsafe { e.u.ind.t as u8 });
            e.u.info = code_abc(fs, OpCode::GetI, 0, unsafe { e.u.ind.t as u32 }, unsafe { e.u.ind.idx as u32 }) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXSTR => {
            free_reg(fs, unsafe { e.u.ind.t as u8 });
            e.u.info = code_abc(fs, OpCode::GetField, 0, unsafe { e.u.ind.t as u32 }, unsafe { e.u.ind.idx as u32 }) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXED => {
            free_reg(fs, unsafe { e.u.ind.idx as u8 });
            free_reg(fs, unsafe { e.u.ind.t as u8 });
            e.u.info = code_abc(fs, OpCode::GetTable, 0, unsafe { e.u.ind.t as u32 }, unsafe { e.u.ind.idx as u32 }) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VVARARG | ExpKind::VCALL => {
            setoneret(fs, e);
        }
        _ => {}
    }
}

// Port of discharge2reg from lcode.c
pub fn discharge2reg(fs: &mut FuncState, e: &mut ExpDesc, reg: u8) {
    discharge_vars(fs, e);
    match e.kind {
        ExpKind::VNIL => {
            code_abc(fs, OpCode::LoadNil, reg as u32, 0, 0);
        }
        ExpKind::VFALSE | ExpKind::VTRUE => {
            // LoadBool not available, use LoadNil/LoadK instead
            if e.kind == ExpKind::VTRUE {
                code_abc(fs, OpCode::LoadTrue, reg as u32, 0, 0);
            } else {
                code_abc(fs, OpCode::LoadFalse, reg as u32, 0, 0);
            }
        }
        ExpKind::VK => {
            code_abx(fs, OpCode::LoadK, reg as u32, unsafe { e.u.info as u32 });
        }
        ExpKind::VKFLT => {
            // TODO: Add constant to chunk
            code_abx(fs, OpCode::LoadK, reg as u32, 0);
        }
        ExpKind::VKINT => {
            code_asbx(fs, OpCode::LoadI, reg as u32, unsafe { e.u.ival as i32 });
        }
        ExpKind::VNONRELOC => {
            if unsafe { e.u.info } != reg as i32 {
                code_abc(fs, OpCode::Move, reg as u32, unsafe { e.u.info as u32 }, 0);
            }
        }
        ExpKind::VRELOC => {
            let pc = unsafe { e.u.info as usize };
            Instruction::set_a(&mut fs.chunk.code[pc], reg as u32);
        }
        _ => {}
    }
    e.kind = ExpKind::VNONRELOC;
    e.u.info = reg as i32;
}

// Port of freeexp from lcode.c
pub fn free_exp(fs: &mut FuncState, e: &ExpDesc) {
    if e.kind == ExpKind::VNONRELOC {
        free_reg(fs, unsafe { e.u.info as u8 });
    }
}

// Port of freereg from lcode.c
pub fn free_reg(fs: &mut FuncState, reg: u8) {
    if reg >= fs.nactvar && reg < fs.freereg {
        fs.freereg -= 1;
    }
}

// Port of reserveregs from lcode.c
pub fn reserve_regs(fs: &mut FuncState, n: u8) {
    fs.freereg += n;
    if (fs.freereg as usize) > fs.chunk.max_stack_size {
        fs.chunk.max_stack_size = fs.freereg as usize;
    }
}

// Port of luaK_nil from lcode.c
pub fn nil(fs: &mut FuncState, from: u8, n: u8) {
    if n > 0 {
        code_abc(fs, OpCode::LoadNil, from as u32, (n - 1) as u32, 0);
    }
}

// Port of luaK_setoneret from lcode.c
pub fn setoneret(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.kind == ExpKind::VCALL {
        e.kind = ExpKind::VNONRELOC;
        let pc = unsafe { e.u.info as usize };
        Instruction::set_c(&mut fs.chunk.code[pc], 2);
    } else if e.kind == ExpKind::VVARARG {
        let pc = unsafe { e.u.info as usize };
        Instruction::set_c(&mut fs.chunk.code[pc], 2);
        e.kind = ExpKind::VRELOC;
    }
}

// Port of luaK_setreturns from lcode.c (lines 722-732)
// void luaK_setreturns (FuncState *fs, expdesc *e, int nresults) {
//   Instruction *pc = &getinstruction(fs, e);
//   if (e->k == VCALL)
//     SETARG_C(*pc, nresults + 1);
//   else {
//     lua_assert(e->k == VVARARG);
//     SETARG_C(*pc, nresults + 1);
//     SETARG_A(*pc, fs->freereg);
//     luaK_reserveregs(fs, 1);
//   }
// }
pub fn setreturns(fs: &mut FuncState, e: &mut ExpDesc, nresults: u8) {
    let pc = unsafe { e.u.info as usize };
    if e.kind == ExpKind::VCALL {
        Instruction::set_c(&mut fs.chunk.code[pc], (nresults + 1) as u32);
    } else {
        // Must be VVARARG
        Instruction::set_c(&mut fs.chunk.code[pc], (nresults + 1) as u32);
        Instruction::set_a(&mut fs.chunk.code[pc], fs.freereg as u32);
        reserve_regs(fs, 1);
    }
}

// Port of tonumeral from lcode.c (lines 59-73)
fn tonumeral(e: &ExpDesc, _v: Option<&mut f64>) -> bool {
    if e.has_jumps() {
        return false;
    }
    match e.kind {
        ExpKind::VKINT | ExpKind::VKFLT => true,
        _ => false,
    }
}

// Constant management functions
use crate::lua_value::LuaValue;

const MAXINDEXRK: usize = 255; // Maximum index for R/K operands

// Add boolean true to constants
fn bool_t(fs: &mut FuncState) -> usize {
    add_constant(fs, LuaValue::boolean(true))
}

// Add boolean false to constants
fn bool_f(fs: &mut FuncState) -> usize {
    add_constant(fs, LuaValue::boolean(false))
}

// Add nil to constants
fn nil_k(fs: &mut FuncState) -> usize {
    add_constant(fs, LuaValue::nil())
}

// Add integer to constants
fn int_k(fs: &mut FuncState, i: i64) -> usize {
    add_constant(fs, LuaValue::integer(i))
}

// Add number to constants
fn number_k(fs: &mut FuncState, n: f64) -> usize {
    add_constant(fs, LuaValue::float(n))
}

// Add string constant to chunk
pub fn string_k(fs: &mut FuncState, s: String) -> usize {
    // Intern string to ObjectPool and get StringId
    let (string_id, _) = fs.pool.create_string(&s);

    // Add LuaValue with StringId to constants
    let value = LuaValue::string(string_id);
    fs.chunk.constants.push(value);
    fs.chunk.constants.len() - 1
}

// Helper to add constant to chunk
fn add_constant(fs: &mut FuncState, value: LuaValue) -> usize {
    // Try to find existing constant
    for (i, k) in fs.chunk.constants.iter().enumerate() {
        if constants_equal(k, &value) {
            return i;
        }
    }
    // Add new constant
    fs.chunk.constants.push(value);
    fs.chunk.constants.len() - 1
}

// Check if two constants are equal
fn constants_equal(a: &LuaValue, b: &LuaValue) -> bool {
    if a.is_nil() && b.is_nil() {
        return true;
    }
    if a.is_boolean() && b.is_boolean() {
        return a.as_boolean() == b.as_boolean();
    }
    if a.is_integer() && b.is_integer() {
        return a.as_integer() == b.as_integer();
    }
    if a.is_float() && b.is_float() {
        return a.as_float() == b.as_float();
    }
    if a.is_string() && b.is_string() {
        return a.as_string_id() == b.as_string_id();
    }
    false
}

// Port of luaK_exp2K from lcode.c (lines 1000-1026)
fn exp2k(fs: &mut FuncState, e: &mut ExpDesc) -> bool {
    if !e.has_jumps() {
        let info = match e.kind {
            ExpKind::VTRUE => bool_t(fs),
            ExpKind::VFALSE => bool_f(fs),
            ExpKind::VNIL => nil_k(fs),
            ExpKind::VKINT => int_k(fs, unsafe { e.u.ival }),
            ExpKind::VKFLT => number_k(fs, unsafe { e.u.nval }),
            ExpKind::VKSTR => {
                // String already in constants, use existing info
                unsafe { e.u.info as usize }
            }
            ExpKind::VK => unsafe { e.u.info as usize },
            _ => return false,
        };
        
        if info <= MAXINDEXRK {
            e.kind = ExpKind::VK;
            e.u.info = info as i32;
            return true;
        }
    }
    false
}

// Port of exp2RK from lcode.c (lines 1030-1036)
fn exp2rk(fs: &mut FuncState, e: &mut ExpDesc) -> bool {
    if exp2k(fs, e) {
        true
    } else {
        exp2anyreg(fs, e);
        false
    }
}

// Port of getjumpcontrol from lcode.c (lines 245-250)
fn get_jump_control(fs: &FuncState, pc: usize) -> usize {
    if pc >= 1 && pc < fs.chunk.code.len() {
        let prev_instr = fs.chunk.code[pc - 1];
        let prev_op = OpCode::from(Instruction::get_opcode(prev_instr));
        // Check if previous instruction is a test mode instruction
        if matches!(prev_op, OpCode::Test | OpCode::TestSet | 
                    OpCode::Eq | OpCode::Lt | OpCode::Le |
                    OpCode::EqK | OpCode::EqI | OpCode::LtI | 
                    OpCode::LeI | OpCode::GtI | OpCode::GeI) {
            return pc - 1;
        }
    }
    pc
}

// Port of negatecondition from lcode.c (lines 1103-1108)
fn negatecondition(fs: &mut FuncState, e: &mut ExpDesc) {
    let pc = get_jump_control(fs, unsafe { e.u.info as usize });
    if pc < fs.chunk.code.len() {
        let instr = &mut fs.chunk.code[pc];
        let k = Instruction::get_k(*instr);
        Instruction::set_k(instr, !k);
    }
}

// Port of condjump from lcode.c (lines 223-226)
fn condjump(fs: &mut FuncState, op: OpCode, a: u32, b: u32, c: u32, k: bool) -> isize {
    code_abck(fs, op, a, b, c, k);
    jump(fs) as isize
}

// Port of discharge2anyreg from lcode.c (lines 882-886)
fn discharge2anyreg(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.kind != ExpKind::VNONRELOC {
        reserve_regs(fs, 1);
        discharge2reg(fs, e, fs.freereg - 1);
    }
}

// Port of removelastinstruction from lcode.c (lines 373-376)
fn remove_last_instruction(fs: &mut FuncState) {
    if fs.pc > 0 {
        fs.pc -= 1;
        fs.chunk.code.pop();
    }
}

const NO_REG: u32 = 255;

// Port of jumponcond from lcode.c (lines 1118-1130)
fn jumponcond(fs: &mut FuncState, e: &mut ExpDesc, cond: bool) -> isize {
    if e.kind == ExpKind::VRELOC {
        let ie = fs.chunk.code[unsafe { e.u.info as usize }];
        if OpCode::from(Instruction::get_opcode(ie)) == OpCode::Not {
            remove_last_instruction(fs);
            let b = Instruction::get_b(ie);
            return condjump(fs, OpCode::Test, b, 0, 0, !cond);
        }
    }
    discharge2anyreg(fs, e);
    free_exp(fs, e);
    condjump(fs, OpCode::TestSet, NO_REG, unsafe { e.u.info as u32 }, 0, cond)
}

// Port of luaK_goiftrue from lcode.c (lines 1135-1160)
pub fn goiftrue(fs: &mut FuncState, e: &mut ExpDesc) {
    discharge_vars(fs, e);
    let pc = match e.kind {
        ExpKind::VJMP => {
            negatecondition(fs, e);
            unsafe { e.u.info as isize }
        }
        ExpKind::VK | ExpKind::VKFLT | ExpKind::VKINT | ExpKind::VKSTR | ExpKind::VTRUE => {
            -1 // NO_JUMP: always true
        }
        _ => jumponcond(fs, e, false),
    };
    concat(fs, &mut e.f, pc);
    patchtohere(fs, e.t);
    e.t = -1;
}

// Port of luaK_goiffalse from lcode.c (lines 1162-1183)
pub fn goiffalse(fs: &mut FuncState, e: &mut ExpDesc) {
    discharge_vars(fs, e);
    let pc = match e.kind {
        ExpKind::VJMP => unsafe { e.u.info as isize },
        ExpKind::VNIL | ExpKind::VFALSE => -1, // NO_JUMP: always false
        _ => jumponcond(fs, e, true),
    };

    concat(fs, &mut e.t, pc);
    patchtohere(fs, e.f);
    e.f = -1;
}

// Binary operator enum to match C code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpr {
    Add, Sub, Mul, Div, IDiv, Mod, Pow,
    BAnd, BOr, BXor, Shl, Shr,
    Concat,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,
}

// Port of luaK_infix from lcode.c (lines 1637-1678)
pub fn infix(fs: &mut FuncState, op: BinOpr, v: &mut ExpDesc) {
    discharge_vars(fs, v);
    match op {
        BinOpr::And => {
            goiftrue(fs, v);
        }
        BinOpr::Or => {
            goiffalse(fs, v);
        }
        BinOpr::Concat => {
            exp2nextreg(fs, v);
        }
        BinOpr::Add | BinOpr::Sub | BinOpr::Mul | BinOpr::Div | BinOpr::IDiv |
        BinOpr::Mod | BinOpr::Pow | BinOpr::BAnd | BinOpr::BOr | BinOpr::BXor |
        BinOpr::Shl | BinOpr::Shr => {
            if !tonumeral(v, None) {
                exp2anyreg(fs, v);
            }
        }
        BinOpr::Eq | BinOpr::Ne => {
            if !tonumeral(v, None) {
                exp2rk(fs, v);
            }
        }
        BinOpr::Lt | BinOpr::Le | BinOpr::Gt | BinOpr::Ge => {
            if !tonumeral(v, None) {
                exp2anyreg(fs, v);
            }
        }
    }
}

// Simplified implementation of luaK_posfix - generate binary operation
pub fn posfix(fs: &mut FuncState, op: OpCode, e1: &mut ExpDesc, e2: &mut ExpDesc) {
    discharge_vars(fs, e2);
    
    // Save freereg before allocating operands
    let old_free = fs.freereg;
    
    // Get operands in registers
    let o1 = exp2anyreg(fs, e1);
    let o2 = exp2anyreg(fs, e2);
    
    // Free both operand registers
    free_reg(fs, o2);
    free_reg(fs, o1);
    
    // Restore freereg and allocate result register
    fs.freereg = old_free;
    let res = fs.freereg;
    reserve_regs(fs, 1);
    
    // Generate binary op instruction
    code_abc(fs, op, res as u32, o1 as u32, o2 as u32);
    
    e1.kind = ExpKind::VNONRELOC;
    e1.u.info = res as i32;
}

// Simplified implementation of luaK_prefix - generate unary operation  
pub fn prefix(fs: &mut FuncState, op: OpCode, e: &mut ExpDesc) {
    discharge_vars(fs, e);
    
    let o = exp2anyreg(fs, e);
    free_reg(fs, o);
    
    let res = fs.freereg;
    reserve_regs(fs, 1);
    code_abc(fs, op, res as u32, o as u32, 0);
    
    e.kind = ExpKind::VNONRELOC;
    e.u.info = res as i32;
}

// Port of luaK_exp2anyregup from lcode.c (lines 978-981)
pub fn exp2anyregup(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.kind != ExpKind::VUPVAL || e.has_jumps() {
        exp2anyreg(fs, e);
    }
}

// Port of luaK_exp2val from lcode.c (lines 988-993)
pub fn exp2val(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.kind == ExpKind::VJMP || e.has_jumps() {
        exp2anyreg(fs, e);
    } else {
        discharge_vars(fs, e);
    }
}

// Port of luaK_indexed from lcode.c (lines 1280-1310)
pub fn indexed(fs: &mut FuncState, t: &mut ExpDesc, k: &mut ExpDesc) {
    use crate::compiler::expression::ExpKind;
    
    // Convert string to constant if needed
    if k.kind == ExpKind::VKSTR {
        // str2K - convert to constant
        let k_idx = unsafe { k.u.info as usize };
        k.kind = ExpKind::VK;
        k.u.info = k_idx as i32;
    }
    
    // Table must be in local/nonreloc/upval
    if t.kind == ExpKind::VUPVAL && !is_kstr(k) {
        exp2anyreg(fs, t);
    }
    
    if t.kind == ExpKind::VUPVAL {
        let temp = unsafe { t.u.info };
        t.u.ind.t = temp as i16;
        t.u.ind.idx = unsafe { k.u.info as i16 };
        t.kind = ExpKind::VINDEXUP;
    } else {
        // Register index of the table
        t.u.ind.t = if t.kind == ExpKind::VLOCAL {
            unsafe { t.u.var.ridx as i16 }
        } else {
            unsafe { t.u.info as i16 }
        };
        
        if is_kstr(k) {
            t.u.ind.idx = unsafe { k.u.info as i16 };
            t.kind = ExpKind::VINDEXSTR;
        } else if is_cint(k) {
            t.u.ind.idx = unsafe { k.u.ival as i16 };
            t.kind = ExpKind::VINDEXI;
        } else {
            t.u.ind.idx = exp2anyreg(fs, k) as i16;
            t.kind = ExpKind::VINDEXED;
        }
    }
}

// Check if expression is a constant string
fn is_kstr(e: &ExpDesc) -> bool {
    e.kind == ExpKind::VK || e.kind == ExpKind::VKSTR
}

// Check if expression is a constant integer in valid range
fn is_cint(e: &ExpDesc) -> bool {
    e.kind == ExpKind::VKINT
}

// Port of luaK_self from lcode.c (lines 1087-1097)
pub fn self_op(fs: &mut FuncState, e: &mut ExpDesc, key_idx: u8) {
    let ereg = exp2anyreg(fs, e);
    free_exp(fs, e);
    
    let base = fs.freereg;
    e.u.info = base as i32;
    e.kind = ExpKind::VNONRELOC;
    reserve_regs(fs, 2); // function and 'self'
    
    // SELF A B C: R(A+1) := R(B); R(A) := R(B)[RK(C)]
    code_abc(fs, OpCode::Self_, base as u32, ereg as u32, key_idx as u32);
}

// Port of luaK_setmultret - set call/vararg to return multiple values
pub fn setmultret(fs: &mut FuncState, e: &mut ExpDesc) {
    setreturns(fs, e, 0); // 0 means LUA_MULTRET
}

// Port of luaK_fixline from lcode.c (lines 1787-1790)
pub fn fixline(fs: &mut FuncState, line: usize) {
    // Remove last line info and add it again with new line
    // For now, we just ensure line_info has correct size
    if fs.chunk.line_info.len() > 0 {
        let last_idx = fs.chunk.line_info.len() - 1;
        fs.chunk.line_info[last_idx] = line as u32;
    }
}

// Port of luaK_exp2const from lcode.c (lines 85-108)
pub fn exp2const(fs: &FuncState, e: &ExpDesc) -> Option<crate::lua_value::LuaValue> {
    if e.has_jumps() {
        return None;
    }
    
    match e.kind {
        ExpKind::VFALSE => Some(LuaValue::boolean(false)),
        ExpKind::VTRUE => Some(LuaValue::boolean(true)),
        ExpKind::VNIL => Some(LuaValue::nil()),
        ExpKind::VKSTR => {
            // String constant - already in constants
            let idx = unsafe { e.u.info } as usize;
            if idx < fs.chunk.constants.len() {
                Some(fs.chunk.constants[idx])
            } else {
                None
            }
        }
        ExpKind::VK => {
            // Constant in K
            let idx = unsafe { e.u.info } as usize;
            if idx < fs.chunk.constants.len() {
                Some(fs.chunk.constants[idx])
            } else {
                None
            }
        }
        ExpKind::VKINT => Some(LuaValue::integer(unsafe { e.u.ival })),
        ExpKind::VKFLT => Some(LuaValue::float(unsafe { e.u.nval })),
        ExpKind::VCONST => {
            // TODO: get from actvar array
            None
        }
        _ => None,
    }
}
