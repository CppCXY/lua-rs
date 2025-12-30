// Code generation - Port from lcode.c (Lua 5.4.8)
// This file corresponds to lua-5.5.0/src/lcode.c
use crate::compiler::func_state::FuncState;
use crate::compiler::parser::BinaryOperator;
use crate::compiler::tm_kind::TmKind;
use crate::compiler::{ExpUnion, IndVars};
use crate::lua_value::LuaValueKind;
use crate::lua_vm::{Instruction, OpCode};

// Port of int2sC from lcode.c (macro)
// Convert integer to sC format (with OFFSET_sC = 128)
fn int2sc(i: i32) -> u32 {
    ((i as u32).wrapping_add(127)) & 0xFF
}

// Port of fitsC from lcode.c:660-662
// Check whether 'i' can be stored in an 'sC' operand
fn fits_c(i: i64) -> bool {
    let offset_sc = 127i64;
    let max_arg_c = 255u32; // MAXARG_C = 255 for 8-bit C field
    (i.wrapping_add(offset_sc) as u64) <= (max_arg_c as u64)
}

// Port of isSCnumber from lcode.c:1257-1271
// Check whether expression 'e' is a literal integer or float in proper range to fit in sC
fn is_scnumber(e: &ExpDesc, pi: &mut i32, isfloat: &mut bool) -> bool {
    let i = match e.kind {
        ExpKind::VKINT => e.u.ival(),
        ExpKind::VKFLT => {
            // Try to convert float to integer
            let fval = e.u.nval();
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
pub fn const_to_exp(value: LuaValue, e: &mut ExpDesc) {
    match value.kind() {
        LuaValueKind::Integer => {
            e.kind = ExpKind::VKINT;
            e.u = ExpUnion::IVal(value.as_integer().unwrap_or(0));
        }
        LuaValueKind::Float => {
            e.kind = ExpKind::VKFLT;
            e.u = ExpUnion::NVal(value.as_float().unwrap_or(0.0));
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
            e.u = ExpUnion::Str(value.as_string_id().unwrap_or(StringId::short(0)));
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
    fs.chunk.line_info.push(fs.lexer.lastline as u32); // Use lastline (lcode.c:389)
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
    fs.chunk.line_info.push(fs.lexer.lastline as u32); // Use lastline (lcode.c:390)
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
    fs.chunk.line_info.push(fs.lexer.lastline as u32); // Use lastline
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
    fs.chunk.line_info.push(fs.lexer.lastline as u32); // Use lastline
    fs.pc += 1;
    pc
}

// Port of luaK_codevABCk - for vABCk format instructions (NEWTABLE, SETLIST)
// Used when C field is 10 bits (vC) instead of 8 bits
pub fn code_vabck(fs: &mut FuncState, op: OpCode, a: u32, b: u32, c: u32, k: bool) -> usize {
    let instr = Instruction::create_vabck(op, a, b, c, k);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.chunk.line_info.push(fs.lexer.lastline as u32);
    fs.pc += 1;
    pc
}

// Generate instruction with sJ field (for JMP)
pub fn code_asj(fs: &mut FuncState, op: OpCode, sj: i32) -> usize {
    let instr = Instruction::create_sj(op, sj);
    let pc = fs.pc;
    fs.chunk.code.push(instr);
    fs.chunk.line_info.push(fs.lexer.lastline as u32); // Use lastline
    fs.pc += 1;
    pc
}

use crate::compiler::expression::{ExpDesc, ExpKind};
use crate::{LuaValue, StringId};

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
    let needs_vararg_table = fs.chunk.needs_vararg_table;
    let num_params = fs.chunk.param_count;

    // lcode.c:1932-1933: If function uses vararg table, clear hidden vararg flag
    // In Lua 5.5: if (p->flag & PF_VATAB) p->flag &= ~PF_VAHID;
    if needs_vararg_table {
        fs.chunk.use_hidden_vararg = false;
    }
    let use_hidden_vararg = fs.chunk.use_hidden_vararg;

    for i in 0..fs.pc {
        let instr = &mut fs.chunk.code[i];
        let opcode = OpCode::from(Instruction::get_opcode(*instr));

        match opcode {
            OpCode::Return0 | OpCode::Return1 => {
                // lcode.c:1941-1944: Convert RETURN0/RETURN1 to RETURN if needed
                // Only convert when needclose OR use_hidden_vararg (PF_VAHID)
                if needclose || use_hidden_vararg {
                    // Convert to RETURN
                    let a = Instruction::get_a(*instr);
                    // For RETURN0, B=1 (0 returns + 1); for RETURN1, B=2 (1 return + 1)
                    let b = if opcode == OpCode::Return0 { 1 } else { 2 };
                    let mut new_instr = Instruction::create_abck(OpCode::Return, a, b, 0, false);

                    // lcode.c:1948-1950: Set k and C fields
                    if needclose {
                        Instruction::set_k(&mut new_instr, true);
                    }
                    if use_hidden_vararg {
                        Instruction::set_c(&mut new_instr, (num_params + 1) as u32);
                    }

                    *instr = new_instr;
                }
            }
            OpCode::Return | OpCode::TailCall => {
                // lcode.c:1948-1950: Set k and C fields for existing RETURN/TAILCALL
                if needclose {
                    Instruction::set_k(instr, true);
                }
                if use_hidden_vararg {
                    Instruction::set_c(instr, (num_params + 1) as u32);
                }
            }
            OpCode::GetVarg => {
                // lcode.c:1953-1956: GETVARG instruction handling
                // If function has a vararg table (PF_VATAB), convert to GETTABLE
                if needs_vararg_table {
                    let pc = &mut fs.chunk.code[i];
                    Instruction::set_opcode(pc, OpCode::GetTable);
                }
            }
            OpCode::Vararg => {
                // lcode.c:1958-1961: VARARG instruction k flag handling
                // If function has a vararg table (PF_VATAB), set k flag
                if needs_vararg_table {
                    let pc = &mut fs.chunk.code[i];
                    Instruction::set_k(pc, true);
                }
            }
            OpCode::Jmp => {
                // lcode.c:1963-1966: Fix jumps to final target
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
    while count < 100 {
        // Prevent infinite loops
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

// Port of luaK_jumpto macro from lcode.h:60
// #define luaK_jumpto(fs,t) luaK_patchlist(fs, luaK_jump(fs), t)
pub fn jumpto(fs: &mut FuncState, target: usize) {
    let jmp = jump(fs);
    patchlist(fs, jmp as isize, target as isize);
}

// Port of luaK_getlabel from lcode.c:234-237
// int luaK_getlabel (FuncState *fs)
// Port of lcode.c:233-236
pub fn get_label(fs: &mut FuncState) -> usize {
    fs.last_target = fs.pc;
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

// Port of luaK_getlabel from lcode.c:233-236
// int luaK_getlabel (FuncState *fs)
// Marks current position as a jump target and returns the current pc.
// This updates lasttarget which prevents instruction merging/optimization
// at jump targets (e.g., loop entry points should not merge with previous LOADNIL).
pub fn getlabel(fs: &mut FuncState) -> usize {
    fs.last_target = fs.pc;
    fs.pc
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
        return false; // Not a TESTSET instruction
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
            return e.u.info() as u8;
        }
        if e.u.info() >= fs.nactvar as i32 {
            exp2reg(fs, e, e.u.info() as u8);
            return e.u.info() as u8;
        }
    }
    exp2nextreg(fs, e)
}

// Port of exp2reg from lcode.c:915-938
// static void exp2reg (FuncState *fs, expdesc *e, int reg)
pub fn exp2reg(fs: &mut FuncState, e: &mut ExpDesc, reg: u8) {
    // lcode.c:918: must check VJMP BEFORE discharge2reg, as it changes the kind!
    let was_vjmp = e.kind == ExpKind::VJMP;
    let vjmp_info = if was_vjmp { e.u.info() as isize } else { -1 };

    discharge2reg(fs, e, reg);

    if was_vjmp {
        // expression itself is a test - put this jump in 't' list
        concat(fs, &mut e.t, vjmp_info);
    }
    if e.has_jumps() {
        // lcode.c:919-934: need to patch jump lists
        let mut p_f = -1_isize; // position of an eventual LOAD false
        let mut p_t = -1_isize; // position of an eventual LOAD true

        if need_value(fs, e.t) || need_value(fs, e.f) {
            // lcode.c:922-926
            // Note: must use was_vjmp, not e.kind, because discharge2reg already changed it!
            let fj = if was_vjmp { -1 } else { jump(fs) as isize };
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
    e.u = ExpUnion::Info(reg as i32);
    e.kind = ExpKind::VNONRELOC;
}

// Port of luaK_vapar2local from lcode.c:808-813
// Change a vararg parameter into a regular local variable
pub fn vapar_to_local(fs: &mut FuncState, var: &mut ExpDesc) {
    // lcode.c:809: needvatab(fs->f); function will need a vararg table
    fs.chunk.needs_vararg_table = true;
    // lcode.c:811: now a vararg parameter is equivalent to a regular local variable
    var.kind = ExpKind::VLOCAL;
}

// Port of luaK_dischargevars from lcode.c:766-817
// void luaK_dischargevars (FuncState *fs, expdesc *e)
pub fn discharge_vars(fs: &mut FuncState, e: &mut ExpDesc) {
    match e.kind {
        ExpKind::VCONST => {
            // Convert const variable to its actual value
            let vidx = e.u.info() as usize;
            if let Some(var_desc) = fs.actvar.get(vidx) {
                if let Some(value) = var_desc.const_value {
                    // Convert the const value to an expression
                    const_to_exp(value, e);
                }
            }
        }
        ExpKind::VVARGVAR => {
            // lcode.c:825-828: VVARGVAR converts to VLOCAL, then falls through
            vapar_to_local(fs, e);
            // FALLTHROUGH to VLOCAL case
            // After vapar_to_local, e.kind is VLOCAL, so process as VLOCAL
            e.u = ExpUnion::Info(e.u.var().ridx as i32);
            e.kind = ExpKind::VNONRELOC;
        }
        ExpKind::VLOCAL => {
            e.u = ExpUnion::Info(e.u.var().ridx as i32);
            e.kind = ExpKind::VNONRELOC;
        }
        ExpKind::VUPVAL => {
            e.u = ExpUnion::Info(code_abc(fs, OpCode::GetUpval, 0, e.u.info() as u32, 0) as i32);
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXUP => {
            e.u = ExpUnion::Info(code_abc(
                fs,
                OpCode::GetTabUp,
                0,
                e.u.ind().t as u32,
                e.u.ind().idx as u32,
            ) as i32);
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXI => {
            free_reg(fs, e.u.ind().t as u8);
            e.u = ExpUnion::Info(code_abc(
                fs,
                OpCode::GetI,
                0,
                e.u.ind().t as u32,
                e.u.ind().idx as u32,
            ) as i32);
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXSTR => {
            free_reg(fs, e.u.ind().t as u8);
            e.u = ExpUnion::Info(code_abc(
                fs,
                OpCode::GetField,
                0,
                e.u.ind().t as u32,
                e.u.ind().idx as u32,
            ) as i32);
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VINDEXED => {
            free_reg(fs, e.u.ind().idx as u8);
            free_reg(fs, e.u.ind().t as u8);
            e.u = ExpUnion::Info(code_abc(
                fs,
                OpCode::GetTable,
                0,
                e.u.ind().t as u32,
                e.u.ind().idx as u32,
            ) as i32);
            e.kind = ExpKind::VRELOC;
        }
        ExpKind::VVARGIND => {
            // Lua 5.5: indexed vararg parameter - generate GETVARG instruction
            free_reg(fs, e.u.ind().t as u8);
            free_reg(fs, e.u.ind().idx as u8);
            e.u = ExpUnion::Info(code_abc(
                fs,
                OpCode::GetVarg,
                0,
                e.u.ind().t as u32,
                e.u.ind().idx as u32,
            ) as i32);
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
            code_abx(fs, OpCode::LoadK, reg as u32, e.u.info() as u32);
        }
        ExpKind::VK => {
            // lcode.c:841: luaK_codek(fs, reg, e->u.info);
            code_abx(fs, OpCode::LoadK, reg as u32, e.u.info() as u32);
        }
        ExpKind::VKFLT => {
            // lcode.c:681-687: luaK_float(fs, reg, e->u.nval);
            // Try to use LOADF if float can be exactly represented as integer in sBx range
            let val = e.u.nval();

            // Check if float can be exactly converted to integer (no fractional part)
            if val.fract() == 0.0 && val.is_finite() {
                let int_val = val as i64;
                // Check if integer fits in sBx range (17-bit signed): -65535 to 65535
                // Same range as LOADI (uses fitsBx in official Lua)
                let max_sbx = (Instruction::MAX_BX - (Instruction::OFFSET_SBX as u32)) as i64;
                let min_sbx = -(Instruction::OFFSET_SBX as i64);
                if int_val >= min_sbx && int_val <= max_sbx {
                    // Use LOADF: encodes integer in sBx, loads as float at runtime
                    code_asbx(fs, OpCode::LoadF, reg as u32, int_val as i32);
                } else {
                    // Value too large, use LOADK
                    let k_idx = number_k(fs, val);
                    code_abx(fs, OpCode::LoadK, reg as u32, k_idx as u32);
                }
            } else {
                // Float has fractional part, use LOADK
                let k_idx = number_k(fs, val);
                code_abx(fs, OpCode::LoadK, reg as u32, k_idx as u32);
            }
        }
        ExpKind::VKINT => {
            // Check if value fits in sBx field (17-bit signed)
            // Port of luaK_int from lcode.c:673-678 and fitsBx from lcode.c:668-670
            let ival = e.u.ival();
            // sBx range check: -OFFSET_sBx to (MAXARG_Bx - OFFSET_sBx)
            // OFFSET_sBx = 65535, MAXARG_Bx = 131071, so range is -65535 to 65535
            let max_sbx = (Instruction::MAX_BX - (Instruction::OFFSET_SBX as u32)) as i64;
            let min_sbx = -(Instruction::OFFSET_SBX as i64);
            if ival >= min_sbx && ival <= max_sbx {
                code_asbx(fs, OpCode::LoadI, reg as u32, ival as i32);
            } else {
                // Value too large for LOADI, must use LOADK
                let k_idx = integer_k(fs, ival);
                code_abx(fs, OpCode::LoadK, reg as u32, k_idx as u32);
            }
        }
        ExpKind::VNONRELOC => {
            if e.u.info() != reg as i32 {
                code_abc(fs, OpCode::Move, reg as u32, e.u.info() as u32, 0);
            }
        }
        ExpKind::VVARGVAR => {
            // Lua 5.5: vararg parameter used as value (not indexed)
            // Move from vararg parameter register to target register
            let vreg = e.u.var().ridx;
            if vreg as u32 != reg as u32 {
                code_abc(fs, OpCode::Move, reg as u32, vreg as u32, 0);
            }
        }
        ExpKind::VRELOC => {
            let pc = e.u.info() as usize;
            Instruction::set_a(&mut fs.chunk.code[pc], reg as u32);
        }
        _ => {}
    }
    e.kind = ExpKind::VNONRELOC;
    e.u = ExpUnion::Info(reg as i32);
}

// Port of freeexp from lcode.c:558-561
// static void freeexp (FuncState *fs, expdesc *e)
pub fn free_exp(fs: &mut FuncState, e: &ExpDesc) {
    if e.kind == ExpKind::VNONRELOC {
        free_reg(fs, e.u.info() as u8);
    }
}

// Port of freeexps from lcode.c:567-572
// Free registers used by expressions 'e1' and 'e2' (if any) in proper order
fn free_exps(fs: &mut FuncState, e1: &ExpDesc, e2: &ExpDesc) {
    let r1 = if e1.kind == ExpKind::VNONRELOC {
        e1.u.info()
    } else {
        -1
    };
    let r2 = if e2.kind == ExpKind::VNONRELOC {
        e2.u.info()
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
// Port of freereg from lcode.c:492-498
// static void freereg (FuncState *fs, int reg)
pub fn free_reg(fs: &mut FuncState, reg: u8) {
    // lcode.c:493: if (reg >= luaY_nvarstack(fs))
    // Use nvarstack() which skips const variables, not nactvar
    let nvars = fs.nvarstack();
    if reg >= nvars && reg < fs.freereg {
        fs.freereg -= 1;
        // lcode.c:495: lua_assert(reg == fs->freereg);
        debug_assert_eq!(
            reg, fs.freereg,
            "freereg mismatch: expected reg {} to equal freereg {}",
            reg, fs.freereg
        );
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
pub fn nil(fs: &mut FuncState, from: u8, n: u8) {
    if n == 0 {
        return;
    }

    let pc = fs.pc;

    // Optimization: merge with previous LOADNIL if registers are contiguous
    // Port of lcode.c:136-151 optimization logic
    // Check if previous instruction exists and is not a jump target (lcode.c:117-123)
    if pc > 0 && pc > fs.last_target {
        let prev_pc = pc - 1;
        let prev_instr = fs.chunk.code[prev_pc];

        if Instruction::get_opcode(prev_instr) == OpCode::LoadNil {
            let pfrom = Instruction::get_a(prev_instr) as u8;
            let pl = pfrom + Instruction::get_b(prev_instr) as u8; // Last register in previous LOADNIL
            let l = from + n - 1; // Last register in new LOADNIL

            // Check if ranges can be connected (lcode.c:139-140)
            // Two cases:
            // 1. New range is to the right: pfrom <= from && from <= pl + 1
            // 2. New range is to the left: from <= pfrom && pfrom <= l + 1
            if (pfrom <= from && from <= pl + 1) || (from <= pfrom && pfrom <= l + 1) {
                // Merge: compute union of both ranges (lcode.c:141-143)
                let new_from = pfrom.min(from);
                let new_l = pl.max(l);
                let new_b = new_l - new_from;

                // Update previous instruction (lcode.c:144-145)
                Instruction::set_a(&mut fs.chunk.code[prev_pc], new_from as u32);
                Instruction::set_b(&mut fs.chunk.code[prev_pc], new_b as u32);
                return;
            }
        }
    }

    // Cannot merge, emit a new LOADNIL instruction (lcode.c:149)
    code_abc(fs, OpCode::LoadNil, from as u32, (n - 1) as u32, 0);
}

// Port of luaK_setoneret from lcode.c:755-766
// void luaK_setoneret (FuncState *fs, expdesc *e)
// Adjust a call/vararg expression to produce exactly one result.
// Calls are created returning one result (C=2), so they don't need fixing.
// For VCALL, just changes the expression type to VNONRELOC with fixed position.
// For VVARARG, sets C=2 (one result) and changes to VRELOC.
pub fn setoneret(fs: &mut FuncState, e: &mut ExpDesc) {
    if e.kind == ExpKind::VCALL {
        // lcode.c:758: already returns 1 value
        // lcode.c:759: lua_assert(GETARG_C(getinstruction(fs, e)) == 2);
        let pc = e.u.info() as usize;
        debug_assert_eq!(Instruction::get_c(fs.chunk.code[pc]), 2); // Should already be 2

        // lcode.c:760-761: e->k = VNONRELOC; e->u.info = GETARG_A(...)
        e.kind = ExpKind::VNONRELOC;
        e.u = ExpUnion::Info(Instruction::get_a(fs.chunk.code[pc]) as i32);
    } else if e.kind == ExpKind::VVARARG {
        // lcode.c:763: SETARG_C(getinstruction(fs, e), 2);
        let pc = e.u.info() as usize;
        Instruction::set_c(&mut fs.chunk.code[pc], 2);
        // lcode.c:764: e->k = VRELOC;
        e.kind = ExpKind::VRELOC;
    }
}

// Port of luaK_setreturns from lcode.c:722-732
// void luaK_setreturns (FuncState *fs, expdesc *e, int nresults)
pub fn setreturns(fs: &mut FuncState, e: &mut ExpDesc, nresults: u8) {
    let pc = e.u.info() as usize;
    if e.kind == ExpKind::VCALL {
        Instruction::set_c(&mut fs.chunk.code[pc], (nresults as u32) + 1);
    } else {
        // Must be VVARARG
        Instruction::set_c(&mut fs.chunk.code[pc], (nresults as u32) + 1);
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

const MAXINDEXRK: usize = 255; // Maximum index for R/K operands

// Add boolean true to constants
// Port of boolT from lcode.c:636-640
fn bool_t(fs: &mut FuncState) -> usize {
    // Use boolean value itself as both key and value (lcode.c:639)
    let val = LuaValue::boolean(true);
    add_constant(fs, val)
}

// Add boolean false to constants
// Port of boolF from lcode.c:626-630
fn bool_f(fs: &mut FuncState) -> usize {
    // Use boolean value itself as both key and value (lcode.c:629)
    let val = LuaValue::boolean(false);
    add_constant(fs, val)
}

// Add nil to constants
// Port of nilK from lcode.c:665-671
fn nil_k(fs: &mut FuncState) -> usize {
    // Cannot use nil as key; instead use table itself as key (lcode.c:669)
    // static int nilK (FuncState *fs) {
    //   TValue k, v;
    //   setnilvalue(&v);
    //   /* cannot use nil as key; instead use table itself */
    //   sethvalue(fs->ls->L, &k, fs->kcache);
    //   return k2proto(fs, &k, &v);
    // }
    let key = LuaValue::table(fs.kcache);
    let val = LuaValue::nil();
    add_constant_with_key(fs, key, val)
}

// Add integer to constants
fn int_k(fs: &mut FuncState, i: i64) -> usize {
    let val = LuaValue::integer(i);
    add_constant(fs, val)
}

// Add number to constants
// Port of luaK_numberK from lcode.c:619-639
fn number_k(fs: &mut FuncState, n: f64) -> usize {
    let val = LuaValue::float(n);
    
    // Special handling for float 0.0 to avoid collision with integer 0
    // Port of lcode.c:621-624:
    //   if (r == 0) {
    //     setpvalue(&kv, fs);  /* use FuncState as index */
    //     return k2proto(fs, &kv, &o);  /* cannot collide */
    //   }
    if n == 0.0 {
        // Use kcache table itself as key (like nilK does)
        // This prevents collision with integer 0 in the kcache table
        let key = LuaValue::table(fs.kcache);
        return add_constant_with_key(fs, key, val);
    }
    
    // Port of lcode.c:626-638: use perturbed key to avoid integer collision
    // const int nbm = l_floatatt(MANT_DIG);  // 53 for double
    // const lua_Number q = l_mathop(ldexp)(l_mathop(1.0), -nbm + 1);  // 2^-52
    // const lua_Number k =  r * (1 + q);  /* key */
    const MANT_DIG: i32 = 53; // mantissa digits for f64
    let q = 2.0_f64.powi(-MANT_DIG + 1); // 2^-52
    let k = n * (1.0 + q);  // perturbed key
    
    // Check if perturbed key can be converted to integer (lcode.c:630)
    // If yes, don't use kcache (would collide), just create new entry
    if k.floor() == k && k >= i64::MIN as f64 && k <= i64::MAX as f64 {
        // Key is still an integer, would collide - create new entry directly
        // Port of lcode.c:636: return addk(fs, fs->f, &o);
        let idx = fs.chunk.constants.len();
        fs.chunk.constants.push(val);
        return idx;
    }
    
    // Use perturbed key for kcache lookup (lcode.c:631-633)
    let key = LuaValue::float(k);
    let idx = add_constant_with_key(fs, key.clone(), val.clone());
    
    // Verify the stored value matches (lcode.c:632)
    if let Some(existing) = fs.chunk.constants.get(idx) {
        if val.raw_equal(existing, fs.pool) {
            return idx;
        }
    }
    
    // Collision detected - create new entry without caching (lcode.c:636)
    let idx = fs.chunk.constants.len();
    fs.chunk.constants.push(val);
    idx
}

// Add an integer constant to the constant table
// Port of luaK_int from lcode.c:717 (constant table part)
fn integer_k(fs: &mut FuncState, i: i64) -> usize {
    let val = LuaValue::integer(i);
    add_constant(fs, val)
}

// Port of stringK from lcode.c:576-580
// static int stringK (FuncState *fs, TString *s)
pub fn string_k(fs: &mut FuncState, s: String) -> usize {
    // Intern string to ObjectPool and get StringId
    let (string_id, _) = fs.pool.create_string(&s);

    // Add LuaValue with StringId to constants (check for duplicates)
    // For strings, key == value (strings are deduplicated globally)
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
// Port of str2K from lcode.c:774-779
// static int str2K (FuncState *fs, expdesc *e) {
//   lua_assert(e->k == VKSTR);
//   e->u.info = stringK(fs, e->u.strval);
//   e->k = VK;
//   return e->u.info;
// }
// In our Rust implementation, VKSTR already contains the constant index in u.info
// (set by string_k when creating the expression), so we just change the kind.
fn str2k(fs: &mut FuncState, e: &mut ExpDesc) -> usize {
    debug_assert!(e.kind == ExpKind::VKSTR);
    e.kind = ExpKind::VK;
    let str_id = e.u.str();
    let value = LuaValue::string(str_id);
    let k = add_constant(fs, value);
    e.u = ExpUnion::Info(k as i32);
    k
}

// Helper to add constant to chunk
// Port of addk from lcode.c:544-571
// key: The key used for lookup in scanner table (for deduplication)
// value: The actual value to store in the constant table
//
// For nil/false/true, key should be a special sentinel value to ensure global deduplication
// For strings, key == value (strings are deduplicated globally)
// For other types (numbers), key == value (numbers are deduplicated per-function)
// but we use rust's HashMap to handle that, so we just pass the same value for both key and value.
// Port of k2proto from lcode.c:565-575 (with separate key)
// static int k2proto (FuncState *fs, TValue *key, TValue *v)
fn add_constant_with_key(fs: &mut FuncState, key: LuaValue, value: LuaValue) -> usize {
    // Query kcache table with key (lcode.c:567)
    let found_idx: Option<usize> = {
        if let Some(kcache_table) = fs.pool.get_table(fs.kcache) {
            // luaH_get returns the value if found (stored as integer index)
            if let Some(idx_value) = kcache_table.raw_get(&key) {
                idx_value.as_integer().map(|i| i as usize)
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(idx) = found_idx {
        // Check if we can reuse this constant (lcode.c:568-572)
        if idx < fs.chunk.constants.len()
            && let Some(existing) = fs.chunk.constants.get(idx)
        {
            // Must match value (lcode.c:570-571)
            // Note: For floats, collisions can happen; for non-floats, must be equal
            if value.raw_equal(existing, fs.pool) {
                return idx;
            }
        }
    }

    // Constant not found or cannot be reused; create a new entry (lcode.c:573-577)
    let idx = fs.chunk.constants.len();
    fs.chunk.constants.push(value.clone());

    // Store key->index mapping in kcache table (lcode.c:575-576)
    if let Some(kcache_table) = fs.pool.get_table_mut(fs.kcache) {
        kcache_table.raw_set(key, LuaValue::integer(idx as i64));
    }

    idx
}

// Port of k2proto from lcode.c:565-575 (when key == value)
fn add_constant(fs: &mut FuncState, value: LuaValue) -> usize {
    add_constant_with_key(fs, value.clone(), value)
}

// Port of luaK_exp2K from lcode.c:1000-1026
// static int luaK_exp2K (FuncState *fs, expdesc *e)
fn exp2k(fs: &mut FuncState, e: &mut ExpDesc) -> bool {
    if !e.has_jumps() {
        // Handle VCONST: convert compile-time constant to actual constant expression
        if e.kind == ExpKind::VCONST {
            let vidx = e.u.info() as usize;
            if let Some(var) = fs.actvar.get(vidx) {
                if let Some(value) = var.const_value {
                    const_to_exp(value, e);
                    // Continue to process the converted expression
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }

        let info = match e.kind {
            ExpKind::VTRUE => bool_t(fs),
            ExpKind::VFALSE => bool_f(fs),
            ExpKind::VNIL => nil_k(fs),
            ExpKind::VKINT => int_k(fs, e.u.ival()),
            ExpKind::VKFLT => number_k(fs, e.u.nval()),
            ExpKind::VKSTR => str2k(fs, e),
            ExpKind::VK => e.u.info() as usize, // Already a constant, just use the info
            _ => return false,
        };

        if info <= MAXINDEXRK {
            e.kind = ExpKind::VK;
            e.u = ExpUnion::Info(info as i32);
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
    let pc = get_jump_control(fs, e.u.info() as usize);
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
        fs.chunk.line_info.pop(); // Must also remove line info!
    }
}

const NO_REG: u32 = 255;

// Port of jumponcond from lcode.c:1118-1130
// static int jumponcond (FuncState *fs, expdesc *e, int cond)
fn jumponcond(fs: &mut FuncState, e: &mut ExpDesc, cond: bool) -> isize {
    if e.kind == ExpKind::VRELOC {
        let ie = fs.chunk.code[e.u.info() as usize];
        if OpCode::from(Instruction::get_opcode(ie)) == OpCode::Not {
            remove_last_instruction(fs);
            let b = Instruction::get_b(ie);
            return condjump(fs, OpCode::Test, b, 0, 0, !cond);
        }
    }
    discharge2anyreg(fs, e);
    free_exp(fs, e);
    condjump(fs, OpCode::TestSet, NO_REG, e.u.info() as u32, 0, cond)
}

// Port of luaK_goiftrue from lcode.c:1135-1160
// void luaK_goiftrue (FuncState *fs, expdesc *e)
pub fn goiftrue(fs: &mut FuncState, e: &mut ExpDesc) {
    discharge_vars(fs, e);
    let pc = match e.kind {
        ExpKind::VJMP => {
            negatecondition(fs, e);
            e.u.info() as isize
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
        ExpKind::VJMP => e.u.info() as isize,
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
            // Port of lcode.c:1670-1676: Use isSCnumber, not tonumeral
            let mut im: i32 = 0;
            let mut isfloat: bool = false;
            if !is_scnumber(v, &mut im, &mut isfloat) {
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
            if e1.u.info() as u32 + 1 == a {
                free_exp(fs, e2);
                Instruction::set_a(&mut fs.chunk.code[prev_pc], e1.u.info() as u32);
                Instruction::set_b(&mut fs.chunk.code[prev_pc], n + 1);
                return;
            }
        }
    }
    // New concat opcode
    code_abc(fs, OpCode::Concat, e1.u.info() as u32, 2, 0);
    free_exp(fs, e2);
}

// Port of codeABRK from lcode.c:1040-1044
// Generate instruction with A, B, and RK operand (Register or K constant)
// This allows SETFIELD/SETTABLE to use constant values directly
pub fn code_abrk(fs: &mut FuncState, opcode: OpCode, a: u32, b: u32, ec: &mut ExpDesc) {
    let k = exp2rk(fs, ec);
    let c = ec.u.info() as u32;
    code_abck(fs, opcode, a, b, c, k);
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
        r2 = e2.u.info() as u32; // constant index
    }
    // Regular case: compare two registers
    else {
        opcode = OpCode::Eq;
        r2 = exp2anyreg(fs, e2) as u32;
    }

    free_exps(fs, e1, e2);

    let k = op == BinaryOperator::OpEq;
    let pc = condjump(fs, opcode, r1, r2, isfloat as u32, k);

    e1.u = ExpUnion::Info(pc as i32);
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

    e1.u = ExpUnion::Info(pc as i32);
    e1.kind = ExpKind::VJMP;
}

// Check if operator is foldable (arithmetic or bitwise)
// Port of foldbinop macro from lcode.h:45
fn foldbinop(op: BinaryOperator) -> bool {
    use BinaryOperator::*;
    matches!(
        op,
        OpAdd
            | OpSub
            | OpMul
            | OpDiv
            | OpIDiv
            | OpMod
            | OpPow
            | OpBAnd
            | OpBOr
            | OpBXor
            | OpShl
            | OpShr
    )
}

// Check if folding operation is valid and won't raise errors
// Port of validop from lcode.c:1316-1330
// Note: Official takes TValue pointers, but we already have extracted values
fn validop(op: BinaryOperator, e1: &ExpDesc, e2: &ExpDesc) -> bool {
    use BinaryOperator::*;
    use ExpKind::{VKFLT, VKINT};

    match op {
        // Bitwise operations need integer-convertible operands
        // Official: luaV_tointegerns checks if values can convert to integer
        OpBAnd | OpBOr | OpBXor | OpShl | OpShr => {
            // Both operands must be integers or floats that can convert to integers
            let ok1 = match e1.kind {
                VKINT => true,
                VKFLT => {
                    let v = e1.u.nval();
                    v.is_finite()
                        && v.fract() == 0.0
                        && v >= i64::MIN as f64
                        && v <= i64::MAX as f64
                }
                _ => false,
            };
            let ok2 = match e2.kind {
                VKINT => true,
                VKFLT => {
                    let v = e2.u.nval();
                    v.is_finite()
                        && v.fract() == 0.0
                        && v >= i64::MIN as f64
                        && v <= i64::MAX as f64
                }
                _ => false,
            };
            ok1 && ok2
        }
        // Division operations cannot have 0 divisor
        // Official: nvalue(v2) != 0
        OpDiv | OpIDiv | OpMod => match e2.kind {
            VKINT => e2.u.ival() != 0,
            VKFLT => e2.u.nval() != 0.0,
            _ => false,
        },
        _ => true, // everything else is valid
    }
}

// Try to constant-fold a binary operation
// Port of constfolding from lcode.c:1337-1356
// Mimics luaO_rawarith: if both operands are INTEGER type, result is INTEGER; otherwise FLOAT
fn constfolding(_fs: &FuncState, op: BinaryOperator, e1: &mut ExpDesc, e2: &ExpDesc) -> bool {
    use BinaryOperator::*;
    use ExpKind::{VKFLT, VKINT};

    // Check original types (not whether values can be represented as integers)
    let e1_is_int_type = matches!(e1.kind, VKINT);
    let e2_is_int_type = matches!(e2.kind, VKINT);

    // Check if both operands are numeric constants and extract values
    let (v1, i1) = match e1.kind {
        VKINT => {
            let iv = e1.u.ival();
            (iv as f64, iv)
        }
        VKFLT => {
            let f = e1.u.nval();
            let as_int = f as i64;
            (f, as_int)
        }
        _ => return false,
    };

    let (v2, i2) = match e2.kind {
        VKINT => {
            let iv = e2.u.ival();
            (iv as f64, iv)
        }
        VKFLT => {
            let f = e2.u.nval();
            let as_int = f as i64;
            (f, as_int)
        }
        _ => return false,
    };

    // Check if operation is valid (no division by zero, etc.)
    if !validop(op, e1, e2) {
        return false;
    }

    // Bitwise operations: require both operands to be representable as integers
    // IMPORTANT: Lua treats integers as UNSIGNED for bitwise operations (see lvm.h intop macro)
    // intop(op,v1,v2) = l_castU2S(l_castS2U(v1) op l_castS2U(v2))
    match op {
        OpBAnd | OpBOr | OpBXor => {
            // Check if float values can be exactly converted to integers
            let v1_can_be_int = if e1_is_int_type {
                true
            } else {
                (i1 as f64) == v1
            };
            let v2_can_be_int = if e2_is_int_type {
                true
            } else {
                (i2 as f64) == v2
            };

            if !v1_can_be_int || !v2_can_be_int {
                return false;
            }

            e1.kind = VKINT;
            // Cast to unsigned, perform operation, cast back to signed
            let u1 = i1 as u64;
            let u2 = i2 as u64;
            e1.u = ExpUnion::IVal(match op {
                OpBAnd => (u1 & u2) as i64,
                OpBOr => (u1 | u2) as i64,
                OpBXor => (u1 ^ u2) as i64,
                _ => unreachable!(),
            });
            return true;
        }
        OpShl | OpShr => {
            let v1_can_be_int = if e1_is_int_type {
                true
            } else {
                (i1 as f64) == v1
            };
            let v2_can_be_int = if e2_is_int_type {
                true
            } else {
                (i2 as f64) == v2
            };

            if !v1_can_be_int || !v2_can_be_int {
                return false;
            }

            e1.kind = VKINT;
            // Port of luaV_shiftl from lvm.c:780-793
            // Lua uses unsigned shift (logical shift, not arithmetic)
            let u1 = i1 as u64;
            e1.u = ExpUnion::IVal(match op {
                OpShl => {
                    if i2 < 0 {
                        // Shift right with negative y
                        let shift_amount = -i2;
                        if shift_amount >= 64 {
                            0
                        } else {
                            (u1 >> shift_amount) as i64
                        }
                    } else {
                        // Shift left with positive y
                        if i2 >= 64 { 0 } else { (u1 << i2) as i64 }
                    }
                }
                OpShr => {
                    // luaV_shiftr(x,y) = luaV_shiftl(x, -y)
                    let neg_i2 = -i2;
                    if neg_i2 < 0 {
                        // Shift right
                        let shift_amount = -neg_i2;
                        if shift_amount >= 64 {
                            0
                        } else {
                            (u1 >> shift_amount) as i64
                        }
                    } else {
                        // Shift left
                        if neg_i2 >= 64 {
                            0
                        } else {
                            (u1 << neg_i2) as i64
                        }
                    }
                }
                _ => unreachable!(),
            });
            return true;
        }
        _ => {}
    }

    // Arithmetic operations follow luaO_rawarith logic:
    // If both operands have INTEGER type, try integer arithmetic; otherwise use float
    match op {
        OpDiv | OpPow => {
            // These operations always produce float results
            let result = match op {
                OpDiv => v1 / v2,
                OpPow => v1.powf(v2),
                _ => unreachable!(),
            };

            // Port of lcode.c:1348-1351: folds neither NaN nor 0.0 (to avoid problems with -0.0)
            if result.is_nan() || result == 0.0 {
                return false;
            }

            e1.kind = VKFLT;
            e1.u = ExpUnion::NVal(result);
            return true;
        }
        OpAdd | OpSub | OpMul | OpIDiv | OpMod => {
            // If both operands are INTEGER type, try integer operation
            if e1_is_int_type && e2_is_int_type {
                let int_result = match op {
                    OpAdd => i1.checked_add(i2),
                    OpSub => i1.checked_sub(i2),
                    OpMul => i1.checked_mul(i2),
                    OpIDiv => i1.checked_div(i2),
                    OpMod => {
                        // Use Lua's modulo definition: a % b = a - floor(a/b) * b
                        // NOT Rust's rem_euclid which differs for negative divisors
                        let quot = (i1 as f64) / (i2 as f64);
                        let floor_quot = quot.floor() as i64;
                        match floor_quot.checked_mul(i2) {
                            Some(prod) => i1.checked_sub(prod),
                            None => None,
                        }
                    }
                    _ => unreachable!(),
                };

                if let Some(res) = int_result {
                    e1.kind = VKINT;
                    e1.u = ExpUnion::IVal(res);
                    return true;
                }
                // Integer operation failed (overflow), fall through to float
            }

            // At least one operand is FLOAT type, or integer operation overflowed
            let result = match op {
                OpAdd => v1 + v2,
                OpSub => v1 - v2,
                OpMul => v1 * v2,
                OpIDiv => (v1 / v2).floor(),
                OpMod => v1 - (v1 / v2).floor() * v2,
                _ => unreachable!(),
            };

            // Port of lcode.c:1348-1351: folds neither NaN nor 0.0 (to avoid problems with -0.0)
            if result.is_nan() || result == 0.0 {
                return false;
            }

            e1.kind = VKFLT;
            e1.u = ExpUnion::NVal(result);
            return true;
        }
        _ => return false,
    }
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
            // Port of codecommutative logic from lcode.c:1517-1527
            // For commutative operators, if first operand is numeric constant (tonumeral check),
            // swap them to enable immediate/K optimizations on the second operand
            let mut flip = false;
            if matches!(op, BinaryOperator::OpAdd | BinaryOperator::OpMul) {
                // Check if e1 is a numeric constant (tonumeral check in lcode.c:1520)
                // tonumeral only accepts VKINT and VKFLT, NOT VK (which might be string)
                if matches!(e1.kind, ExpKind::VKINT | ExpKind::VKFLT) {
                    // Swap operands to put constant on right side
                    swapexps(e1, e2);
                    flip = true;
                }
            } else if matches!(
                op,
                BinaryOperator::OpBAnd | BinaryOperator::OpBOr | BinaryOperator::OpBXor
            ) {
                // For bitwise operations, use codebitwise logic (lcode.c:1533-1549)
                // Only check for VKINT (not VKFLT), swap to put it on right
                if matches!(e1.kind, ExpKind::VKINT) {
                    swapexps(e1, e2);
                    flip = true;
                }
            }

            // Port of codebini from lcode.c:1572-1598 and finishbinexpneg:1464-1480
            // For ADD: Check if e2 is a small integer constant (isSCint check) - lcode.c:1523
            // For SUB: Check if can negate second operand (finishbinexpneg) - lcode.c:1733-1737
            if op == BinaryOperator::OpAdd {
                if let ExpKind::VKINT = e2.kind {
                    let imm_val = e2.u.ival();
                    // For ADD: check if value fits in sC field (int2sC(i) = i + 127 must be in 0-255)
                    // So i must be in range -127 to 128
                    if imm_val >= -127 && imm_val <= 128 {
                        // Use ADDI instruction
                        let r1 = exp2anyreg(fs, e1);
                        let enc_imm = ((imm_val + 127) & 0xff) as u32;
                        let pc = code_abc(fs, OpCode::AddI, 0, r1 as u32, enc_imm);

                        free_exp(fs, e1);
                        free_exp(fs, e2);

                        e1.kind = ExpKind::VRELOC;
                        e1.u = ExpUnion::Info(pc as i32);

                        // Generate MMBINI for metamethod fallback
                        let mm_imm = ((imm_val + 127) & 0xff) as u32;
                        code_abck(
                            fs,
                            OpCode::MmBinI,
                            r1 as u32,
                            mm_imm,
                            TmKind::Add as u32,
                            flip,
                        );
                        return;
                    }
                }
            } else if op == BinaryOperator::OpSub {
                // SUB can be converted to ADDI with negated immediate (finishbinexpneg)
                // But BOTH original and negated values must fit in range!
                if let ExpKind::VKINT = e2.kind {
                    let imm_val = e2.u.ival();
                    let neg_imm = -imm_val;
                    // Both values must fit in -127..128 range (sC field capacity)
                    if imm_val >= -127 && imm_val <= 128 && neg_imm >= -127 && neg_imm <= 128 {
                        // Use ADDI with negated immediate
                        let r1 = exp2anyreg(fs, e1);
                        let enc_imm = ((neg_imm + 127) & 0xff) as u32; // Encode negated value for ADDI
                        let pc = code_abc(fs, OpCode::AddI, 0, r1 as u32, enc_imm);

                        free_exp(fs, e1);
                        free_exp(fs, e2);

                        e1.kind = ExpKind::VRELOC;
                        e1.u = ExpUnion::Info(pc as i32);

                        // Generate MMBINI with ORIGINAL value for metamethod
                        // (finishbinexpneg corrects the metamethod argument - lcode.c:1476)
                        let mm_imm = ((imm_val + 127) & 0xff) as u32;
                        code_abck(
                            fs,
                            OpCode::MmBinI,
                            r1 as u32,
                            mm_imm,
                            TmKind::Sub as u32,
                            flip,
                        );
                        return;
                    }
                }
            }

            // Handle shift operations (lcode.c:1745-1760)
            if op == BinaryOperator::OpShl {
                // OPR_SHL has three cases:
                // 1. e1 is small int -> SHLI (immediate << register)
                // 2. e2 can be negated -> SHRI (a << b = a >> -b)
                // 3. else -> regular SHL
                if let ExpKind::VKINT = e1.kind {
                    let imm_val = e1.u.ival();
                    if imm_val >= -127 && imm_val <= 128 {
                        // Case 1: SHLI (immediate << register)
                        swapexps(e1, e2); // Put immediate on right for processing
                        let r1 = exp2anyreg(fs, e1);
                        let enc_imm = ((imm_val + 127) & 0xff) as u32;
                        let pc = code_abc(fs, OpCode::ShlI, 0, r1 as u32, enc_imm);

                        free_exp(fs, e1);
                        free_exp(fs, e2);

                        e1.kind = ExpKind::VRELOC;
                        e1.u = ExpUnion::Info(pc as i32);

                        // MMBINI with flip=1 since immediate was on left
                        let mm_imm = ((imm_val + 127) & 0xff) as u32;
                        code_abck(
                            fs,
                            OpCode::MmBinI,
                            r1 as u32,
                            mm_imm,
                            TmKind::Shl as u32,
                            true,
                        );
                        return;
                    }
                } else if let ExpKind::VKINT = e2.kind {
                    // Case 2: Check if e2 can be negated -> use SHRI
                    let imm_val = e2.u.ival();
                    let neg_imm = -imm_val;
                    if imm_val >= -127 && imm_val <= 128 && neg_imm >= -127 && neg_imm <= 128 {
                        // Use SHRI with negated immediate (a << b = a >> -b)
                        let r1 = exp2anyreg(fs, e1);
                        let enc_imm = ((neg_imm + 127) & 0xff) as u32;
                        let pc = code_abc(fs, OpCode::ShrI, 0, r1 as u32, enc_imm);

                        free_exp(fs, e1);
                        free_exp(fs, e2);

                        e1.kind = ExpKind::VRELOC;
                        e1.u = ExpUnion::Info(pc as i32);

                        // MMBINI with ORIGINAL value for TM_SHL
                        let mm_imm = ((imm_val + 127) & 0xff) as u32;
                        code_abck(
                            fs,
                            OpCode::MmBinI,
                            r1 as u32,
                            mm_imm,
                            TmKind::Shl as u32,
                            flip,
                        );
                        return;
                    }
                }
                // Case 3: Fall through to regular codebinexpval
            } else if op == BinaryOperator::OpShr {
                // OPR_SHR: if e2 is small int -> SHRI, else regular SHR
                if let ExpKind::VKINT = e2.kind {
                    let imm_val = e2.u.ival();
                    if imm_val >= -127 && imm_val <= 128 {
                        let r1 = exp2anyreg(fs, e1);
                        let enc_imm = ((imm_val + 127) & 0xff) as u32;
                        let pc = code_abc(fs, OpCode::ShrI, 0, r1 as u32, enc_imm);

                        free_exp(fs, e1);
                        free_exp(fs, e2);

                        e1.kind = ExpKind::VRELOC;
                        e1.u = ExpUnion::Info(pc as i32);

                        let mm_imm = ((imm_val + 127) & 0xff) as u32;
                        code_abck(
                            fs,
                            OpCode::MmBinI,
                            r1 as u32,
                            mm_imm,
                            TmKind::Shr as u32,
                            flip,
                        );
                        return;
                    }
                }
                // Fall through to regular codebinexpval
            }

            // Try to use K operand optimization
            // For bitwise operations, only use K instructions if e2 is VKINT (lcode.c:1540)
            // For arithmetic operations, use K if tonumeral(e2) succeeds (lcode.c:1505)
            let use_k_instruction = match op {
                // Bitwise operations: only VKINT can use K instructions
                BinaryOperator::OpBAnd | BinaryOperator::OpBOr | BinaryOperator::OpBXor => {
                    matches!(e2.kind, ExpKind::VKINT) && exp2k(fs, e2)
                }
                // Arithmetic operations: VKINT or VKFLT can use K instructions
                BinaryOperator::OpAdd
                | BinaryOperator::OpSub
                | BinaryOperator::OpMul
                | BinaryOperator::OpDiv
                | BinaryOperator::OpIDiv
                | BinaryOperator::OpMod
                | BinaryOperator::OpPow => {
                    matches!(e2.kind, ExpKind::VKINT | ExpKind::VKFLT) && exp2k(fs, e2)
                }
                _ => false,
            };

            if use_k_instruction {
                // e2 is a valid K operand, generate K-series instruction
                let k_idx = e2.u.info();
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
                        e1.u = ExpUnion::Info(res as i32);
                        return;
                    }
                };

                // Port of finishbinexpval from lcode.c:1407-1418
                // Generate K-series instruction with A=0, will be fixed by discharge2reg
                let pc = code_abc(fs, opcode, 0, r1 as u32, k_idx as u32);

                // Free both operands (freeexps) - must use free_exps to maintain proper order
                free_exps(fs, e1, e2);

                // Mark as relocatable - target register will be decided later
                e1.kind = ExpKind::VRELOC;
                e1.u = ExpUnion::Info(pc as i32);

                // Generate metamethod fallback instruction (MMBINK)
                // Like finishbinexpval in lcode.c:1416
                // TM events from ltm.h (TM_ADD=6, TM_SUB=7, etc.)
                let tm_event = match op {
                    BinaryOperator::OpAdd => TmKind::Add,   // TM_ADD
                    BinaryOperator::OpSub => TmKind::Sub,   // TM_SUB
                    BinaryOperator::OpMul => TmKind::Mul,   // TM_MUL
                    BinaryOperator::OpMod => TmKind::Mod,   // TM_MOD
                    BinaryOperator::OpPow => TmKind::Pow,   // TM_POW
                    BinaryOperator::OpDiv => TmKind::Div,   // TM_DIV
                    BinaryOperator::OpIDiv => TmKind::IDiv, // TM_IDIV
                    BinaryOperator::OpBAnd => TmKind::Band, // TM_BAND
                    BinaryOperator::OpBOr => TmKind::Bor,   // TM_BOR
                    BinaryOperator::OpBXor => TmKind::Bxor, // TM_BXOR
                    _ => TmKind::N,                         // Invalid for other ops
                };
                // Use code_abck to include flip bit (k-flag) like in MMBINI
                code_abck(
                    fs,
                    OpCode::MmBinK,
                    r1 as u32,
                    k_idx as u32,
                    tm_event as u32,
                    flip,
                );
            } else {
                // Both operands in registers - port of codebinNoK (lcode.c:1490-1496)
                // If we flipped operands for K optimization attempt, swap back to original order
                if flip {
                    swapexps(e1, e2);
                }

                // Now port of codebinexpval (lcode.c:1425-1434)
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

                // Free both operands (freeexps) - must use free_exps to maintain proper order
                free_exps(fs, e1, e2);

                // Mark as relocatable
                e1.kind = ExpKind::VRELOC;
                e1.u = ExpUnion::Info(pc as i32);

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
// Port of removevalues from lcode.c:280-283
// static void removevalues (FuncState *fs, int list)
fn removevalues(fs: &mut FuncState, mut list: isize) {
    const NO_JUMP: isize = -1;
    while list != NO_JUMP {
        patchtestreg(fs, list as usize, NO_REG as u8);
        list = get_jump(fs, list as usize);
    }
}

// Port of codenot from lcode.c:1231-1261
fn codenot(fs: &mut FuncState, e: &mut ExpDesc) {
    discharge_vars(fs, e);
    match e.kind {
        ExpKind::VNIL | ExpKind::VFALSE => {
            // true == not nil == not false (lcode.c:1234)
            e.kind = ExpKind::VTRUE;
        }
        ExpKind::VK | ExpKind::VKFLT | ExpKind::VKINT | ExpKind::VKSTR | ExpKind::VTRUE => {
            // false == not "x" == not 0.5 == not 1 == not true (lcode.c:1237)
            e.kind = ExpKind::VFALSE;
        }
        ExpKind::VJMP => {
            // Negate the condition (lcode.c:1241)
            negatecondition(fs, e);
        }
        ExpKind::VRELOC | ExpKind::VNONRELOC => {
            // Generate NOT instruction (lcode.c:1244-1251)
            discharge2anyreg(fs, e);
            free_exp(fs, e);
            let pc = code_abc(fs, OpCode::Not, 0, e.u.info() as u32, 0);
            e.u = ExpUnion::Info(pc as i32);
            e.kind = ExpKind::VRELOC;
        }
        _ => {} // Should not happen
    }
    // Interchange true and false lists (lcode.c:1256)
    std::mem::swap(&mut e.t, &mut e.f);
    // Values are useless when negated (lcode.c:1257-1258)
    removevalues(fs, e.f);
    removevalues(fs, e.t);
}

// Simplified implementation of luaK_prefix - generate unary operation
// Port of codeunexpval from lcode.c:1392-1398
// Generate code for unary operation that produces a value
fn codeunexpval(fs: &mut FuncState, op: OpCode, e: &mut ExpDesc) {
    let r = exp2anyreg(fs, e); // opcodes operate only on registers
    free_exp(fs, e);
    let pc = code_abc(fs, op, 0, r as u32, 0); // result register is 0 (will be relocated)
    e.u = ExpUnion::Info(pc as i32);
    e.kind = ExpKind::VRELOC; // all those operations are relocatable
}

pub fn prefix(fs: &mut FuncState, op: OpCode, e: &mut ExpDesc) {
    discharge_vars(fs, e);

    // Port of luaK_prefix from lcode.c:1616-1631
    match op {
        OpCode::Unm => {
            // Try constant folding for unary minus (lcode.c:1620-1622)
            match e.kind {
                ExpKind::VKINT => {
                    // Negate integer constant in place
                    let val = e.u.ival();
                    e.u = ExpUnion::IVal(val.wrapping_neg());
                    return;
                }
                ExpKind::VKFLT => {
                    // Negate float constant in place
                    let val = e.u.nval();
                    let negated = -val;
                    // Check if result is -0.0 (bit pattern 0x8000000000000000)
                    // Official Lua doesn't fold -0.0 to constant because it needs special handling
                    if negated.to_bits() == 0x8000000000000000 {
                        // Don't fold, emit UNM instruction instead
                        codeunexpval(fs, op, e);
                        return;
                    }
                    e.u = ExpUnion::NVal(negated);
                    return;
                }
                _ => {}
            }
            // Otherwise fall through to codeunexpval
            codeunexpval(fs, op, e);
        }
        OpCode::BNot => {
            // Try constant folding for bitwise not (lcode.c:1620-1622)
            if e.kind == ExpKind::VKINT {
                let val = e.u.ival();
                e.u = ExpUnion::IVal(!val);
                return;
            }
            // Otherwise fall through to codeunexpval
            codeunexpval(fs, op, e);
        }
        OpCode::Len => {
            // LEN operation (lcode.c:1625)
            codeunexpval(fs, op, e);
        }
        OpCode::Not => {
            // NOT operation (lcode.c:1627)
            codenot(fs, e);
        }
        _ => unreachable!(),
    }
}

// Port of luaK_exp2anyregup from lcode.c:978-981
// void luaK_exp2anyregup (FuncState *fs, expdesc *e)
pub fn exp2anyregup(fs: &mut FuncState, e: &mut ExpDesc) {
    // lcode.c:1034-1035: Skip exp2anyreg for VUPVAL and VVARGVAR (unless has jumps)
    // if ((e->k != VUPVAL && e->k != VVARGVAR) || hasjumps(e))
    //     luaK_exp2anyreg(fs, e);
    if (e.kind != ExpKind::VUPVAL && e.kind != ExpKind::VVARGVAR) || e.has_jumps() {
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
    // lcode.c:1358-1360: int keystr = -1; if (k->k == VKSTR) keystr = str2K(fs, k);
    let mut keystr: i32 = -1;
    if k.kind == ExpKind::VKSTR {
        // str2K - convert to constant
        str2k(fs, k);
        keystr = k.u.info();
    }

    // Table must be in local/nonreloc/upval/vvargvar
    if t.kind == ExpKind::VUPVAL && !is_kstr(fs, k) {
        exp2anyreg(fs, t);
    }

    if t.kind == ExpKind::VUPVAL {
        let temp = t.u.info();
        t.u = ExpUnion::Ind(IndVars {
            t: temp as i16,
            idx: k.u.info() as i16,
            ro: false,
            keystr: keystr, // lcode.c:1394: t->u.ind.keystr = keystr;
        });
        t.kind = ExpKind::VINDEXUP;
    } else if t.kind == ExpKind::VVARGVAR {
        // Lua 5.5: indexing the vararg parameter
        let kreg = exp2anyreg(fs, k); // put key in some register
        let vreg = t.u.var().ridx; // register with vararg param
        // lua_assert(vreg == fs->f->numparams);
        t.u = ExpUnion::Ind(IndVars {
            t: vreg as i16,
            idx: kreg as i16,
            ro: false,
            keystr: keystr,
        });
        t.kind = ExpKind::VVARGIND; // t represents vararg[k]
    } else {
        // Register index of the table
        t.u.ind_mut().t = if t.kind == ExpKind::VLOCAL {
            t.u.var().ridx as i16
        } else {
            t.u.info() as i16
        };

        if is_kstr(fs, k) {
            t.u.ind_mut().idx = k.u.info() as i16;
            t.kind = ExpKind::VINDEXSTR;
        } else if is_cint(k) {
            t.u.ind_mut().idx = k.u.ival() as i16;
            t.kind = ExpKind::VINDEXI;
        } else {
            t.u.ind_mut().idx = exp2anyreg(fs, k) as i16;
            t.kind = ExpKind::VINDEXED;
        }
        // lcode.c:1394: t->u.ind.keystr = keystr;
        t.u.ind_mut().keystr = keystr;
    }
}

#[inline]
fn hasjumps(e: &ExpDesc) -> bool {
    e.t != e.f
}

// Check if expression is a constant string
fn is_kstr(fs: &FuncState, e: &ExpDesc) -> bool {
    let ok1 = e.kind == ExpKind::VK && !hasjumps(e) && e.u.info() <= Instruction::MAX_B as i32;
    if !ok1 {
        return false;
    }

    let k_idx = e.u.info() as usize;
    if k_idx >= fs.chunk.constants.len() {
        return false;
    }

    let const_val = &fs.chunk.constants[k_idx];
    if let Some(string_id) = const_val.as_string_id() {
        return fs.pool.is_short_string(string_id);
    }

    false
}

fn is_kint(e: &ExpDesc) -> bool {
    e.kind == ExpKind::VKINT && !hasjumps(e)
}

// Check if expression is a constant integer in valid range for SETI
// SETI's B field is only 8 bits (0-255), matching Lua C's isCint check
fn is_cint(e: &ExpDesc) -> bool {
    is_kint(e) && e.u.ival() as u64 <= Instruction::MAX_C as u64
}

// Port of luaK_self from lcode.c:1323-1343
// void luaK_self (FuncState *fs, expdesc *e, expdesc *key)
pub fn self_op(fs: &mut FuncState, e: &mut ExpDesc, key: &mut ExpDesc) {
    // Port of luaK_self from lcode.c:1323-1343
    let ereg = exp2anyreg(fs, e);
    free_exp(fs, e);

    let base = fs.freereg;
    e.u = ExpUnion::Info(base as i32);
    e.kind = ExpKind::VNONRELOC;
    reserve_regs(fs, 2); // function and 'self'

    // SELF A B C: R(A+1) := R(B); R(A) := R(B)[RK(C)]
    // Follow Lua 5.5: key should be VKSTR at this point (lcode.c:1332: lua_assert(key->k == VKSTR))
    // Only emit OP_SELF when method name is a SHORT string constant.
    // Lua 5.5 has LUAI_MAXSHORTLEN=40, strings longer than that cannot use SELF optimization
    // (see lcode.c:1333: strisshr check).
    const LUAI_MAXSHORTLEN: usize = 40; // Lua 5.5 short string length limit
    
    // Check if key is a VKSTR (method name string)
    let can_use_self = if key.kind == ExpKind::VKSTR {
        // Check if it's a short string
        let str_id = key.u.str();
        if let Some(lua_str) = fs.pool.get_string(str_id) {
            lua_str.as_str().len() <= LUAI_MAXSHORTLEN
        } else {
            false
        }
    } else {
        false // Not VKSTR
    };
    
    if can_use_self {
        // Method name is a short string - try to convert to VK via exp2K
        // This calls str2k which adds the string to constant table (lcode.c:1333: luaK_exp2K)
        if exp2k(fs, key) {
            // Successfully converted to VK and fits in MAXINDEXRK
            // Emit OP_SELF with k = 0 (Lua 5.5 never sets k flag for SELF)
            code_abck(
                fs,
                OpCode::Self_,
                base as u32,
                ereg as u32,
                key.u.info() as u32,
                false,
            );
        } else {
            // Constant index too large, fallback to MOVE + GETTABLE
            exp2anyreg(fs, key);
            code_abc(fs, OpCode::Move, (base + 1) as u32, ereg as u32, 0);
            let kc = key.u.info() as u32;
            code_abc(fs, OpCode::GetTable, base as u32, ereg as u32, kc);
        }
    } else {
        // Not a short string or not VKSTR - fallback: put method name in a register and emit MOVE + GETTABLE
        exp2anyreg(fs, key);
        code_abc(fs, OpCode::Move, (base + 1) as u32, ereg as u32, 0);
        let kc = key.u.info() as u32;
        code_abc(fs, OpCode::GetTable, base as u32, ereg as u32, kc);
    }
    free_exp(fs, key);
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
    if fs.chunk.line_info.len() > 0 {
        let last_idx = fs.chunk.line_info.len() - 1;
        fs.chunk.line_info[last_idx] = line as u32;
    }
}

// Port of luaK_exp2const from lcode.c:85-108
// int luaK_exp2const (FuncState *fs, const expdesc *e, TValue *v)
pub fn exp2const(fs: &FuncState, e: &ExpDesc) -> Option<LuaValue> {
    if e.has_jumps() {
        return None;
    }

    let result = match e.kind {
        ExpKind::VFALSE => Some(LuaValue::boolean(false)),
        ExpKind::VTRUE => Some(LuaValue::boolean(true)),
        ExpKind::VNIL => Some(LuaValue::nil()),
        ExpKind::VKSTR => {
            // String constant - already in constants
            let id = e.u.str();
            Some(LuaValue::string(id))
        }
        ExpKind::VK => {
            // Constant in K
            let idx = e.u.info() as usize;
            if idx < fs.chunk.constants.len() {
                Some(fs.chunk.constants[idx])
            } else {
                None
            }
        }
        ExpKind::VKINT => Some(LuaValue::integer(e.u.ival())),
        ExpKind::VKFLT => {
            let val = e.u.nval();
            // Reject -0.0 as compile-time constant (like official Lua)
            // Official Lua doesn't optimize -0.0 to RDKCTC because it needs special handling
            if val.to_bits() == 0x8000000000000000 {
                return None;
            }
            Some(LuaValue::float(val))
        }
        ExpKind::VCONST => {
            // Get from actvar array (port of const2val from lcode.c:75-78)
            let vidx = e.u.info() as usize;
            if let Some(var_desc) = fs.actvar.get(vidx) {
                var_desc.const_value
            } else {
                None
            }
        }
        _ => None,
    };

    result
}

// LUA_MULTRET constant from lua.h
pub const LUA_MULTRET: u32 = u32::MAX;

// Port of hasmultret from lcode.c:86-90
pub fn hasmultret(e: &ExpDesc) -> bool {
    matches!(e.kind, ExpKind::VCALL | ExpKind::VVARARG)
}

// Port of luaK_setlist from lcode.c:1892-1911
pub fn setlist(fs: &mut FuncState, base: u8, nelems: u32, tostore: u32) {
    debug_assert!(tostore != 0);

    let c = if tostore == LUA_MULTRET { 0 } else { tostore };

    // SETLIST uses vABCk format, so vC field is 10 bits (0-1023), not 8 bits (0-255)
    // From lcode.c:1896: if (nelems <= MAXARG_vC)
    if nelems <= Instruction::MAX_V_C {
        code_vabck(fs, OpCode::SetList, base as u32, c, nelems, false);
    } else {
        // Need extra argument for large index (lcode.c:1898-1901)
        let extra = nelems / (Instruction::MAX_V_C + 1);
        let c_arg = nelems % (Instruction::MAX_V_C + 1);
        code_vabck(fs, OpCode::SetList, base as u32, c, c_arg, true);
        code_extraarg(fs, extra);
    }

    fs.freereg = base + 1; // free registers with list values
}

// Port of luaK_settablesize from lcode.c:1793-1801
// void luaK_settablesize (FuncState *fs, int pc, int ra, int asize, int hsize)
pub fn settablesize(fs: &mut FuncState, pc: usize, ra: u8, asize: u32, hsize: u32) {
    // B field: hash size (lcode.c:1795)
    // rb = (hsize != 0) ? luaO_ceillog2(hsize) + 1 : 0
    let rb = if hsize != 0 {
        // Ceiling of log2(hsize) + 1
        let bits = 32 - (hsize - 1).leading_zeros();
        bits + 1
    } else {
        0
    };

    // C field: lower bits of array size (lcode.c:1877)
    // rc = asize % (MAXARG_vC + 1)
    // Note: NEWTABLE uses vABC format, so vC field is 10 bits (0-1023), not 8 bits (0-255)
    let rc = asize % (Instruction::MAX_V_C + 1);

    // EXTRAARG: higher bits of array size (lcode.c:1876)
    // extra = asize / (MAXARG_vC + 1)
    let extra = asize / (Instruction::MAX_V_C + 1);
    // k flag: true if needs EXTRAARG (lcode.c:1878)
    let k = extra > 0;

    // Update the NEWTABLE instruction (lcode.c:1880)
    let inst = &mut fs.chunk.code[pc];
    let opcode = Instruction::get_opcode(*inst);
    debug_assert_eq!(opcode, OpCode::NewTable);

    // NEWTABLE uses vABCk format (10-bit vC field), not ABCk format (8-bit C field)
    *inst = Instruction::create_vabck(OpCode::NewTable, ra as u32, rb, rc, k);

    // Update EXTRAARG instruction (lcode.c:1881)
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
    fs.chunk.line_info.push(fs.lexer.lastline as u32); // Use lastline
    fs.pc += 1;
    pc
}

// Port of luaK_codecheckglobal from lcode.c:715-726
// void luaK_codecheckglobal (FuncState *fs, expdesc *var, int k, int line)
pub fn codecheckglobal(fs: &mut FuncState, var: &mut ExpDesc, mut k: i32, line: usize) {
    // lcode.c:716: luaK_exp2anyreg(fs, var);
    exp2anyreg(fs, var);

    // lcode.c:717: luaK_fixline(fs, line);
    fixline(fs, line);

    // lcode.c:718: k = (k >= MAXARG_Bx) ? 0 : k + 1;
    // This modifies k in-place!
    const MAX_BX: i32 = (1 << 17) - 1; // 17 bits for Bx field
    k = if k >= MAX_BX { 0 } else { k + 1 };

    // lcode.c:719: luaK_codeABx(fs, OP_ERRNNIL, var->u.info, k);
    code_abx(fs, OpCode::ErrNNil, var.u.info() as u32, k as u32);

    // lcode.c:720: luaK_fixline(fs, line);
    fixline(fs, line);

    // lcode.c:721: freeexp(fs, var);
    // Port of freeexp from lcode.c:525-527
    // static void freeexp (FuncState *fs, expdesc *e) {
    //   if (e->k == VNONRELOC)
    //     freereg(fs, e->u.info);
    // }
    if var.kind == ExpKind::VNONRELOC {
        // freereg is just freering = reg (lcode.c:401-404)
        // We don't have a freereg function, so just decrease freereg
        // Actually we should call reserveregs with negative value, but that's not how Rust works
        // In Lua, freereg(fs, r) does: fs->freereg = r (lcode.c:403)
        let reg = var.u.info() as u8;
        if fs.freereg > reg {
            fs.freereg = reg;
        }
    }
}
