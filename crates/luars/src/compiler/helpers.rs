// Helper functions for code generation (对齐lcode.c)
use super::*;
use crate::lua_value::LuaValue;
use crate::lua_vm::{Instruction, OpCode};

/// NO_JUMP constant - invalid jump position
pub const NO_JUMP: i32 = -1;

/// Maximum number of registers in a Lua function
const MAXREGS: u32 = 255;

/// Emit an instruction and return its position (对齐luaK_code)
pub(crate) fn code(c: &mut Compiler, instr: u32) -> usize {
    let pos = c.chunk.code.len();
    c.chunk.code.push(instr);
    // Save line info
    c.chunk.line_info.push(c.last_line);
    pos
}

/// Emit an ABC instruction (internal)
fn code_abc_internal(c: &mut Compiler, op: OpCode, a: u32, b: u32, bc: u32) -> usize {
    let inst = Instruction::create_abc(op, a, b, bc);
    code(c, inst)
}

/// Emit an ABC instruction (对齐luaK_codeABC)
pub(crate) fn code_abc(c: &mut Compiler, op: OpCode, a: u32, b: u32, bc: u32) -> usize {
    code_abc_internal(c, op, a, b, bc)
}

/// Emit an ABCk instruction (对齐luaK_codeABCk)
pub(crate) fn code_abck(c: &mut Compiler, op: OpCode, a: u32, b: u32, bc: u32, k: bool) -> usize {
    let instr = Instruction::create_abck(op, a, b, bc, k);
    code(c, instr)
}

/// Emit an ABx instruction (对齐luaK_codeABx)
pub(crate) fn code_abx(c: &mut Compiler, op: OpCode, a: u32, bx: u32) -> usize {
    let instr = Instruction::create_abx(op, a, bx);
    code(c, instr)
}

/// Emit an AsBx instruction (对齐codeAsBx)
pub(crate) fn code_asbx(c: &mut Compiler, op: OpCode, a: u32, sbx: i32) -> usize {
    let instr = Instruction::create_asbx(op, a, sbx);
    code(c, instr)
}

/// Emit an sJ instruction (对齐codesJ)
pub(crate) fn code_sj(c: &mut Compiler, op: OpCode, sj: i32) -> usize {
    let instr = Instruction::create_sj(op, sj);
    code(c, instr)
}

/// Emit a JMP instruction and return its position (对齐luaK_jump)
pub(crate) fn jump(c: &mut Compiler) -> usize {
    code_sj(c, OpCode::Jmp, NO_JUMP)
}

/// Get current code position (label) (对齐luaK_getlabel)
pub(crate) fn get_label(c: &Compiler) -> usize {
    c.chunk.code.len()
}

/// Fix a jump instruction to jump to dest (对齐fixjump)
pub(crate) fn fix_jump(c: &mut Compiler, pc: usize, dest: usize) {
    let offset = (dest as i32) - (pc as i32) - 1;
    if offset < -0x1FFFFFF || offset > 0x1FFFFFF {
        // Error: control structure too long
        return;
    }
    let mut instr = c.chunk.code[pc];
    Instruction::set_sj(&mut instr, offset);
    c.chunk.code[pc] = instr;
}

/// Get jump destination (对齐getjump)
fn get_jump(c: &Compiler, pc: usize) -> i32 {
    let instr = c.chunk.code[pc];
    let offset = Instruction::get_sj(instr);
    if offset == NO_JUMP {
        NO_JUMP
    } else {
        (pc as i32) + 1 + offset
    }
}

/// Concatenate jump lists (对齐luaK_concat)
pub(crate) fn concat(c: &mut Compiler, l1: &mut i32, l2: i32) {
    if l2 == NO_JUMP {
        return;
    }
    if *l1 == NO_JUMP {
        *l1 = l2;
    } else {
        let mut list = *l1;
        loop {
            let next = get_jump(c, list as usize);
            if next == NO_JUMP {
                break;
            }
            list = next;
        }
        fix_jump(c, list as usize, l2 as usize);
    }
}

/// Patch jump list to target (对齐luaK_patchlist)
pub(crate) fn patch_list(c: &mut Compiler, mut list: i32, target: usize) {
    while list != NO_JUMP {
        let next = get_jump(c, list as usize);
        fix_jump(c, list as usize, target);
        list = next;
    }
}

/// Patch jump list to current position (对齐luaK_patchtohere)
pub(crate) fn patch_to_here(c: &mut Compiler, list: i32) {
    let here = get_label(c);
    patch_list(c, list, here);
}

/// Add constant to constant table (对齐addk)
pub(crate) fn add_constant(c: &mut Compiler, value: LuaValue) -> u32 {
    // Try to reuse existing constant
    for (i, k) in c.chunk.constants.iter().enumerate() {
        if k.raw_equal(&value) {
            return i as u32;
        }
    }
    // Add new constant
    let idx = c.chunk.constants.len() as u32;
    c.chunk.constants.push(value);
    idx
}

/// Add string constant (对齐stringK)
pub(crate) fn string_k(_c: &mut Compiler, _s: String) -> u32 {
    // TODO: Need to implement string interning through VM
    // For now, return placeholder constant index
    // This needs to be fixed when we implement proper constant handling
    0
}

/// Add integer constant (对齐luaK_intK)
pub(crate) fn int_k(c: &mut Compiler, n: i64) -> u32 {
    add_constant(c, LuaValue::integer(n))
}

/// Add number constant (对齐luaK_numberK)
pub(crate) fn number_k(c: &mut Compiler, n: f64) -> u32 {
    add_constant(c, LuaValue::number(n))
}

/// Emit LOADNIL instruction with optimization (对齐luaK_nil)
pub(crate) fn nil(c: &mut Compiler, from: u32, n: u32) {
    if n == 0 {
        return;
    }
    // TODO: optimize by merging with previous LOADNIL
    code_abc(c, OpCode::LoadNil, from, n - 1, 0);
}

/// Reserve n registers (对齐luaK_reserveregs)
pub(crate) fn reserve_regs(c: &mut Compiler, n: u32) {
    check_stack(c, n);
    c.freereg += n;
}

/// Check if we need more stack space (对齐luaK_checkstack)
pub(crate) fn check_stack(c: &mut Compiler, n: u32) {
    let newstack = c.freereg + n;
    if newstack > c.peak_freereg {
        c.peak_freereg = newstack;
    }
    if newstack > MAXREGS {
        // Error: function needs too many registers
        return;
    }
    if (newstack as usize) > c.chunk.max_stack_size {
        c.chunk.max_stack_size = newstack as usize;
    }
}

/// Emit RETURN instruction (对齐luaK_ret)
pub(crate) fn ret(c: &mut Compiler, first: u32, nret: i32) {
    let op = match nret {
        0 => OpCode::Return0,
        1 => OpCode::Return1,
        _ => OpCode::Return,
    };
    if matches!(op, OpCode::Return) {
        code_abc(c, op, first, (nret + 1) as u32, 0);
    } else if matches!(op, OpCode::Return1) {
        code_abc(c, op, first, 0, 0);
    } else {
        code_abc(c, op, 0, 0, 0);
    }
}

/// Get number of active variables in register stack (对齐luaY_nvarstack)
pub(crate) fn nvarstack(c: &Compiler) -> u32 {
    // Count locals that are in registers (not compile-time constants)
    let mut count = 0;
    for local in c.scope_chain.borrow().locals.iter() {
        if !local.is_const {
            count = count.max(local.reg + 1);
        }
    }
    count
}

/// Free a register (对齐freereg)
pub(crate) fn free_reg(c: &mut Compiler, reg: u32) {
    if reg >= nvarstack(c) {
        c.freereg -= 1;
        debug_assert!(reg == c.freereg);
    }
}

/// Jump to specific label (对齐luaK_jumpto)
pub(crate) fn jump_to(c: &mut Compiler, target: usize) {
    let here = get_label(c);
    let offset = (target as i32) - (here as i32) - 1;
    code_sj(c, crate::lua_vm::OpCode::Jmp, offset);
}

/// Fix for loop jump instruction (对齐fixforjump)
/// Used to patch FORPREP and FORLOOP instructions with correct jump offsets
pub(crate) fn fix_for_jump(c: &mut Compiler, pc: usize, dest: usize, back: bool) {
    use crate::lua_vm::Instruction;
    
    let mut offset = (dest as i32) - (pc as i32) - 1;
    if back {
        offset = -offset;
    }
    
    // Check if offset fits in Bx field (18 bits unsigned)
    if offset < 0 || offset > 0x3FFFF {
        panic!("Control structure too long");
    }
    
    // Get the instruction and modify its Bx field
    let mut instr = c.chunk.code[pc];
    Instruction::set_bx(&mut instr, offset as u32);
    c.chunk.code[pc] = instr;
}
