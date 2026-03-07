/// Loop body analysis for JIT compilation.
///
/// Scans the bytecode between a `ForPrep` and its matching `ForLoop`,
/// decides whether the body is safe to JIT (integer arithmetic only),
/// and returns a decoded form that the compiler can work with directly.
use crate::lua_value::Chunk;
use crate::lua_vm::OpCode;

/// A single decoded body instruction (MmBin companion stripped out).
#[derive(Debug, Clone, Copy)]
pub enum BodyInstr {
    // ── Register-register arithmetic (Add/Sub/Mul + MmBin) ──────────────
    AddRR { dest: u8, lhs: u8, rhs: u8 },
    SubRR { dest: u8, lhs: u8, rhs: u8 },
    MulRR { dest: u8, lhs: u8, rhs: u8 },
    /// R[dest] = R[B] // R[C]   (floor div, from IDiv + MmBin)
    IDivRR { dest: u8, lhs: u8, rhs: u8 },
    /// R[dest] = R[B] % R[C]   (Lua mod, from Mod + MmBin)
    ModRR  { dest: u8, lhs: u8, rhs: u8 },

    // ── Register-immediate arithmetic ────────────────────────────────────
    /// R[dest] = R[src] + imm  (from AddI or AddK with integer const)
    AddImm  { dest: u8, src: u8, imm: i64 },
    /// R[dest] = R[src] - imm  (from SubK with integer const)
    SubImm  { dest: u8, src: u8, imm: i64 },
    /// R[dest] = R[src] * imm  (from MulK with integer const)
    MulImm  { dest: u8, src: u8, imm: i64 },
    /// R[dest] = R[src] // imm  (floor div by non-zero integer const)
    IDivImm { dest: u8, src: u8, imm: i64 },
    /// R[dest] = R[src] % imm   (Lua mod by non-zero integer const)
    ModImm  { dest: u8, src: u8, imm: i64 },

    // ── Bitwise register-register (BAnd/BOr/BXor/Shl/Shr + MmBin) ───────
    BAndRR { dest: u8, lhs: u8, rhs: u8 },
    BOrRR  { dest: u8, lhs: u8, rhs: u8 },
    BXorRR { dest: u8, lhs: u8, rhs: u8 },

    // ── Bitwise register-immediate (*K + MmBinK) ─────────────────────────
    BAndImm { dest: u8, src: u8, imm: i64 },
    BOrImm  { dest: u8, src: u8, imm: i64 },
    BXorImm { dest: u8, src: u8, imm: i64 },

    // ── Unary (no MmBin companion) ────────────────────────────────────────
    /// R[dest] = -R[src]
    Unm  { dest: u8, src: u8 },
    /// R[dest] = ~R[src]
    BNot { dest: u8, src: u8 },

    // ── Cheap data-movement ops ───────────────────────────────────────────
    Move  { dest: u8, src: u8 },
    LoadI { dest: u8, imm: i64 },

    // ── Shift ops ────────────────────────────────────────────────────────
    /// R[dest] = lua_shiftr(R[src], imm)  (from ShrI + MmBinI)
    /// imm is a compile-time constant shift amount:
    ///   imm in [1, 63] → ushr(src, imm);  imm in [-63, -1] → ishl(src, -imm);
    ///   |imm| >= 64 → 0;  imm == 0 → src unchanged.
    ShrImm { dest: u8, src: u8, imm: i64 },
    /// R[dest] = lua_shiftl(imm_val, R[src])  (from ShlI + MmBinI)
    /// The constant `imm_val` is the VALUE being shifted; R[src] is the shift count.
    ShlIConst { dest: u8, src: u8, imm: i64 },
    /// R[dest] = lua_shiftl(R[lhs], R[rhs])  (from Shl + MmBin)
    ShlRR { dest: u8, lhs: u8, rhs: u8 },
    /// R[dest] = lua_shiftr(R[lhs], R[rhs])  (from Shr + MmBin)
    ShrRR { dest: u8, lhs: u8, rhs: u8 },

    // ── Control flow (comparison + Jmp fused) ────────────────────────────
    /// Compare register vs signed-immediate, then conditional jump.
    /// Encodes a CmpXxx + Jmp pair.
    /// `cc`: 0=Eq, 1=Lt, 2=Le, 3=Gt, 4=Ge
    /// `k`: the `k` flag from the comparison instruction.
    /// If `(R[reg] cc imm) != k` → skip Jmp (fall through to next BodyInstr).
    /// Else → jump to BodyInstr at index `target` (absolute index in body vec).
    CmpImmJmp { reg: u8, imm: i64, cc: u8, k: bool, target: u16 },
    /// Compare register vs register, then conditional jump.
    /// `cc`: 0=Eq, 1=Lt, 2=Le
    CmpRRJmp { lhs: u8, rhs: u8, cc: u8, k: bool, target: u16 },
    /// Unconditional jump to BodyInstr at index `target`.
    Jmp { target: u16 },
}

impl BodyInstr {
    /// The destination register of this instruction, if any.
    /// Control-flow instructions (CmpImmJmp, CmpRRJmp, Jmp) return `None`.
    pub fn dest(&self) -> Option<u8> {
        match self {
            Self::AddRR  { dest, .. } | Self::SubRR  { dest, .. } | Self::MulRR  { dest, .. }
            | Self::IDivRR { dest, .. } | Self::ModRR  { dest, .. }
            | Self::AddImm { dest, .. } | Self::SubImm { dest, .. } | Self::MulImm { dest, .. }
            | Self::IDivImm{ dest, .. } | Self::ModImm { dest, .. }
            | Self::BAndRR { dest, .. } | Self::BOrRR  { dest, .. } | Self::BXorRR { dest, .. }
            | Self::BAndImm{ dest, .. } | Self::BOrImm { dest, .. } | Self::BXorImm{ dest, .. }
            | Self::Unm    { dest, .. } | Self::BNot   { dest, .. }
            | Self::Move   { dest, .. } | Self::LoadI  { dest, .. }
            | Self::ShrImm { dest, .. } | Self::ShlIConst { dest, .. }
            | Self::ShlRR  { dest, .. } | Self::ShrRR  { dest, .. } => Some(*dest),
            Self::CmpImmJmp { .. } | Self::CmpRRJmp { .. } | Self::Jmp { .. } => None,
        }
    }

    /// Fill `buf` with the source register numbers; return how many were written.
    /// Only register operands are returned (immediate values are excluded).
    pub fn source_regs(&self, buf: &mut [u8; 2]) -> usize {
        match self {
            // Two source registers
            Self::AddRR  { lhs, rhs, .. } | Self::SubRR  { lhs, rhs, .. }
            | Self::MulRR  { lhs, rhs, .. } | Self::IDivRR { lhs, rhs, .. }
            | Self::ModRR  { lhs, rhs, .. }
            | Self::BAndRR { lhs, rhs, .. } | Self::BOrRR  { lhs, rhs, .. }
            | Self::BXorRR { lhs, rhs, .. }
            | Self::ShlRR  { lhs, rhs, .. } | Self::ShrRR  { lhs, rhs, .. }
            => { buf[0] = *lhs; buf[1] = *rhs; 2 }
            // One source register
            Self::AddImm { src, .. } | Self::SubImm { src, .. } | Self::MulImm { src, .. }
            | Self::IDivImm{ src, .. } | Self::ModImm { src, .. }
            | Self::BAndImm{ src, .. } | Self::BOrImm { src, .. } | Self::BXorImm{ src, .. }
            | Self::Unm    { src, .. } | Self::BNot   { src, .. }
            | Self::Move   { src, .. }
            | Self::ShrImm { src, .. } | Self::ShlIConst { src, .. }
            => { buf[0] = *src; 1 }
            // One source register (comparison vs imm)
            Self::CmpImmJmp { reg, .. } => { buf[0] = *reg; 1 }
            // Two source registers (comparison reg vs reg)
            Self::CmpRRJmp { lhs, rhs, .. } => { buf[0] = *lhs; buf[1] = *rhs; 2 }
            // No source register
            Self::LoadI { .. } | Self::Jmp { .. } => 0,
        }
    }
}

/// Result of a successful loop analysis.
pub struct LoopAnalysis {
    /// `a` field from the ForPrep instruction (loop vars at base+a, a+1, a+2).
    pub a: u8,
    /// Bytecode index of the ForLoop instruction.
    pub for_loop_pc: usize,
    /// Decoded body instructions (MmBin* companions already removed).
    pub body: Vec<BodyInstr>,
    /// Registers *written* by the body — these become loop-carried SSA vars.
    pub written: Vec<u8>,
    /// Subset of `written` that are **read before first written** in the body.
    /// These registers must hold valid integers at loop entry (type-checked).
    /// Pure body-local temporaries (written before first use) are NOT here.
    pub loop_carried: Vec<u8>,
}

/// Try to analyze the integer for-loop whose `ForPrep` is at `prep_pc`.
///
/// Returns `None` if the loop cannot be JIT-compiled
/// (e.g. non-integer, function calls, upvalue access, …).
pub fn analyze(chunk: &Chunk, prep_pc: usize) -> Option<LoopAnalysis> {
    let code = &chunk.code;

    let prep = code.get(prep_pc)?;
    let a   = prep.get_a() as u8;
    let bx  = prep.get_bx() as usize;

    // ForPrep skips (bx + 1) instructions when the loop should not execute.
    // So ForLoop is at prep_pc + bx + 1.
    let for_loop_pc = prep_pc + bx + 1;
    if for_loop_pc >= code.len() {
        return None;
    }
    if code[for_loop_pc].get_opcode() != OpCode::ForLoop {
        return None;
    }

    // Body bytecodes: [prep_pc+1 .. for_loop_pc)
    let raw_body = &code[prep_pc + 1..for_loop_pc];
    if raw_body.is_empty() {
        return None;
    }

    let mut body: Vec<BodyInstr> = Vec::new();
    let mut written: Vec<u8>     = Vec::new();
    let mut reads:   Vec<u8>     = Vec::new();

    // raw_offset → body index mapping (for resolving jump targets)
    let mut raw_to_body: Vec<(usize, usize)> = Vec::new();
    // deferred target fixups: (body_index, raw_body_target_offset)
    let mut fixups: Vec<(usize, usize)> = Vec::new();

    let mut i = 0;
    while i < raw_body.len() {
        raw_to_body.push((i, body.len()));
        let instr = raw_body[i];
        match instr.get_opcode() {
            // ── register-register arithmetic (followed by MmBin) ──────────
            OpCode::Add => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::AddRR { dest, lhs, rhs });
                i += 2;
            }
            OpCode::Sub => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::SubRR { dest, lhs, rhs });
                i += 2;
            }
            OpCode::Mul => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::MulRR { dest, lhs, rhs });
                i += 2;
            }
            OpCode::IDiv => {
                // Floor division: reg // reg.  Non-zero divisor is verified at
                // runtime in the compiled loop (deopt on zero).
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::IDivRR { dest, lhs, rhs });
                i += 2;
            }
            OpCode::Mod => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::ModRR { dest, lhs, rhs });
                i += 2;
            }
            // ── register-immediate arithmetic ─────────────────────────────
            OpCode::AddI => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = instr.get_sc() as i64;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::AddImm { dest, src, imm });
                i += 2;
            }
            // ── register-constant arithmetic (*K + MmBinK) ────────────────
            OpCode::AddK => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = const_int(chunk, instr.get_c() as usize)?;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::AddImm { dest, src, imm });
                i += 2;
            }
            OpCode::SubK => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = const_int(chunk, instr.get_c() as usize)?;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::SubImm { dest, src, imm });
                i += 2;
            }
            OpCode::MulK => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = const_int(chunk, instr.get_c() as usize)?;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::MulImm { dest, src, imm });
                i += 2;
            }
            OpCode::IDivK => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = const_int(chunk, instr.get_c() as usize)?;
                if imm == 0 { return None; }  // constant div-by-zero: bail
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::IDivImm { dest, src, imm });
                i += 2;
            }
            OpCode::ModK => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = const_int(chunk, instr.get_c() as usize)?;
                if imm == 0 { return None; }  // constant mod-by-zero: bail
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::ModImm { dest, src, imm });
                i += 2;
            }
            // ── bitwise register-register (BAnd/BOr/BXor + MmBin) ─────────
            OpCode::BAnd => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::BAndRR { dest, lhs, rhs });
                i += 2;
            }
            OpCode::BOr => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::BOrRR { dest, lhs, rhs });
                i += 2;
            }
            OpCode::BXor => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::BXorRR { dest, lhs, rhs });
                i += 2;
            }
            // ── bitwise register-constant (*K + MmBinK) ───────────────────
            OpCode::BAndK => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = const_int(chunk, instr.get_c() as usize)?;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::BAndImm { dest, src, imm });
                i += 2;
            }
            OpCode::BOrK => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = const_int(chunk, instr.get_c() as usize)?;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::BOrImm { dest, src, imm });
                i += 2;
            }
            OpCode::BXorK => {
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = const_int(chunk, instr.get_c() as usize)?;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::BXorImm { dest, src, imm });
                i += 2;
            }
            // ── unary ops (no MmBin companion for integer operands) ────────
            OpCode::Unm => {
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::Unm { dest, src });
                i += 1;
            }
            OpCode::BNot => {
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::BNot { dest, src });
                i += 1;
            }
            // ── cheap non-arithmetic ops (no MmBin companion) ─────────────
            OpCode::Move => {
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::Move { dest, src });
                i += 1;
            }
            OpCode::LoadI => {
                let dest = instr.get_a() as u8;
                let imm  = instr.get_sbx() as i64;
                track(&mut written, dest);
                body.push(BodyInstr::LoadI { dest, imm });
                i += 1;
            }
            // LoadK with an integer constant → same as LoadI (no companion)
            OpCode::LoadK => {
                let dest = instr.get_a() as u8;
                let imm  = const_int(chunk, instr.get_bx() as usize)?;
                track(&mut written, dest);
                body.push(BodyInstr::LoadI { dest, imm });
                i += 1;
            }
            // ── shift ops (followed by MmBin companion) ───────────────────
            OpCode::ShrI => {
                // R[A] = lua_shiftr(R[B], sC)
                // sC is the constant shift amount (signed).
                // In the fast integer path, pc += 1 to skip the MmBinI companion.
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let src  = instr.get_b() as u8;
                let imm  = instr.get_sc() as i64;
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::ShrImm { dest, src, imm });
                i += 2;
            }
            OpCode::ShlI => {
                // R[A] = lua_shiftl(sC, R[B])   ← note: constant is the VALUE, register is the COUNT
                if !expect_mmbin(raw_body, i) { return None; }
                let dest    = instr.get_a() as u8;
                let src     = instr.get_b() as u8;  // register holding the shift count
                let imm_val = instr.get_sc() as i64; // the constant value being shifted
                track(&mut written, dest);
                track(&mut reads, src);
                body.push(BodyInstr::ShlIConst { dest, src, imm: imm_val });
                i += 2;
            }
            OpCode::Shl => {
                // R[A] = lua_shiftl(R[B], R[C])
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::ShlRR { dest, lhs, rhs });
                i += 2;
            }
            OpCode::Shr => {
                // R[A] = lua_shiftr(R[B], R[C]) = lua_shiftl(R[B], -R[C])
                if !expect_mmbin(raw_body, i) { return None; }
                let dest = instr.get_a() as u8;
                let lhs  = instr.get_b() as u8;
                let rhs  = instr.get_c() as u8;
                track(&mut written, dest);
                track(&mut reads, lhs);
                track(&mut reads, rhs);
                body.push(BodyInstr::ShrRR { dest, lhs, rhs });
                i += 2;
            }
            // ── comparison + Jmp fused pairs ──────────────────────────────
            // Pattern: CmpXxx A sB k  followed by  Jmp sJ
            // Semantics: if (R[A] cc sB) != k then skip Jmp (fall to next)
            //            else pc = pc_after_jmp + sJ
            OpCode::EqI | OpCode::LtI | OpCode::LeI | OpCode::GtI | OpCode::GeI => {
                // Next instruction must be Jmp
                if i + 1 >= raw_body.len() { return None; }
                let jmp_instr = raw_body[i + 1];
                if jmp_instr.get_opcode() != OpCode::Jmp { return None; }

                let reg = instr.get_a() as u8;
                let imm = instr.get_sb() as i64;
                let k   = instr.get_k();
                let cc  = match instr.get_opcode() {
                    OpCode::EqI => 0u8,
                    OpCode::LtI => 1,
                    OpCode::LeI => 2,
                    OpCode::GtI => 3,
                    OpCode::GeI => 4,
                    _ => unreachable!(),
                };
                // sJ is relative to (i+2), i.e. after the Jmp instruction itself.
                // Target in raw_body = i + 2 + sJ
                let sj = jmp_instr.get_sj();
                let raw_target = (i as isize + 2 + sj as isize) as usize;

                track(&mut reads, reg);
                let body_idx = body.len();
                body.push(BodyInstr::CmpImmJmp { reg, imm, cc, k, target: 0 });
                fixups.push((body_idx, raw_target));
                i += 2;
            }
            OpCode::Eq | OpCode::Lt | OpCode::Le => {
                if i + 1 >= raw_body.len() { return None; }
                let jmp_instr = raw_body[i + 1];
                if jmp_instr.get_opcode() != OpCode::Jmp { return None; }

                let lhs = instr.get_a() as u8;
                let rhs = instr.get_b() as u8;
                let k   = instr.get_k();
                let cc  = match instr.get_opcode() {
                    OpCode::Eq => 0u8,
                    OpCode::Lt => 1,
                    OpCode::Le => 2,
                    _ => unreachable!(),
                };
                let sj = jmp_instr.get_sj();
                let raw_target = (i as isize + 2 + sj as isize) as usize;

                track(&mut reads, lhs);
                track(&mut reads, rhs);
                let body_idx = body.len();
                body.push(BodyInstr::CmpRRJmp { lhs, rhs, cc, k, target: 0 });
                fixups.push((body_idx, raw_target));
                i += 2;
            }
            OpCode::Jmp => {
                // Standalone Jmp (e.g. end of then-branch jumping over else-branch)
                let sj = instr.get_sj();
                let raw_target = (i as isize + 1 + sj as isize) as usize;
                let body_idx = body.len();
                body.push(BodyInstr::Jmp { target: 0 });
                fixups.push((body_idx, raw_target));
                i += 1;
            }
            // Anything else (calls, table access, upvalues, …) → bail out
            _ => return None,
        }
    }

    // Record mapping for one-past-end (jump targets may point to body end)
    raw_to_body.push((i, body.len()));

    // Resolve jump targets: map raw_body offsets → body indices
    for (body_idx, raw_target) in &fixups {
        let target_body = raw_to_body.iter()
            .find(|&&(raw_off, _)| raw_off == *raw_target)
            .map(|&(_, bi)| bi);
        let Some(t) = target_body else { return None; };
        if t > u16::MAX as usize { return None; }
        match &mut body[*body_idx] {
            BodyInstr::CmpImmJmp { target, .. }
            | BodyInstr::CmpRRJmp { target, .. }
            | BodyInstr::Jmp { target } => *target = t as u16,
            _ => unreachable!(),
        }
    }

    // Sanity: we need at least one body instruction that actually modifies state
    if written.is_empty() {
        return None;
    }

    // Determine which written registers are "loop-carried":
    // a register is loop-carried if it is first *read* before it is first
    // *written* in a single pass through the body.  Pure temporaries (always
    // written before their first use) are NOT loop-carried.
    let mut loop_carried: Vec<u8> = Vec::new();
    let mut defined_in_body: Vec<u8> = Vec::new();
    let mut src_buf = [0u8; 2];
    for instr in &body {
        let n = instr.source_regs(&mut src_buf);
        for &s in &src_buf[..n] {
            // A source that is in `written` but not yet defined this iteration
            // must carry its value from the previous iteration (or from before
            // the loop on the first iteration) → loop-carried.
            if written.contains(&s) && !defined_in_body.contains(&s) {
                track(&mut loop_carried, s);
            }
        }
        defined_in_body.extend(instr.dest());
    }

    Some(LoopAnalysis { a, for_loop_pc, body, written, loop_carried })
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn track(vec: &mut Vec<u8>, r: u8) {
    if !vec.contains(&r) {
        vec.push(r);
    }
}

/// Returns `true` if `code[at + 1]` exists and is any `MmBin*` companion opcode.
///
/// All arithmetic/bitwise binary opcodes that the JIT handles emit one of
/// {`MmBin`, `MmBinI`, `MmBinK`} as a metamethod fallback guard immediately
/// after the main instruction.  We skip it (`i += 2`) when recognising these
/// instructions.
fn expect_mmbin(code: &[crate::lua_vm::Instruction], at: usize) -> bool {
    code.get(at + 1)
        .map(|instr| matches!(
            instr.get_opcode(),
            OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK
        ))
        .unwrap_or(false)
}

/// Look up `chunk.constants[idx]` and return its value as `i64` if it is a
/// strict integer (`LUA_VNUMINT`).  Returns `None` otherwise.
fn const_int(chunk: &Chunk, idx: usize) -> Option<i64> {
    chunk.constants.get(idx)?.as_integer_strict()
}
