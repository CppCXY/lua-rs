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
    if target == fs.pc as isize {
        patchtohere(fs, list);
    } else {
        while list != -1 {
            let next = get_jump(fs, list as usize);
            fix_jump(fs, list as usize, target as usize);
            list = next;
        }
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
pub fn exp2val(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.has_jumps() {
        exp2anyreg(fs, e);
    } else {
        discharge_vars(fs, e);
    }
}

// Port of dischargevars from lcode.c
pub fn discharge_vars(fs: &mut FuncState, e: &mut ExpDesc) {
    match e.kind {
        ExpKind::VLOCAL => {
            e.kind = ExpKind::VNONRELOC;
            e.u.info = unsafe { e.u.var.ridx as i32 };
        }
        ExpKind::VUPVAL => {
            let reg = fs.freereg;
            reserve_regs(fs, 1);
            code_abc(fs, OpCode::GetUpval, reg as u32, unsafe { e.u.info as u32 }, 0);
            e.kind = ExpKind::VNONRELOC;
            e.u.info = reg as i32;
        }
        ExpKind::VINDEXED => {
            let op = OpCode::GetTable;
            free_reg(fs, unsafe { e.u.ind.idx as u8 });
            free_reg(fs, unsafe { e.u.ind.t as u8 });
            let reg = fs.freereg;
            reserve_regs(fs, 1);
            code_abc(
                fs,
                op,
                reg as u32,
                unsafe { e.u.ind.t as u32 },
                unsafe { e.u.ind.idx as u32 },
            );
            e.kind = ExpKind::VNONRELOC;
            e.u.info = reg as i32;
        }
        ExpKind::VVARARG | ExpKind::VCALL => {
            code_abc(fs, OpCode::Return, 0, 1, 0);
            e.kind = ExpKind::VNONRELOC;
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
