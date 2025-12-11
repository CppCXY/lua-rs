/// Binary operator infix/posfix handling (Lua 5.4 compatible)
/// Mirrors lcode.c: luaK_infix and luaK_posfix
use super::Compiler;
use super::exp2reg::*;
use super::expdesc::*;
use super::helpers::*;
use crate::lua_vm::{Instruction, OpCode};
use emmylua_parser::BinaryOperator;

/// Process first operand of binary operation before reading second operand
/// Lua equivalent: luaK_infix (lcode.c L1637-1679)
pub fn luak_infix(c: &mut Compiler, op: BinaryOperator, v: &mut ExpDesc) {
    match op {
        BinaryOperator::OpAnd => {
            luak_goiftrue(c, v);
        }
        BinaryOperator::OpOr => {
            luak_goiffalse(c, v);
        }
        BinaryOperator::OpConcat => {
            // CRITICAL: operand must be on the stack (in a register)
            // This ensures consecutive register allocation for concatenation
            exp_to_next_reg(c, v);
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
            // For arithmetic/bitwise: only discharge if not already in a register or constant
            // VLocal and VNonReloc are already in registers, don't copy them!
            if !matches!(v.kind, ExpKind::VKInt | ExpKind::VKFlt | ExpKind::VLocal | ExpKind::VNonReloc) {
                discharge_vars(c, v);
            }
        }
        BinaryOperator::OpEq | BinaryOperator::OpNe => {
            // For equality: can use constants
            // Official Lua: exp2RK(fs, v) - converts to register or constant
            // We'll discharge to any register for now
            if !matches!(v.kind, ExpKind::VK | ExpKind::VKInt | ExpKind::VKFlt) {
                let _reg = exp_to_any_reg(c, v);
            }
        }
        BinaryOperator::OpLt | BinaryOperator::OpLe | BinaryOperator::OpGt | BinaryOperator::OpGe => {
            // For comparisons: discharge to register
            if !matches!(v.kind, ExpKind::VKInt | ExpKind::VKFlt) {
                let _reg = exp_to_any_reg(c, v);
            }
        }
        _ => {}
    }
}

/// Finalize code for binary operation after reading second operand
/// Lua equivalent: luaK_posfix (lcode.c L1706-1760)
pub fn luak_posfix(
    c: &mut Compiler,
    op: BinaryOperator,
    e1: &mut ExpDesc,
    e2: &mut ExpDesc,
    _line: usize,
) -> Result<(), String> {
    // First discharge vars on e2
    discharge_vars(c, e2);

    match op {
        BinaryOperator::OpAnd => {
            lua_assert(e1.t == NO_JUMP, "e1.t should be NO_JUMP for AND");
            luak_concat(c, &mut e2.f, e1.f);
            *e1 = e2.clone();
        }
        BinaryOperator::OpOr => {
            lua_assert(e1.f == NO_JUMP, "e1.f should be NO_JUMP for OR");
            luak_concat(c, &mut e2.t, e1.t);
            *e1 = e2.clone();
        }
        BinaryOperator::OpConcat => {
            // e1 .. e2
            // CRITICAL: Ensure e2 goes to the register right after e1
            // If e1 is VReloc (previous CONCAT), get its register from var.ridx
            let e1_reg = if e1.kind == ExpKind::VReloc { e1.var.ridx } else { e1.info };
            // Set freereg to e1_reg+1 so exp_to_next_reg puts e2 there
            c.freereg = e1_reg + 1;
            // Force e2 to next register (consecutive with e1)
            exp_to_next_reg(c, e2);
            codeconcat(c, e1, e2)?;
        }
        BinaryOperator::OpAdd | BinaryOperator::OpMul => {
            // Commutative operations
            codecommutative(c, op, e1, e2)?;
        }
        BinaryOperator::OpSub
        | BinaryOperator::OpDiv
        | BinaryOperator::OpIDiv
        | BinaryOperator::OpMod
        | BinaryOperator::OpPow => {
            codearith(c, op, e1, e2)?;
        }
        BinaryOperator::OpBAnd | BinaryOperator::OpBOr | BinaryOperator::OpBXor => {
            codebitwise(c, op, e1, e2)?;
        }
        BinaryOperator::OpShl | BinaryOperator::OpShr => {
            codebitwise(c, op, e1, e2)?;
        }
        BinaryOperator::OpEq | BinaryOperator::OpNe => {
            codecomp(c, op, e1, e2)?;
        }
        BinaryOperator::OpLt | BinaryOperator::OpLe | BinaryOperator::OpGt | BinaryOperator::OpGe => {
            codecomp(c, op, e1, e2)?;
        }
        _ => {
            return Err(format!("Unsupported binary operator: {:?}", op));
        }
    }

    Ok(())
}

//======================================================================================
// Helper functions for posfix operations
//======================================================================================

/// Create code for '(e1 .. e2)' - Lua equivalent: codeconcat (lcode.c L1686-1700)
fn codeconcat(c: &mut Compiler, e1: &mut ExpDesc, e2: &ExpDesc) -> Result<(), String> {
    // OFFICIAL LUA: lcode.c L1686-1700
    // Check if e2's last instruction is a CONCAT (merge optimization)
    if e2.kind == ExpKind::VReloc && c.chunk.code.len() > 0 {
        let ie2_pc = e2.info as usize;
        if ie2_pc < c.chunk.code.len() {
            let ie2 = c.chunk.code[ie2_pc];
            if Instruction::get_opcode(ie2) == OpCode::Concat {
                let n = Instruction::get_b(ie2); // # of elements concatenated in e2
                // Get e1's register: if VReloc use var.ridx, else use info
                let e1_reg = if e1.kind == ExpKind::VReloc { e1.var.ridx } else { e1.info };
                // Check if e1 ends just before e2's concatenation starts
                let e2_start_reg = Instruction::get_a(ie2);
                lua_assert(
                    e1_reg == e2_start_reg - 1,
                    "CONCAT merge: e1 must be just before e2",
                );
                let result_reg = e1_reg;
                free_exp(c, e1);
                // Merge: extend e2's CONCAT to include e1
                c.chunk.code[ie2_pc] = Instruction::encode_abc(OpCode::Concat, result_reg, n + 1, 0);
                // Result is still VReloc pointing to the merged instruction
                e1.kind = ExpKind::VReloc;
                e1.info = ie2_pc as u32;
                e1.var.ridx = result_reg;
                return Ok(());
            }
        }
    }

    // e2 is not a concatenation - emit new CONCAT
    // CRITICAL: Do NOT call exp_to_next_reg(e1) - e1 is already in the correct register!
    // Official Lua: luaK_codeABC(fs, OP_CONCAT, e1->u.info, 2, 0);
    let a_value = e1.info;
    let pc = c.chunk.code.len();
    emit(c, Instruction::encode_abc(OpCode::Concat, a_value, 2, 0));
    free_exp(c, e2);
    // CRITICAL: CONCAT consumes R[A+1] to R[A+B-1], result is in R[A]
    // Set freereg to R[A+1] since only R[A] is live after CONCAT
    // This is ESSENTIAL to prevent exp_to_any_reg from allocating wrong registers
    c.freereg = a_value + 1;
    // CRITICAL: Result must be VReloc pointing to the CONCAT instruction
    // This allows subsequent CONCAT operations to detect and merge
    // Store register in var.ridx for merge comparison
    e1.kind = ExpKind::VReloc;
    e1.info = pc as u32;      // Point to the CONCAT instruction
    e1.var.ridx = a_value;    // Save register for merge check
    Ok(())
}

/// Commutative arithmetic operations (ADD, MUL)
fn codecommutative(c: &mut Compiler, op: BinaryOperator, e1: &mut ExpDesc, e2: &ExpDesc) -> Result<(), String> {
    use crate::compiler::tagmethod::TagMethod;
    
    // Get register numbers without copying locals
    let left_reg = match e1.kind {
        ExpKind::VLocal => e1.var.ridx,
        ExpKind::VNonReloc => e1.info,
        _ => exp_to_any_reg(c, e1),
    };
    
    let mut e2_clone = e2.clone();
    let right_reg = match e2_clone.kind {
        ExpKind::VLocal => e2_clone.var.ridx,
        ExpKind::VNonReloc => e2_clone.info,
        _ => exp_to_any_reg(c, &mut e2_clone),
    };
    
    let (opcode, tm) = match op {
        BinaryOperator::OpAdd => (OpCode::Add, TagMethod::Add),
        BinaryOperator::OpMul => (OpCode::Mul, TagMethod::Mul),
        _ => unreachable!(),
    };
    
    // Allocate result register (will be optimized by caller if possible)
    let result_reg = alloc_register(c);
    emit(c, Instruction::encode_abc(opcode, result_reg, left_reg, right_reg));
    
    // Emit MMBIN for metamethod binding (Lua 5.4 optimization)
    emit(c, Instruction::encode_abc(OpCode::MmBin, left_reg, right_reg, tm as u32));
    
    free_exp(c, e2);
    
    e1.kind = ExpKind::VNonReloc;
    e1.info = result_reg;
    Ok(())
}

/// Non-commutative arithmetic operations
fn codearith(c: &mut Compiler, op: BinaryOperator, e1: &mut ExpDesc, e2: &ExpDesc) -> Result<(), String> {
    use crate::compiler::tagmethod::TagMethod;
    
    // Get register numbers without copying locals
    let left_reg = match e1.kind {
        ExpKind::VLocal => e1.var.ridx,
        ExpKind::VNonReloc => e1.info,
        _ => exp_to_any_reg(c, e1),
    };
    
    let mut e2_clone = e2.clone();
    let right_reg = match e2_clone.kind {
        ExpKind::VLocal => e2_clone.var.ridx,
        ExpKind::VNonReloc => e2_clone.info,
        _ => exp_to_any_reg(c, &mut e2_clone),
    };
    
    let (opcode, tm) = match op {
        BinaryOperator::OpSub => (OpCode::Sub, TagMethod::Sub),
        BinaryOperator::OpDiv => (OpCode::Div, TagMethod::Div),
        BinaryOperator::OpIDiv => (OpCode::IDiv, TagMethod::IDiv),
        BinaryOperator::OpMod => (OpCode::Mod, TagMethod::Mod),
        BinaryOperator::OpPow => (OpCode::Pow, TagMethod::Pow),
        _ => unreachable!(),
    };
    
    // Allocate result register (will be optimized by caller if possible)
    let result_reg = alloc_register(c);
    emit(c, Instruction::encode_abc(opcode, result_reg, left_reg, right_reg));
    
    // Emit MMBIN for metamethod binding (Lua 5.4 optimization)
    emit(c, Instruction::encode_abc(OpCode::MmBin, left_reg, right_reg, tm as u32));
    
    free_exp(c, e2);
    
    e1.kind = ExpKind::VNonReloc;
    e1.info = result_reg;
    Ok(())
}

/// Bitwise operations
fn codebitwise(c: &mut Compiler, op: BinaryOperator, e1: &mut ExpDesc, e2: &ExpDesc) -> Result<(), String> {
    use crate::compiler::tagmethod::TagMethod;
    
    // Get register numbers without copying locals
    let left_reg = match e1.kind {
        ExpKind::VLocal => e1.var.ridx,
        ExpKind::VNonReloc => e1.info,
        _ => exp_to_any_reg(c, e1),
    };
    
    let mut e2_clone = e2.clone();
    let right_reg = match e2_clone.kind {
        ExpKind::VLocal => e2_clone.var.ridx,
        ExpKind::VNonReloc => e2_clone.info,
        _ => exp_to_any_reg(c, &mut e2_clone),
    };
    
    let (opcode, tm) = match op {
        BinaryOperator::OpBAnd => (OpCode::BAnd, TagMethod::BAnd),
        BinaryOperator::OpBOr => (OpCode::BOr, TagMethod::BOr),
        BinaryOperator::OpBXor => (OpCode::BXor, TagMethod::BXor),
        BinaryOperator::OpShl => (OpCode::Shl, TagMethod::Shl),
        BinaryOperator::OpShr => (OpCode::Shr, TagMethod::Shr),
        _ => unreachable!(),
    };
    
    // Allocate result register (will be optimized by caller if possible)
    let result_reg = alloc_register(c);
    emit(c, Instruction::encode_abc(opcode, result_reg, left_reg, right_reg));
    
    // Emit MMBIN for metamethod binding (Lua 5.4 optimization)
    emit(c, Instruction::encode_abc(OpCode::MmBin, left_reg, right_reg, tm as u32));
    
    free_exp(c, e2);
    
    e1.kind = ExpKind::VNonReloc;
    e1.info = result_reg;
    Ok(())
}

/// Comparison operations
fn codecomp(c: &mut Compiler, op: BinaryOperator, e1: &mut ExpDesc, e2: &ExpDesc) -> Result<(), String> {
    let left_reg = exp_to_any_reg(c, e1);
    let right_reg = exp_to_any_reg(c, &mut e2.clone());
    
    // Generate comparison + boolean result pattern
    let result_reg = alloc_register(c);
    
    let (opcode, swap) = match op {
        BinaryOperator::OpEq => (OpCode::Eq, false),
        BinaryOperator::OpNe => (OpCode::Eq, false), // NE uses EQ with inverted k
        BinaryOperator::OpLt => (OpCode::Lt, false),
        BinaryOperator::OpLe => (OpCode::Le, false),
        BinaryOperator::OpGt => (OpCode::Lt, true), // GT is LT with swapped operands
        BinaryOperator::OpGe => (OpCode::Le, true), // GE is LE with swapped operands
        _ => unreachable!(),
    };
    
    let k = if matches!(op, BinaryOperator::OpNe) { 1 } else { 0 };
    let (a, b) = if swap { (right_reg, left_reg) } else { (left_reg, right_reg) };
    
    emit(c, Instruction::encode_abc(opcode, a, b, k));
    let jump_pos = emit_jump(c, OpCode::Jmp);
    emit(c, Instruction::encode_abc(OpCode::LFalseSkip, result_reg, 0, 0));
    emit(c, Instruction::encode_abc(OpCode::LoadTrue, result_reg, 0, 0));
    patch_jump(c, jump_pos);
    
    e1.kind = ExpKind::VNonReloc;
    e1.info = result_reg;
    Ok(())
}

//======================================================================================
// Jump list helpers
//======================================================================================

const NO_JUMP: i32 = -1;

fn lua_assert(condition: bool, msg: &str) {
    if !condition {
        eprintln!("Assertion failed: {}", msg);
    }
}

/// Go if true (for AND operator)
fn luak_goiftrue(c: &mut Compiler, e: &mut ExpDesc) {
    discharge_vars(c, e);
    
    match e.kind {
        ExpKind::VTrue | ExpKind::VK | ExpKind::VKInt | ExpKind::VKFlt => {
            // Always true - no code needed
        }
        ExpKind::VFalse | ExpKind::VNil => {
            // Always false - emit unconditional jump
            let pc = emit_jump(c, OpCode::Jmp);
            luak_concat(c, &mut e.f, pc as i32);
            e.t = NO_JUMP;
        }
        _ => {
            // Emit TEST instruction
            let reg = exp_to_any_reg(c, e);
            emit(c, Instruction::create_abck(OpCode::Test, reg, 0, 0, false));
            let pc = emit_jump(c, OpCode::Jmp);
            luak_concat(c, &mut e.f, pc as i32);
            e.t = NO_JUMP;
        }
    }
}

/// Go if false (for OR operator)
fn luak_goiffalse(c: &mut Compiler, e: &mut ExpDesc) {
    discharge_vars(c, e);
    
    match e.kind {
        ExpKind::VFalse | ExpKind::VNil => {
            // Always false - no code needed
        }
        ExpKind::VTrue | ExpKind::VK | ExpKind::VKInt | ExpKind::VKFlt => {
            // Always true - emit unconditional jump
            let pc = emit_jump(c, OpCode::Jmp);
            luak_concat(c, &mut e.t, pc as i32);
            e.f = NO_JUMP;
        }
        _ => {
            // Emit TEST instruction with inverted condition
            let reg = exp_to_any_reg(c, e);
            emit(c, Instruction::create_abck(OpCode::Test, reg, 0, 0, true));
            let pc = emit_jump(c, OpCode::Jmp);
            luak_concat(c, &mut e.t, pc as i32);
            e.f = NO_JUMP;
        }
    }
}

/// Concatenate jump lists (Lua: luaK_concat in lcode.c L182-194)
fn luak_concat(c: &mut Compiler, l1: &mut i32, l2: i32) {
    // Skip marker values (like -2 for inverted simple expressions)
    if l2 == NO_JUMP || l2 < -1 {
        return;
    }
    if *l1 == NO_JUMP || *l1 < -1 {
        *l1 = l2;
    } else {
        let mut list = *l1;
        let mut next;
        loop {
            next = get_jump(c, list as usize);
            if next == NO_JUMP {
                break;
            }
            list = next;
        }
        fix_jump(c, list as usize, l2 as usize);
    }
}

fn get_jump(c: &Compiler, pc: usize) -> i32 {
    // Safety check: if pc is out of bounds, it's likely a marker value like -2
    if pc >= c.chunk.code.len() {
        return NO_JUMP;
    }
    let offset = Instruction::get_sj(c.chunk.code[pc]);
    if offset == NO_JUMP {
        NO_JUMP
    } else {
        (pc as i32 + 1) + offset
    }
}

fn fix_jump(c: &mut Compiler, pc: usize, dest: usize) {
    // Safety check: if pc is out of bounds, skip (it's likely a marker value)
    if pc >= c.chunk.code.len() || dest >= c.chunk.code.len() {
        return;
    }
    let offset = (dest as i32) - (pc as i32) - 1;
    let inst = c.chunk.code[pc];
    let opcode = Instruction::get_opcode(inst);
    c.chunk.code[pc] = Instruction::create_sj(opcode, offset);
}
