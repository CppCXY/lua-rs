// Code generation - Port from lcode.c (Lua 5.4.8)
// This file corresponds to lua-5.4.8/src/lcode.c
use crate::compiler::func_state::FuncState;
use crate::compiler::parser::BinaryOperator;
use crate::compiler::tm_kind::TmKind;
use crate::lua_vm::{Instruction, OpCode};

// Port of int2sC from lcode.c (macro)
// Convert integer to sC format (with OFFSET_sC = 128)
fn int2sc(i: i32) -> u32 {
    ((i as u32).wrapping_add(128)) & 0xFF
}

// Port of fitsC from lcode.c:660-662
// Check whether 'i' can be stored in an 'sC' operand
fn fits_c(i: i64) -> bool {
    let offset_sc = 128i64;
    let max_arg_c = 255u32; // MAXARG_C = 255 for 8-bit C field
    (i.wrapping_add(offset_sc) as u64) <= (max_arg_c as u64)
}

// Port of isSCnumber from lcode.c:1257-1271
// Check whether expression 'e' is a literal integer or float in proper range to fit in sC
fn is_scnumber(e: &ExpDesc, pi: &mut i32, isfloat: &mut bool) -> bool {
    let i = match e.kind {
        ExpKind::VKINT => unsafe { e.u.ival },
        ExpKind::VKFLT => {
            // Try to convert float to integer
            let fval = unsafe { e.u.nval };
            let ival = fval as i64;
            // Check if float is exactly equal to integer (F2Ieq mode)
            if (ival as f64) == fval {
                *isfloat = true;
                ival
            } else {
                return false; // Not an integer-equivalent float
            }
        }
        _ => return false, // Not a number
    };

    // Check if it has jumps and if it fits in C field
    if !e.has_jumps() && fits_c(i) {
        *pi = int2sc(i as i32) as i32;
        true
    } else {
        false
    }
}

// Port of const2exp from lcode.c:693-720
// Convert a constant value to an expression
fn const_to_exp(value: crate::lua_value::LuaValue, e: &mut ExpDesc) {
    use crate::lua_value::LuaValueKind;

    match value.kind() {
        LuaValueKind::Integer => {
            e.kind = ExpKind::VKINT;
            e.u.ival = value.as_integer().unwrap_or(0);
        }
        LuaValueKind::Float => {
            e.kind = ExpKind::VKFLT;
            e.u.nval = value.as_float().unwrap_or(0.0);
        }
        LuaValueKind::Boolean => {
            if value.as_boolean().unwrap_or(false) {
                e.kind = ExpKind::VTRUE;
            } else {
                e.kind = ExpKind::VFALSE;
            }
        }
        LuaValueKind::Nil => {
            e.kind = ExpKind::VNIL;
        }
        LuaValueKind::String => {
            e.kind = ExpKind::VKSTR;
            e.u.info = value.as_string_id().unwrap_or(crate::StringId(0)).0 as i32;
        }
        _ => {
            // Other types shouldn't appear as compile-time constants
            e.kind = ExpKind::VNIL;
        }
    }
}

// Port of luaK_codeABC from lcode.c:397-402
// int luaK_codeABCk (FuncState *fs, OpCode o, int a, int b, int c, int k)
pub fn code_abc(fs: &mut FuncState, op: OpCode, a: u32, b: u32, c: u32) -> usize {
    let mut instr = (op as u32) << Instruction::POS_OP;
    Instruction::set_a(&mut instr, a);
    Instruction::set_b(&mut instr, b);
    Instruction::set_c(&mut instr, c);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.chunk.line_info.push(fs.lexer.line as u32);
    fs.pc += 1;
    pc
}

// Port of luaK_codeABx from lcode.c:409-414
// int luaK_codeABx (FuncState *fs, OpCode o, int a, unsigned int bc)
pub fn code_abx(fs: &mut FuncState, op: OpCode, a: u32, bx: u32) -> usize {
    let mut instr = (op as u32) << Instruction::POS_OP;
    Instruction::set_a(&mut instr, a);
    Instruction::set_bx(&mut instr, bx);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.chunk.line_info.push(fs.lexer.line as u32);
    fs.pc += 1;
    pc
}

// Port of codeAsBx from lcode.c:419-424
// static int codeAsBx (FuncState *fs, OpCode o, int a, int bc)
pub fn code_asbx(fs: &mut FuncState, op: OpCode, a: u32, sbx: i32) -> usize {
    let mut instr = (op as u32) << Instruction::POS_OP;
    Instruction::set_a(&mut instr, a);
    let bx = (sbx + Instruction::OFFSET_SBX) as u32;
    Instruction::set_bx(&mut instr, bx);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.chunk.line_info.push(fs.lexer.line as u32);
    fs.pc += 1;
    pc
}

// Port of luaK_codeABCk from lcode.c:397-402
// int luaK_codeABCk (FuncState *fs, OpCode o, int a, int b, int c, int k)
pub fn code_abck(fs: &mut FuncState, op: OpCode, a: u32, b: u32, c: u32, k: bool) -> usize {
    let mut instr = (op as u32) << Instruction::POS_OP;
    Instruction::set_a(&mut instr, a);
    Instruction::set_b(&mut instr, b);
    Instruction::set_c(&mut instr, c);
    Instruction::set_k(&mut instr, k);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.chunk.line_info.push(fs.lexer.line as u32);
    fs.pc += 1;
    pc
}

// Generate instruction with sJ field (for JMP)
pub fn code_asj(fs: &mut FuncState, op: OpCode, sj: i32) -> usize {
    let instr = Instruction::create_sj(op, sj);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.chunk.line_info.push(fs.lexer.line as u32);
    fs.pc += 1;
    pc
}

use crate::compiler::expression::{ExpDesc, ExpKind};

// Port of luaK_ret from lcode.c:207-214
// void luaK_ret (FuncState *fs, int first, int nret)
pub fn ret(fs: &mut FuncState, first: u8, nret: u8) -> usize {
    // Use optimized RETURN0/RETURN1 when possible
    let op = match nret {
        0 => OpCode::Return0,
        1 => OpCode::Return1,
        _ => OpCode::Return,
    };
    code_abc(fs, op, first as u32, nret.wrapping_add(1) as u32, 0)
}

// Port of luaK_finish from lcode.c:1847-1876
// void luaK_finish (FuncState *fs)
pub fn finish(fs: &mut FuncState) {
    let needclose = fs.needclose;
    let is_vararg = fs.is_vararg;
    let num_params = fs.chunk.param_count;
    
    for i in 0..fs.pc {
        let instr = &mut fs.chunk.code[i];
        let opcode = OpCode::from(Instruction::get_opcode(*instr));
        
        match opcode {
            OpCode::Return0 | OpCode::Return1 => {
                // lcode.c:1854-1859: Convert RETURN0/RETURN1 to RETURN if needed
                if needclose || is_vararg {
                    // Convert to RETURN
                    let a = Instruction::get_a(*instr);
                    let b = Instruction::get_b(*instr);
                    let mut new_instr = Instruction::create_abck(OpCode::Return, a, b, 0, false);
                    
                    // lcode.c:1861-1865: Set k and C fields
                    if needclose {
                        Instruction::set_k(&mut new_instr, true);
                    }
                    if is_vararg {
                        Instruction::set_c(&mut new_instr, (num_params + 1) as u32);
                    }
                    
                    *instr = new_instr;
                }
            }
            OpCode::Return | OpCode::TailCall => {
                // lcode.c:1861-1865: Set k and C fields for existing RETURN/TAILCALL
                if needclose {
                    Instruction::set_k(instr, true);
                }
                if is_vararg {
                    Instruction::set_c(instr, (num_params + 1) as u32);
                }
            }
            OpCode::Jmp => {
                // lcode.c:1867-1870: Fix jumps to final target
                let target = finaltarget(&fs.chunk.code, i);
                fixjump_at(fs, i, target);
            }
            _ => {}
        }
    }
}

// Helper for finish: find final target of a jump chain
fn finaltarget(code: &[u32], mut pc: usize) -> usize {
    let mut count = 0;
    while count < 100 {  // Prevent infinite loops
        if pc >= code.len() {
            break;
        }
        let instr = code[pc];
        if OpCode::from(Instruction::get_opcode(instr)) != OpCode::Jmp {
            break;
        }
        let offset = Instruction::get_sj(instr) as isize;
        if offset == -1 {
            break;
        }
        let next_pc = (pc as isize) + 1 + offset;
        if next_pc < 0 || next_pc >= code.len() as isize {
            break;
        }
        pc = next_pc as usize;
        count += 1;
    }
    pc
}

// Helper for finish: fix jump at specific pc
fn fixjump_at(fs: &mut FuncState, pc: usize, target: usize) {
    let offset = (target as isize) - (pc as isize) - 1;
    if offset < i32::MIN as isize || offset > i32::MAX as isize {
        return;
    }
    
    let instr = &mut fs.chunk.code[pc];
    Instruction::set_sj(instr, offset as i32);
}

// Port of luaK_jump from lcode.c:200-202
// int luaK_jump (FuncState *fs)
pub fn jump(fs: &mut FuncState) -> usize {
    code_asj(fs, OpCode::Jmp, -1)
}

// Port of luaK_getlabel from lcode.c:234-237
// int luaK_getlabel (FuncState *fs)
pub fn get_label(fs: &FuncState) -> usize {
    fs.pc
}

// Port of luaK_patchtohere from lcode.c:312-315
// void luaK_patchtohere (FuncState *fs, int list)
pub fn patchtohere(fs: &mut FuncState, list: isize) {
    let here = get_label(fs) as isize;
    patchlist(fs, list, here);
}

// Port of luaK_concat from lcode.c:174-186
// void luaK_concat (FuncState *fs, int *l1, int l2)
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

// Port of luaK_patchlist from lcode.c:307-310
// void luaK_patchlist (FuncState *fs, int list, int target)
pub fn patchlist(fs: &mut FuncState, list: isize, target: isize) {
    if target < 0 {
        return;
    }
    // lcode.c:309: patchlistaux(fs, list, target, NO_REG, target);
    patchlistaux(fs, list, target, NO_REG as u8, target);
}

// Helper: get jump target from instruction
// Port of getjump from lcode.c:259-265
fn get_jump(fs: &FuncState, pc: usize) -> isize {
    if pc >= fs.chunk.code.len() {
        return -1;
    }
    let instr = fs.chunk.code[pc];
    let offset = Instruction::get_sj(instr);
    if offset == -1 {
        -1
    } else {
        let target = (pc as isize) + 1 + (offset as isize);
        // Ensure target is valid (non-negative)
        if target < 0 { -1 } else { target }
    }
}

pub fn fix_jump(fs: &mut FuncState, pc: usize, target: usize) {
    if pc >= fs.chunk.code.len() {
        return;
    }
    let offset = (target as isize) - (pc as isize) - 1;
    let max_sj = (Instruction::MAX_SJ >> 1) as isize;
    if offset < -max_sj || offset > max_sj {
        // Error: jump too long
        return;
    }
    let instr = &mut fs.chunk.code[pc];
    Instruction::set_sj(instr, offset as i32);
}

// Port of need_value from lcode.c:900-908
// static int need_value (FuncState *fs, int list)
// Check whether list has any jump that do not produce a value
fn need_value(fs: &FuncState, mut list: isize) -> bool {
    const NO_JUMP: isize = -1;
    while list != NO_JUMP {
        let control_pc = get_jump_control(fs, list as usize);
        let i = fs.chunk.code[control_pc];
        let op = OpCode::from(Instruction::get_opcode(i));
        if op != OpCode::TestSet {
            return true;
        }
        list = get_jump(fs, list as usize);
    }
    false
}

// Port of code_loadbool from lcode.c:889-892
// static int code_loadbool (FuncState *fs, int A, OpCode op)
fn code_loadbool(fs: &mut FuncState, a: u8, op: OpCode) -> usize {
    get_label(fs); // those instructions may be jump targets
    code_abc(fs, op, a as u32, 0, 0)
}

// Port of patchtestreg from lcode.c:260-273
// static int patchtestreg (FuncState *fs, int node, int reg)
fn patchtestreg(fs: &mut FuncState, node: usize, reg: u8) -> bool {
    let pc = get_jump_control(fs, node);
    if pc >= fs.chunk.code.len() {
        return false;
    }
    
    let instr = fs.chunk.code[pc];
    if OpCode::from(Instruction::get_opcode(instr)) != OpCode::TestSet {
        return false;  // Not a TESTSET instruction
    }
    
    let b = Instruction::get_b(instr);
    if reg != NO_REG as u8 && reg != b as u8 {
        // Set destination register
        Instruction::set_a(&mut fs.chunk.code[pc], reg as u32);
    } else {
        // No register to put value or register already has the value
        // Change instruction to simple TEST
        let k = Instruction::get_k(instr);
        fs.chunk.code[pc] = Instruction::create_abck(OpCode::Test, b, 0, 0, k);
    }
    true
}

// Port of patchlistaux from lcode.c:271-292
// static void patchlistaux (FuncState *fs, int list, int vtarget, int reg, int dtarget)
// Port of patchlistaux from lcode.c:285-298
// Traverse a list of tests, patching their destination address and registers
// Tests producing values jump to 'vtarget' (and put their values in 'reg')
// Other tests jump to 'dtarget'
fn patchlistaux(fs: &mut FuncState, mut list: isize, vtarget: isize, reg: u8, dtarget: isize) {
    const NO_JUMP: isize = -1;
    while list != NO_JUMP {
        let next = get_jump(fs, list as usize);
        // lcode.c:293: if (patchtestreg(fs, list, reg))
        if patchtestreg(fs, list as usize, reg) {
            // lcode.c:294: fixjump(fs, list, vtarget);
            fix_jump(fs, list as usize, vtarget as usize);
        } else {
            // lcode.c:296: fixjump(fs, list, dtarget);
            fix_jump(fs, list as usize, dtarget as usize);
        }
        list = next;
    }
}

// Port of luaK_exp2nextreg from lcode.c:944-949
// void luaK_exp2nextreg (FuncState *fs, expdesc *e)
pub fn exp2nextreg(fs: &mut FuncState, e: &mut ExpDesc) -> u8 {
    discharge_vars(fs, e);
    free_exp(fs, e);
    reserve_regs(fs, 1);
    let reg = fs.freereg - 1;
    exp2reg(fs, e, reg);
    reg
}

// Port of luaK_exp2anyreg from lcode.c:956-972
// int luaK_exp2anyreg (FuncState *fs, expdesc *e)
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

// Port of exp2reg from lcode.c:915-938
// static void exp2reg (FuncState *fs, expdesc *e, int reg)
pub fn exp2reg(fs: &mut FuncState, e: &mut ExpDesc, reg: u8) {
    discharge2reg(fs, e, reg);
    if e.kind == ExpKind::VJMP {
        // lcode.c:918: expression itself is a test
        concat(fs, &mut e.t, unsafe { e.u.info as isize });
    }
    if e.has_jumps() {
        // lcode.c:919-934: need to patch jump lists
        let mut p_f = -1_isize; // position of an eventual LOAD false
        let mut p_t = -1_isize; // position of an eventual LOAD true

        if need_value(fs, e.t) || need_value(fs, e.f) {
            // lcode.c:922-926
            let fj = if e.kind == ExpKind::VJMP {
                -1
            } else {
                jump(fs) as isize
            };
            p_f = code_loadbool(fs, reg, OpCode::LFalseSkip) as isize;
            p_t = code_loadbool(fs, reg, OpCode::LoadTrue) as isize;
            patchtohere(fs, fj);
        }
        let final_label = get_label(fs) as isize;
        patchlistaux(fs, e.f, final_label, reg, p_f);
        patchlistaux(fs, e.t, final_label, reg, p_t);
    }
    e.f = -1;
    e.t = -1;
    e.u.info = reg as i32;
    e.kind = ExpKind::VNONRELOC;
}

// Port of luaK_dischargevars from lcode.c:766-817
// void luaK_dischargevars (FuncState *fs, expdesc *e)
pub fn discharge_vars(fs: &mut FuncState, e: &mut ExpDesc) {
    match e.kind {
        ExpKind::VCONST => {
            // Convert const variable to its actual value
            let vidx = unsafe { e.u.info } as usize;
            if let Some(var_desc) = fs.actvar.get(vidx) {
                if let Some(value) = var_desc.const_value {
                    // Convert the const value to an expression
                    const_to_exp(value, e);
                }
            }
        }
        ExpKind::VLOCAL => {
            e.u.info = unsafe { e.u.var.ridx as i32 };
            e.kind = ExpKind::VNONRELOC;
        }
        ExpKind::VUPVAL => {
            e.u.info = code_abc(fs, OpCode::GetUpval, 0, unsafe { e.u.info as u32 }, 0) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXUP => {
            e.u.info = code_abc(
                fs,
                OpCode::GetTabUp,
                0,
                unsafe { e.u.ind.t as u32 },
                unsafe { e.u.ind.idx as u32 },
            ) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXI => {
            free_reg(fs, unsafe { e.u.ind.t as u8 });
            e.u.info = code_abc(fs, OpCode::GetI, 0, unsafe { e.u.ind.t as u32 }, unsafe {
                e.u.ind.idx as u32
            }) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXSTR => {
            free_reg(fs, unsafe { e.u.ind.t as u8 });
            e.u.info = code_abc(
                fs,
                OpCode::GetField,
                0,
                unsafe { e.u.ind.t as u32 },
                unsafe { e.u.ind.idx as u32 },
            ) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXED => {
            free_reg(fs, unsafe { e.u.ind.idx as u8 });
            free_reg(fs, unsafe { e.u.ind.t as u8 });
            e.u.info = code_abc(
                fs,
                OpCode::GetTable,
                0,
                unsafe { e.u.ind.t as u32 },
                unsafe { e.u.ind.idx as u32 },
            ) as i32;
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VVARARG | ExpKind::VCALL => {
            setoneret(fs, e);
        }
        _ => {}
    }
}

// Port of discharge2reg from lcode.c:822-875
// static void discharge2reg (FuncState *fs, expdesc *e, int reg)
pub fn discharge2reg(fs: &mut FuncState, e: &mut ExpDesc, reg: u8) {
    discharge_vars(fs, e);
    match e.kind {
        ExpKind::VNIL => {
            // lcode.c:831: luaK_nil(fs, reg, 1)
            nil(fs, reg, 1); // 1 register
        }
        ExpKind::VFALSE => {
            // lcode.c:832
            code_abc(fs, OpCode::LoadFalse, reg as u32, 0, 0);
        }
        ExpKind::VTRUE => {
            // lcode.c:835
            code_abc(fs, OpCode::LoadTrue, reg as u32, 0, 0);
        }
        ExpKind::VKSTR => {
            // lcode.c:838-839: str2K(fs, e); FALLTHROUGH to VK
            str2k(fs, e);
            // Now fall through to VK case
            code_abx(fs, OpCode::LoadK, reg as u32, unsafe { e.u.info as u32 });
        }
        ExpKind::VK => {
            // lcode.c:841: luaK_codek(fs, reg, e->u.info);
            code_abx(fs, OpCode::LoadK, reg as u32, unsafe { e.u.info as u32 });
        }
        ExpKind::VKFLT => {
            // lcode.c:844: luaK_float(fs, reg, e->u.nval);
            // Use LoadF for floats
            let val = unsafe { e.u.nval };
            let k_idx = number_k(fs, val);
            code_abx(fs, OpCode::LoadK, reg as u32, k_idx as u32);
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

// Port of freeexp from lcode.c:558-561
// static void freeexp (FuncState *fs, expdesc *e)
pub fn free_exp(fs: &mut FuncState, e: &ExpDesc) {
    if e.kind == ExpKind::VNONRELOC {
        free_reg(fs, unsafe { e.u.info as u8 });
    }
}

// Port of freeexps from lcode.c:567-572
// Free registers used by expressions 'e1' and 'e2' (if any) in proper order
fn free_exps(fs: &mut FuncState, e1: &ExpDesc, e2: &ExpDesc) {
    let r1 = if e1.kind == ExpKind::VNONRELOC {
        unsafe { e1.u.info }
    } else {
        -1
    };
    let r2 = if e2.kind == ExpKind::VNONRELOC {
        unsafe { e2.u.info }
    } else {
        -1
    };

    // Free in proper order (higher register first)
    if r1 > r2 {
        if r1 >= 0 {
            free_reg(fs, r1 as u8);
        }
        if r2 >= 0 {
            free_reg(fs, r2 as u8);
        }
    } else {
        if r2 >= 0 {
            free_reg(fs, r2 as u8);
        }
        if r1 >= 0 {
            free_reg(fs, r1 as u8);
        }
    }
}

// Port of freereg from lcode.c:527-531
// static void freereg (FuncState *fs, int reg)
pub fn free_reg(fs: &mut FuncState, reg: u8) {
    if reg >= fs.nactvar && reg < fs.freereg {
        fs.freereg -= 1;
    }
}

// Port of luaK_reserveregs from lcode.c:511-516
// void luaK_reserveregs (FuncState *fs, int n)
pub fn reserve_regs(fs: &mut FuncState, n: u8) {
    fs.freereg = fs.freereg.saturating_add(n);
    if (fs.freereg as usize) > fs.chunk.max_stack_size {
        fs.chunk.max_stack_size = fs.freereg as usize;
    }
}

// Port of luaK_nil from lcode.c:136-155
// void luaK_nil (FuncState *fs, int from, int n)
// NOTE: Simplified version without OP_LOADNIL optimization
pub fn nil(fs: &mut FuncState, from: u8, n: u8) {
    if n > 0 {
        code_abc(fs, OpCode::LoadNil, from as u32, (n - 1) as u32, 0);
    }
}

// Port of luaK_setoneret from lcode.c:755-765
// void luaK_setoneret (FuncState *fs, expdesc *e)
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

// Port of luaK_setreturns from lcode.c:722-732
// void luaK_setreturns (FuncState *fs, expdesc *e, int nresults)
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

// Port of tonumeral from lcode.c:59-73
// static int tonumeral (const expdesc *e, TValue *v)
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
use crate::StringId;

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

// Port of stringK from lcode.c:576-580
// static int stringK (FuncState *fs, TString *s)
pub fn string_k(fs: &mut FuncState, s: String) -> usize {
    // Intern string to ObjectPool and get StringId
    let (string_id, _) = fs.pool.create_string(&s);

    // Add LuaValue with StringId to constants (check for duplicates)
    let value = LuaValue::string(string_id);
    add_constant(fs, value)
}

// Port of str2K from lcode.c:738-742
// static void str2K (FuncState *fs, expdesc *e)
// Convert a VKSTR to a VK
// static void str2K (FuncState *fs, expdesc *e) {
//   lua_assert(e->k == VKSTR);
//   e->u.info = stringK(fs, e->u.strval);
//   e->k = VK;
// }
fn str2k(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.kind == ExpKind::VKSTR {
        // String is already in constants (stored in e.u.info), just change kind
        e.kind = ExpKind::VK;
        let string_id = StringId(unsafe { e.u.info as u32 });
        let value = LuaValue::string(string_id);
        let const_idx = add_constant(fs, value);
        e.u.info = const_idx as i32;
    }
}

// Helper to add constant to chunk
fn add_constant(fs: &mut FuncState, value: LuaValue) -> usize {
    // Try to find existing constant
    if let Some(idx) = fs.chunk_constants_map.get(&value) {
        return *idx;
    }
    // Add new constant
    fs.chunk.constants.push(value);
    let idx = fs.chunk.constants.len() - 1;
    fs.chunk_constants_map.insert(value, idx);
    idx
}

// Port of luaK_exp2K from lcode.c:1000-1026
// static int luaK_exp2K (FuncState *fs, expdesc *e)
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

// Port of exp2RK from lcode.c:1036-1042
// static int exp2RK (FuncState *fs, expdesc *e)
fn exp2rk(fs: &mut FuncState, e: &mut ExpDesc) -> bool {
    if exp2k(fs, e) {
        true
    } else {
        exp2anyreg(fs, e);
        false
    }
}

// Port of getjumpcontrol from lcode.c:245-250
// static Instruction *getjumpcontrol (FuncState *fs, int pc)
fn get_jump_control(fs: &FuncState, pc: usize) -> usize {
    if pc >= 1 && pc < fs.chunk.code.len() {
        let prev_instr = fs.chunk.code[pc - 1];
        let prev_op = OpCode::from(Instruction::get_opcode(prev_instr));
        // Check if previous instruction is a test mode instruction
        if matches!(
            prev_op,
            OpCode::Test
                | OpCode::TestSet
                | OpCode::Eq
                | OpCode::Lt
                | OpCode::Le
                | OpCode::EqK
                | OpCode::EqI
                | OpCode::LtI
                | OpCode::LeI
                | OpCode::GtI
                | OpCode::GeI
        ) {
            return pc - 1;
        }
    }
    pc
}

// Port of negatecondition from lcode.c:1103-1108
// static void negatecondition (FuncState *fs, expdesc *e)
fn negatecondition(fs: &mut FuncState, e: &mut ExpDesc) {
    let pc = get_jump_control(fs, unsafe { e.u.info as usize });
    if pc < fs.chunk.code.len() {
        let instr = &mut fs.chunk.code[pc];
        let k = Instruction::get_k(*instr);
        Instruction::set_k(instr, !k);
    }
}

// Port of condjump from lcode.c:223-226
// static int condjump (FuncState *fs, OpCode op, int A, int B, int C, int k)
fn condjump(fs: &mut FuncState, op: OpCode, a: u32, b: u32, c: u32, k: bool) -> isize {
    code_abck(fs, op, a, b, c, k);
    jump(fs) as isize
}

// Port of discharge2anyreg from lcode.c:882-886
// static void discharge2anyreg (FuncState *fs, expdesc *e)
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

// Port of jumponcond from lcode.c:1118-1130
// static int jumponcond (FuncState *fs, expdesc *e, int cond)
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
    condjump(
        fs,
        OpCode::TestSet,
        NO_REG,
        unsafe { e.u.info as u32 },
        0,
        cond,
    )
}

// Port of luaK_goiftrue from lcode.c:1135-1160
// void luaK_goiftrue (FuncState *fs, expdesc *e)
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

// Port of luaK_goiffalse from lcode.c:1165-1183
// void luaK_goiffalse (FuncState *fs, expdesc *e)
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

// Port of luaK_infix from lcode.c:1637-1678
// void luaK_infix (FuncState *fs, BinOpr op, expdesc *v)
pub fn infix(fs: &mut FuncState, op: BinaryOperator, v: &mut ExpDesc) {
    discharge_vars(fs, v);
    match op {
        BinaryOperator::OpAnd => {
            goiftrue(fs, v);
        }
        BinaryOperator::OpOr => {
            goiffalse(fs, v);
        }
        BinaryOperator::OpConcat => {
            exp2nextreg(fs, v);
        }
        BinaryOperator::OpAdd
        | BinaryOperator::OpSub
        | BinaryOperator::OpMul
        | BinaryOperator::OpDiv
        | BinaryOperator::OpIDiv
        | BinaryOperator::OpMod
        | BinaryOperator::OpPow
        | BinaryOperator::OpBAnd
        | BinaryOperator::OpBOr
        | BinaryOperator::OpBXor
        | BinaryOperator::OpShl
        | BinaryOperator::OpShr => {
            if !tonumeral(v, None) {
                exp2anyreg(fs, v);
            }
        }
        BinaryOperator::OpEq | BinaryOperator::OpNe => {
            if !tonumeral(v, None) {
                exp2rk(fs, v);
            }
        }
        BinaryOperator::OpLt
        | BinaryOperator::OpLe
        | BinaryOperator::OpGt
        | BinaryOperator::OpGe => {
            if !tonumeral(v, None) {
                exp2anyreg(fs, v);
            }
        }
        BinaryOperator::OpNop => {}
    }
}

// Port of swapexps from lcode.c:1588-1592
fn swapexps(e1: &mut ExpDesc, e2: &mut ExpDesc) {
    std::mem::swap(e1, e2);
}

// Port of codeconcat from lcode.c:1686-1698
// Create code for '(e1 .. e2)'
fn codeconcat(fs: &mut FuncState, e1: &mut ExpDesc, e2: &mut ExpDesc) {
    // Check if previous instruction is CONCAT to merge multiple concatenations
    if fs.pc > 0 {
        let prev_pc = fs.pc - 1;
        let prev_instr = fs.chunk.code[prev_pc];
        if OpCode::from(Instruction::get_opcode(prev_instr)) == OpCode::Concat {
            let n = Instruction::get_b(prev_instr);
            let a = Instruction::get_a(prev_instr);
            if unsafe { e1.u.info as u32 } + 1 == a {
                free_exp(fs, e2);
                Instruction::set_a(&mut fs.chunk.code[prev_pc], unsafe { e1.u.info as u32 });
                Instruction::set_b(&mut fs.chunk.code[prev_pc], n + 1);
                return;
            }
        }
    }
    // New concat opcode
    code_abc(fs, OpCode::Concat, unsafe { e1.u.info as u32 }, 2, 0);
    free_exp(fs, e2);
}

// Port of codeeq from lcode.c:1585-1612
// Emit code for equality comparisons ('==', '~=')
// 'e1' was already put as RK by 'luaK_infix'
fn codeeq(fs: &mut FuncState, op: BinaryOperator, e1: &mut ExpDesc, e2: &mut ExpDesc) {
    let r1: u32;
    let r2: u32;
    let mut im: i32 = 0;
    let mut isfloat: bool = false;
    let opcode: OpCode;

    // If e1 is not in a register, swap operands
    if e1.kind != ExpKind::VNONRELOC {
        // e1 is VK, VKINT, or VKFLT
        swapexps(e1, e2);
    }

    // First expression must be in register
    r1 = exp2anyreg(fs, e1) as u32;

    // Check if e2 is a small constant number (fits in sC)
    if is_scnumber(e2, &mut im, &mut isfloat) {
        opcode = OpCode::EqI;
        r2 = im as u32; // immediate operand
    }
    // Check if e2 is a constant (can use K operand)
    else if exp2rk(fs, e2) {
        opcode = OpCode::EqK;
        r2 = unsafe { e2.u.info as u32 }; // constant index
    }
    // Regular case: compare two registers
    else {
        opcode = OpCode::Eq;
        r2 = exp2anyreg(fs, e2) as u32;
    }

    free_exps(fs, e1, e2);

    let k = op == BinaryOperator::OpEq;
    let pc = condjump(fs, opcode, r1, r2, isfloat as u32, k);

    e1.u.info = pc as i32;
    e1.kind = ExpKind::VJMP;
}

// Port of codeorder from lcode.c:1553-1581
// Emit code for order comparisons. When using an immediate operand,
// 'isfloat' tells whether the original value was a float.
fn codeorder(fs: &mut FuncState, op: BinaryOperator, e1: &mut ExpDesc, e2: &mut ExpDesc) {
    let r1: u32;
    let r2: u32;
    let mut im: i32 = 0;
    let mut isfloat: bool = false;
    let opcode: OpCode;

    // Check if e2 is a small constant (can use immediate)
    if is_scnumber(e2, &mut im, &mut isfloat) {
        // Use immediate operand
        r1 = exp2anyreg(fs, e1) as u32;
        r2 = im as u32;
        // Map operator to immediate version: < -> LTI, <= -> LEI
        opcode = match op {
            BinaryOperator::OpLt => OpCode::LtI,
            BinaryOperator::OpLe => OpCode::LeI,
            _ => unreachable!(),
        };
    }
    // Check if e1 is a small constant (transform and swap)
    else if is_scnumber(e1, &mut im, &mut isfloat) {
        // Transform (A < B) to (B > A) and (A <= B) to (B >= A)
        r1 = exp2anyreg(fs, e2) as u32;
        r2 = im as u32;
        opcode = match op {
            BinaryOperator::OpLt => OpCode::GtI, // A < B => B > A
            BinaryOperator::OpLe => OpCode::GeI, // A <= B => B >= A
            _ => unreachable!(),
        };
    }
    // Regular case: compare two registers
    else {
        r1 = exp2anyreg(fs, e1) as u32;
        r2 = exp2anyreg(fs, e2) as u32;
        opcode = match op {
            BinaryOperator::OpLt => OpCode::Lt,
            BinaryOperator::OpLe => OpCode::Le,
            _ => unreachable!(),
        };
    }

    free_exps(fs, e1, e2);

    let pc = condjump(fs, opcode, r1, r2, isfloat as u32, true);

    e1.u.info = pc as i32;
    e1.kind = ExpKind::VJMP;
}

// Check if operator is foldable (arithmetic or bitwise)
// Port of foldbinop macro from lcode.h:45
fn foldbinop(op: BinaryOperator) -> bool {
    use BinaryOperator::*;
    matches!(op, OpAdd | OpSub | OpMul | OpDiv | OpIDiv | OpMod | OpPow | OpBAnd | OpBOr | OpBXor | OpShl | OpShr)
}

// Check if folding operation is valid and won't raise errors
// Port of validop from lcode.c:1316-1330
fn validop(op: BinaryOperator, v1: f64, i1: i64, is_int1: bool, v2: f64, i2: i64, is_int2: bool) -> bool {
    use BinaryOperator::*;
    match op {
        // Bitwise operations need integer-convertible operands
        OpBAnd | OpBOr | OpBXor | OpShl | OpShr => {
            // Check both operands are integers or convertible to integers
            is_int1 && is_int2
        }
        // Division operations cannot have 0 divisor
        OpDiv | OpIDiv | OpMod => {
            if is_int2 {
                i2 != 0
            } else {
                v2 != 0.0
            }
        }
        _ => true, // everything else is valid
    }
}

// Try to constant-fold a binary operation
// Port of constfolding from lcode.c:1337-1356
fn constfolding(fs: &FuncState, op: BinaryOperator, e1: &mut ExpDesc, e2: &ExpDesc) -> bool {
    use BinaryOperator::*;
    use ExpKind::{VKFLT, VKINT};
    
    // Check if both operands are numeric constants
    let (v1, i1, is_int1) = match e1.kind {
        VKINT => (unsafe { e1.u.ival } as f64, unsafe { e1.u.ival }, true),
        VKFLT => {
            let f = unsafe { e1.u.nval };
            // Try to convert float to integer for bitwise ops
            let as_int = f as i64;
            let is_int = (as_int as f64) == f;
            (f, as_int, is_int)
        }
        _ => return false,
    };
    
    let (v2, i2, is_int2) = match e2.kind {
        VKINT => (unsafe { e2.u.ival } as f64, unsafe { e2.u.ival }, true),
        VKFLT => {
            let f = unsafe { e2.u.nval };
            let as_int = f as i64;
            let is_int = (as_int as f64) == f;
            (f, as_int, is_int)
        }
        _ => return false,
    };
    
    // Check if operation is valid (no division by zero, etc.)
    if !validop(op, v1, i1, is_int1, v2, i2, is_int2) {
        return false;
    }
    
    // Perform the operation
    let result = match op {
        OpAdd => {
            if is_int1 && is_int2 {
                // Integer addition
                if let Some(res) = i1.checked_add(i2) {
                    e1.kind = VKINT;
                    e1.u.ival = res;
                    return true;
                } else {
                    // Overflow, fallback to float
                    v1 + v2
                }
            } else {
                v1 + v2
            }
        }
        OpSub => {
            if is_int1 && is_int2 {
                if let Some(res) = i1.checked_sub(i2) {
                    e1.kind = VKINT;
                    e1.u.ival = res;
                    return true;
                } else {
                    v1 - v2
                }
            } else {
                v1 - v2
            }
        }
        OpMul => {
            if is_int1 && is_int2 {
                if let Some(res) = i1.checked_mul(i2) {
                    e1.kind = VKINT;
                    e1.u.ival = res;
                    return true;
                } else {
                    v1 * v2
                }
            } else {
                v1 * v2
            }
        }
        OpDiv => v1 / v2,
        OpIDiv => {
            if is_int1 && is_int2 {
                if let Some(res) = i1.checked_div(i2) {
                    e1.kind = VKINT;
                    e1.u.ival = res;
                    return true;
                }
            }
            (v1 / v2).floor()
        }
        OpMod => {
            if is_int1 && is_int2 {
                e1.kind = VKINT;
                e1.u.ival = i1.rem_euclid(i2);
                return true;
            } else {
                v1 - (v1 / v2).floor() * v2
            }
        }
        OpPow => v1.powf(v2),
        OpBAnd => {
            e1.kind = VKINT;
            e1.u.ival = i1 & i2;
            return true;
        }
        OpBOr => {
            e1.kind = VKINT;
            e1.u.ival = i1 | i2;
            return true;
        }
        OpBXor => {
            e1.kind = VKINT;
            e1.u.ival = i1 ^ i2;
            return true;
        }
        OpShl => {
            e1.kind = VKINT;
            e1.u.ival = if i2 >= 0 {
                i1.wrapping_shl(i2 as u32)
            } else {
                i1.wrapping_shr((-i2) as u32)
            };
            return true;
        }
        OpShr => {
            e1.kind = VKINT;
            e1.u.ival = if i2 >= 0 {
                i1.wrapping_shr(i2 as u32)
            } else {
                i1.wrapping_shl((-i2) as u32)
            };
            return true;
        }
        _ => return false,
    };
    
    // For float results, check for NaN and -0.0
    if result.is_nan() || result == 0.0 && result.is_sign_negative() {
        return false;
    }
    
    e1.kind = VKFLT;
    e1.u.nval = result;
    true
}

// Port of luaK_posfix from lcode.c:1706-1783
// void luaK_posfix (FuncState *fs, BinOpr opr, expdesc *e1, expdesc *e2, int line)
pub fn posfix(fs: &mut FuncState, op: BinaryOperator, e1: &mut ExpDesc, e2: &mut ExpDesc) {
    use BinaryOperator;

    discharge_vars(fs, e2);
    
    // Try constant folding first (lcode.c:1709)
    if foldbinop(op) && constfolding(fs, op, e1, e2) {
        return; // done by folding
    }

    match op {
        // lcode.c:1711-1715: OPR_AND
        BinaryOperator::OpAnd => {
            // e1.t should be NO_JUMP (closed by luaK_goiftrue in infix)
            concat(fs, &mut e2.f, e1.f);
            *e1 = e2.clone();
        }
        // lcode.c:1716-1720: OPR_OR
        BinaryOperator::OpOr => {
            // e1.f should be NO_JUMP (closed by luaK_goiffalse in infix)
            concat(fs, &mut e2.t, e1.t);
            *e1 = e2.clone();
        }
        // lcode.c:1721-1724: OPR_CONCAT
        BinaryOperator::OpConcat => {
            exp2nextreg(fs, e2);
            codeconcat(fs, e1, e2);
        }
        // lcode.c:1762-1764: OPR_EQ, OPR_NE
        BinaryOperator::OpEq | BinaryOperator::OpNe => {
            codeeq(fs, op, e1, e2);
        }
        // lcode.c:1765-1775: OPR_GT, OPR_GE (convert to LT, LE by swapping)
        BinaryOperator::OpGt | BinaryOperator::OpGe => {
            // '(a > b)' <=> '(b < a)';  '(a >= b)' <=> '(b <= a)'
            swapexps(e1, e2);
            let new_op = if op == BinaryOperator::OpGt {
                BinaryOperator::OpLt
            } else {
                BinaryOperator::OpLe
            };
            codeorder(fs, new_op, e1, e2);
        }
        // lcode.c:1776-1778: OPR_LT, OPR_LE
        BinaryOperator::OpLt | BinaryOperator::OpLe => {
            codeorder(fs, op, e1, e2);
        }
        // All other arithmetic/bitwise operators
        _ => {
            // Try to use K operand optimization (codearith behavior)
            if exp2k(fs, e2) {
                // e2 is a K operand, generate K-series instruction
                let k_idx = unsafe { e2.u.info };
                let r1 = exp2anyreg(fs, e1);

                // Determine the K-series opcode
                let opcode = match op {
                    BinaryOperator::OpAdd => OpCode::AddK,
                    BinaryOperator::OpSub => OpCode::SubK,
                    BinaryOperator::OpMul => OpCode::MulK,
                    BinaryOperator::OpDiv => OpCode::DivK,
                    BinaryOperator::OpIDiv => OpCode::IDivK,
                    BinaryOperator::OpMod => OpCode::ModK,
                    BinaryOperator::OpPow => OpCode::PowK,
                    BinaryOperator::OpBAnd => OpCode::BAndK,
                    BinaryOperator::OpBOr => OpCode::BOrK,
                    BinaryOperator::OpBXor => OpCode::BXorK,
                    _ => {
                        // No K version, fall back to normal instruction
                        let o2 = exp2anyreg(fs, e2);
                        free_reg(fs, o2);
                        free_reg(fs, r1);

                        let res = fs.freereg;
                        reserve_regs(fs, 1);

                        let opcode = match op {
                            BinaryOperator::OpShl => OpCode::Shl,
                            BinaryOperator::OpShr => OpCode::Shr,
                            _ => unreachable!("Invalid operator"),
                        };

                        code_abc(fs, opcode, res as u32, r1 as u32, o2 as u32);
                        e1.kind = ExpKind::VNONRELOC;
                        e1.u.info = res as i32;
                        return;
                    }
                };

                // Port of finishbinexpval from lcode.c:1407-1418
                // Generate K-series instruction with A=0, will be fixed by discharge2reg
                let pc = code_abc(fs, opcode, 0, r1 as u32, k_idx as u32);
                
                // Free both operands (freeexps)
                free_exp(fs, e1);
                free_exp(fs, e2);
                
                // Mark as relocatable - target register will be decided later
                e1.kind = ExpKind::VRELOC;
                e1.u.info = pc as i32;
                
                // Generate metamethod fallback instruction (MMBINK)
                // Like finishbinexpval in lcode.c:1416
                // TM events from ltm.h (TM_ADD=6, TM_SUB=7, etc.)
                let tm_event = match op {
                    BinaryOperator::OpAdd => TmKind::Add, // TM_ADD
                    BinaryOperator::OpSub => TmKind::Sub,   // TM_SUB
                    BinaryOperator::OpMul => TmKind::Mul,   // TM_MUL
                    BinaryOperator::OpMod => TmKind::Mod,   // TM_MOD
                    BinaryOperator::OpPow => TmKind::Pow,  // TM_POW
                    BinaryOperator::OpDiv => TmKind::Div,  // TM_DIV
                    BinaryOperator::OpIDiv => TmKind::IDiv, // TM_IDIV
                    BinaryOperator::OpBAnd => TmKind::Band, // TM_BAND
                    BinaryOperator::OpBOr => TmKind::Bor,  // TM_BOR
                    BinaryOperator::OpBXor => TmKind::Bxor, // TM_BXOR
                    _ => TmKind::N, // Invalid for other ops
                };
                code_abc(fs, OpCode::MmBinK, r1 as u32, k_idx as u32, tm_event as u32);
            } else {
                // Both operands in registers - port of codebinexpval (lcode.c:1425-1434)
                let o2 = exp2anyreg(fs, e2);
                
                // Determine instruction opcode
                let opcode = match op {
                    BinaryOperator::OpAdd => OpCode::Add,
                    BinaryOperator::OpSub => OpCode::Sub,
                    BinaryOperator::OpMul => OpCode::Mul,
                    BinaryOperator::OpDiv => OpCode::Div,
                    BinaryOperator::OpIDiv => OpCode::IDiv,
                    BinaryOperator::OpMod => OpCode::Mod,
                    BinaryOperator::OpPow => OpCode::Pow,
                    BinaryOperator::OpShl => OpCode::Shl,
                    BinaryOperator::OpShr => OpCode::Shr,
                    BinaryOperator::OpBAnd => OpCode::BAnd,
                    BinaryOperator::OpBOr => OpCode::BOr,
                    BinaryOperator::OpBXor => OpCode::BXor,
                    _ => unreachable!("Invalid operator for opcode generation"),
                };

                // Port of finishbinexpval from lcode.c:1407-1418
                // Generate instruction with A=0, will be fixed by discharge2reg
                let o1 = exp2anyreg(fs, e1);
                let pc = code_abc(fs, opcode, 0, o1 as u32, o2 as u32);
                
                // Free both operands (freeexps)
                free_exp(fs, e1);
                free_exp(fs, e2);
                
                // Mark as relocatable
                e1.kind = ExpKind::VRELOC;
                e1.u.info = pc as i32;
                
                // Generate metamethod fallback instruction (MMBIN)
                // Like finishbinexpval in lcode.c:1416
                let tm_event = match op {
                    BinaryOperator::OpAdd => TmKind::Add,
                    BinaryOperator::OpSub => TmKind::Sub,
                    BinaryOperator::OpMul => TmKind::Mul,
                    BinaryOperator::OpMod => TmKind::Mod,
                    BinaryOperator::OpPow => TmKind::Pow,
                    BinaryOperator::OpDiv => TmKind::Div,
                    BinaryOperator::OpIDiv => TmKind::IDiv,
                    BinaryOperator::OpBAnd => TmKind::Band,
                    BinaryOperator::OpBOr => TmKind::Bor,
                    BinaryOperator::OpBXor => TmKind::Bxor,
                    BinaryOperator::OpShl => TmKind::Shl,
                    BinaryOperator::OpShr => TmKind::Shr,
                    _ => TmKind::N,
                };
                code_abc(fs, OpCode::MmBin, o1 as u32, o2 as u32, tm_event as u32);
            }
        }
    }
}

// Port of codenot from lcode.c:1188-1214
// static void codenot (FuncState *fs, expdesc *e)
fn codenot(fs: &mut FuncState, e: &mut ExpDesc) {
    discharge_vars(fs, e);
    match e.kind {
        ExpKind::VNIL | ExpKind::VFALSE => {
            // true == not nil == not false (lcode.c:1190)
            e.kind = ExpKind::VTRUE;
        }
        ExpKind::VK | ExpKind::VKFLT | ExpKind::VKINT | ExpKind::VKSTR | ExpKind::VTRUE => {
            // false == not "x" == not 0.5 == not 1 == not true (lcode.c:1194)
            e.kind = ExpKind::VFALSE;
        }
        ExpKind::VJMP => {
            // Negate the condition (lcode.c:1197)
            negatecondition(fs, e);
        }
        ExpKind::VRELOC | ExpKind::VNONRELOC => {
            // Generate NOT instruction (lcode.c:1200-1206)
            discharge2anyreg(fs, e);
            free_exp(fs, e);
            let pc = code_abc(fs, OpCode::Not, 0, unsafe { e.u.info as u32 }, 0);
            e.u.info = pc as i32;
            e.kind = ExpKind::VRELOC;
        }
        _ => {} // Should not happen
    }
}

// Simplified implementation of luaK_prefix - generate unary operation
pub fn prefix(fs: &mut FuncState, op: OpCode, e: &mut ExpDesc) {
    discharge_vars(fs, e);

    // Special handling for NOT (lcode.c:1627)
    if op == OpCode::Not {
        codenot(fs, e);
        return;
    }

    let o = exp2anyreg(fs, e);
    free_reg(fs, o);

    let res = fs.freereg;
    reserve_regs(fs, 1);
    code_abc(fs, op, res as u32, o as u32, 0);

    e.kind = ExpKind::VNONRELOC;
    e.u.info = res as i32;
}

// Port of luaK_exp2anyregup from lcode.c:978-981
// void luaK_exp2anyregup (FuncState *fs, expdesc *e)
pub fn exp2anyregup(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.kind != ExpKind::VUPVAL || e.has_jumps() {
        exp2anyreg(fs, e);
    }
}

// Port of luaK_exp2val from lcode.c:988-993
// void luaK_exp2val (FuncState *fs, expdesc *e)
pub fn exp2val(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.kind == ExpKind::VJMP || e.has_jumps() {
        exp2anyreg(fs, e);
    } else {
        discharge_vars(fs, e);
    }
}

// Port of luaK_indexed from lcode.c:1280-1310
// void luaK_indexed (FuncState *fs, expdesc *t, expdesc *k)
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

// Port of luaK_self from lcode.c:1087-1097
// void luaK_self (FuncState *fs, expdesc *e, expdesc *key)
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
    // LUA_MULTRET = -1, which becomes 255 in u8
    // setreturns will do nresults+1, so 255+1=0 in u8 (wrapping), meaning multret
    setreturns(fs, e, 255);
}

// Port of luaK_fixline from lcode.c:1787-1790
// void luaK_fixline (FuncState *fs, int line)
pub fn fixline(fs: &mut FuncState, line: usize) {
    // Remove last line info and add it again with new line
    // For now, we just ensure line_info has correct size
    if fs.chunk.line_info.len() > 0 {
        let last_idx = fs.chunk.line_info.len() - 1;
        fs.chunk.line_info[last_idx] = line as u32;
    }
}

// Port of luaK_exp2const from lcode.c:85-108
// int luaK_exp2const (FuncState *fs, const expdesc *e, TValue *v)
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
            // Get from actvar array (port of const2val from lcode.c:75-78)
            let vidx = unsafe { e.u.info } as usize;
            if let Some(var_desc) = fs.actvar.get(vidx) {
                var_desc.const_value
            } else {
                None
            }
        }
        _ => None,
    }
}

// LUA_MULTRET constant from lua.h
pub const LUA_MULTRET: u32 = u32::MAX;

// Port of hasmultret from lcode.c:86-90
pub fn hasmultret(e: &ExpDesc) -> bool {
    matches!(e.kind, ExpKind::VCALL | ExpKind::VVARARG)
}

// Port of luaK_setlist from lcode.c:1810-1823
pub fn setlist(fs: &mut FuncState, base: u8, nelems: u32, tostore: u32) {
    debug_assert!(tostore != 0);

    let c = if tostore == LUA_MULTRET { 0 } else { tostore };

    const MAXARG_C: u32 = 255; // 8-bit C field

    if nelems <= MAXARG_C {
        code_abc(fs, OpCode::SetList, base as u32, c, nelems);
    } else {
        // Need extra argument for large index
        let extra = nelems / (MAXARG_C + 1);
        let c_arg = nelems % (MAXARG_C + 1);
        code_abck(fs, OpCode::SetList, base as u32, c, c_arg, true);
        code_extraarg(fs, extra);
    }

    fs.freereg = base + 1; // free registers with list values
}

// Port of luaK_settablesize from lcode.c:1793-1801
// void luaK_settablesize (FuncState *fs, int pc, int ra, int asize, int hsize)
pub fn settablesize(fs: &mut FuncState, pc: usize, ra: u8, asize: u32, hsize: u32) {
    const MAXARG_C: u32 = 0xFF; // Maximum value for C field
    
    // B field: hash size (lcode.c:1795)
    // rb = (hsize != 0) ? luaO_ceillog2(hsize) + 1 : 0
    let rb = if hsize != 0 {
        // Ceiling of log2(hsize) + 1
        let bits = 32 - (hsize - 1).leading_zeros();
        bits + 1
    } else {
        0
    };

    // C field: lower bits of array size (lcode.c:1797)
    // rc = asize % (MAXARG_C + 1)
    let rc = asize % (MAXARG_C + 1);
    
    // EXTRAARG: higher bits of array size (lcode.c:1796)
    // extra = asize / (MAXARG_C + 1)
    let extra = asize / (MAXARG_C + 1);
    
    // k flag: true if needs EXTRAARG (lcode.c:1798)
    let k = extra > 0;

    // Update the NEWTABLE instruction (lcode.c:1799)
    let inst = &mut fs.chunk.code[pc];
    let opcode = Instruction::get_opcode(*inst);
    debug_assert_eq!(opcode, OpCode::NewTable);

    *inst = Instruction::create_abck(OpCode::NewTable, ra as u32, rb, rc, k);
    
    // Update EXTRAARG instruction (lcode.c:1800)
    // *(inst + 1) = CREATE_Ax(OP_EXTRAARG, extra);
    if pc + 1 < fs.chunk.code.len() {
        fs.chunk.code[pc + 1] = Instruction::create_ax(OpCode::ExtraArg, extra);
    }
}

// Port of luaK_codeextraarg from lcode.c:415-419
pub fn code_extraarg(fs: &mut FuncState, a: u32) -> usize {
    let inst = Instruction::create_ax(OpCode::ExtraArg, a);
    let pc = fs.pc;
    fs.chunk.code.push(inst);
    fs.chunk.line_info.push(fs.lexer.line as u32);
    fs.pc += 1;
    pc
}
