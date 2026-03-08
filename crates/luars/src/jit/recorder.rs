//! Trace recorder — records interpreter execution into trace IR.
//!
//! # Architecture
//!
//! The recorder is invoked from the interpreter's backward-jump sites.
//! When a backward jump becomes hot (counter reaches threshold), the
//! interpreter creates a `TraceRecorder` and, on each subsequent
//! instruction dispatch, calls `recorder.record_instruction(...)`.
//!
//! The recorder maps each interpreted instruction to one or more `TraceIr`
//! nodes.  It tracks the type of every referenced stack slot via `GuardType`
//! nodes and takes a `Snapshot` before each guard so the runtime can
//! reconstruct VM state on a side exit.
//!
//! Recording ends when:
//! - The interpreter reaches the same backward-jump target where recording
//!   started → `RecordResult::LoopClosed` (success).
//! - An unsupported opcode is encountered → abort.
//! - The trace exceeds length/exit limits → abort.

use crate::lua_vm::opcode::OpCode;
use crate::lua_vm::Instruction;
use crate::lua_value::LuaValue;

use super::trace::*;

/// Maximum number of IR instructions in a single trace.
const MAX_TRACE_LEN: usize = 4096;

/// Maximum number of snapshots (side exits) per trace.
const MAX_SNAPSHOTS: usize = 256;

/// Maximum inlined call depth during recording.
const MAX_CALL_DEPTH: u32 = 8;

// ── Slot-to-TRef mapping ──────────────────────────────────────────────────────

/// Tracks which trace IR value (`TRef`) currently represents each VM slot.
///
/// During recording, each time a slot is loaded we record a `LoadSlot` +
/// `GuardType` and map `slot → TRef`.  When a slot is stored we update
/// the mapping.  At snapshot time we can dump the mapping into `SnapEntry`s.
#[derive(Clone, Debug, Default)]
struct SlotMap {
    /// `entries[i] = Some((tref, ty))` means slot `i` is live with value
    /// `tref` of type `ty`.  `None` means "not yet touched — inherit from VM".
    entries: Vec<Option<(TRef, IrType)>>,
}

impl SlotMap {
    fn get(&self, slot: u16) -> Option<(TRef, IrType)> {
        self.entries.get(slot as usize).copied().flatten()
    }

    fn set(&mut self, slot: u16, tref: TRef, ty: IrType) {
        let idx = slot as usize;
        if idx >= self.entries.len() {
            self.entries.resize(idx + 1, None);
        }
        self.entries[idx] = Some((tref, ty));
    }
}

// ── TraceRecorder ─────────────────────────────────────────────────────────────

/// The trace recorder, driven by the interpreter one instruction at a time.
pub struct TraceRecorder {
    /// Monotonically increasing trace identifier.
    trace_id: u32,
    /// IR instruction buffer.
    ops: Vec<TraceIr>,
    /// Snapshots taken before guards.
    snapshots: Vec<Snapshot>,
    /// Map from VM stack slot → current TRef.
    slot_map: SlotMap,
    /// The bytecode PC where recording started (the backward-jump target).
    pub head_pc: u32,
    /// Stack base at recording start.
    head_base: usize,
    /// Raw pointer to the Chunk where the trace head lives.
    chunk_ptr: *const u8,
    /// Current call depth relative to the trace entry.
    call_depth: u32,
    /// Whether we have emitted `LoopStart` yet (set on second visit to head).
    loop_started: bool,
    /// How many times we have visited the trace head during recording.
    head_visits: u32,
    /// How many instructions recorded so far.
    len: usize,
    /// Phi node info: (ops_index, slot, entry_tref) for each Phi placeholder.
    /// Filled at LoopStart, backedge patched at LoopEnd.
    phi_entries: Vec<(usize, u16, TRef)>,
}

impl TraceRecorder {
    /// Create a new recorder.
    ///
    /// * `trace_id` — unique ID for this trace.
    /// * `head_pc` — bytecode PC of the backward-jump *target* (loop top).
    /// * `head_base` — stack base at recording start.
    /// * `chunk_ptr` — raw pointer to the Chunk.
    pub fn new(trace_id: u32, head_pc: u32, head_base: usize, chunk_ptr: *const u8) -> Self {
        Self {
            trace_id,
            ops: Vec::with_capacity(256),
            snapshots: Vec::new(),
            slot_map: SlotMap::default(),
            head_pc,
            head_base,
            chunk_ptr,
            call_depth: 0,
            loop_started: false,
            head_visits: 0,
            len: 0,
            phi_entries: Vec::new(),
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────

    /// Emit an IR instruction and return its `TRef`.
    fn emit(&mut self, ir: TraceIr) -> TRef {
        let idx = self.ops.len();
        self.ops.push(ir);
        self.len += 1;
        TRef(idx as u32)
    }

    /// Try to constant-fold a binary IR instruction.
    /// If both operands are KInt/KFloat constants, compute the result at
    /// record time and return a new constant TRef. Otherwise emit the IR.
    fn fold_or_emit(&mut self, ir: TraceIr) -> TRef {
        if let Some(folded) = self.try_fold(&ir) {
            return self.emit(folded);
        }
        self.emit(ir)
    }

    fn try_fold(&self, ir: &TraceIr) -> Option<TraceIr> {
        match ir {
            // Integer arithmetic
            TraceIr::AddInt { lhs, rhs } => self.fold_int_bin(*lhs, *rhs, |a, b| a.wrapping_add(b)),
            TraceIr::SubInt { lhs, rhs } => self.fold_int_bin(*lhs, *rhs, |a, b| a.wrapping_sub(b)),
            TraceIr::MulInt { lhs, rhs } => self.fold_int_bin(*lhs, *rhs, |a, b| a.wrapping_mul(b)),
            TraceIr::IDivInt { lhs, rhs } => {
                if let (TraceIr::KInt(a), TraceIr::KInt(b)) = (&self.ops[lhs.index()], &self.ops[rhs.index()]) {
                    if *b != 0 { Some(TraceIr::KInt(a.wrapping_div(*b))) } else { None }
                } else { None }
            }
            TraceIr::ModInt { lhs, rhs } => {
                if let (TraceIr::KInt(a), TraceIr::KInt(b)) = (&self.ops[lhs.index()], &self.ops[rhs.index()]) {
                    if *b != 0 { Some(TraceIr::KInt(a.wrapping_rem(*b))) } else { None }
                } else { None }
            }
            // Float arithmetic
            TraceIr::AddFloat { lhs, rhs } => self.fold_float_bin(*lhs, *rhs, |a, b| a + b),
            TraceIr::SubFloat { lhs, rhs } => self.fold_float_bin(*lhs, *rhs, |a, b| a - b),
            TraceIr::MulFloat { lhs, rhs } => self.fold_float_bin(*lhs, *rhs, |a, b| a * b),
            TraceIr::DivFloat { lhs, rhs } => self.fold_float_bin(*lhs, *rhs, |a, b| a / b),
            // Bitwise
            TraceIr::BAndInt { lhs, rhs } => self.fold_int_bin(*lhs, *rhs, |a, b| a & b),
            TraceIr::BOrInt  { lhs, rhs } => self.fold_int_bin(*lhs, *rhs, |a, b| a | b),
            TraceIr::BXorInt { lhs, rhs } => self.fold_int_bin(*lhs, *rhs, |a, b| a ^ b),
            TraceIr::ShlInt  { lhs, rhs } => self.fold_int_bin(*lhs, *rhs, |a, b| a.wrapping_shl(b as u32)),
            TraceIr::ShrInt  { lhs, rhs } => self.fold_int_bin(*lhs, *rhs, |a, b| a.wrapping_shr(b as u32)),
            // Unary
            TraceIr::NegInt { src } => {
                if let TraceIr::KInt(v) = &self.ops[src.index()] {
                    Some(TraceIr::KInt(v.wrapping_neg()))
                } else { None }
            }
            TraceIr::NegFloat { src } => {
                if let TraceIr::KFloat(v) = &self.ops[src.index()] {
                    Some(TraceIr::KFloat(-v))
                } else { None }
            }
            TraceIr::BNotInt { src } => {
                if let TraceIr::KInt(v) = &self.ops[src.index()] {
                    Some(TraceIr::KInt(!v))
                } else { None }
            }
            TraceIr::IntToFloat { src } => {
                if let TraceIr::KInt(v) = &self.ops[src.index()] {
                    Some(TraceIr::KFloat(*v as f64))
                } else { None }
            }
            _ => None,
        }
    }

    fn fold_int_bin(&self, lhs: TRef, rhs: TRef, f: impl FnOnce(i64, i64) -> i64) -> Option<TraceIr> {
        if let (TraceIr::KInt(a), TraceIr::KInt(b)) = (&self.ops[lhs.index()], &self.ops[rhs.index()]) {
            Some(TraceIr::KInt(f(*a, *b)))
        } else { None }
    }

    fn fold_float_bin(&self, lhs: TRef, rhs: TRef, f: impl FnOnce(f64, f64) -> f64) -> Option<TraceIr> {
        let a = match &self.ops[lhs.index()] {
            TraceIr::KFloat(v) => *v,
            TraceIr::KInt(v) => *v as f64,
            _ => return None,
        };
        let b = match &self.ops[rhs.index()] {
            TraceIr::KFloat(v) => *v,
            TraceIr::KInt(v) => *v as f64,
            _ => return None,
        };
        Some(TraceIr::KFloat(f(a, b)))
    }

    /// Take a snapshot of the current interpreter state for a side exit.
    fn snapshot(&mut self, pc: u32, base: usize) -> u32 {
        let snap_id = self.snapshots.len() as u32;
        let entries: Vec<SnapEntry> = self
            .slot_map
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                e.map(|(tref, ty)| SnapEntry {
                    slot: i as u16,
                    val: SnapValue::Ref(tref),
                    ty,
                })
            })
            .collect();
        self.snapshots.push(Snapshot {
            pc,
            base,
            depth: self.call_depth,
            entries,
        });
        snap_id
    }

    /// Ensure a stack slot has been loaded and type-guarded.
    /// Returns the `TRef` for the slot's value.
    fn ensure_slot(&mut self, slot: u16, ty: IrType, pc: u32, base: usize) -> TRef {
        if let Some((tref, existing_ty)) = self.slot_map.get(slot) {
            if existing_ty == ty {
                return tref;
            }
            // Type changed — need a new guard.
        }
        let snap_id = self.snapshot(pc, base);
        self.emit(TraceIr::GuardType {
            slot,
            expected: ty,
            snap_id,
        });
        let tref = self.emit(TraceIr::LoadSlot { slot });
        self.slot_map.set(slot, tref, ty);
        tref
    }

    /// Write a computed value to a stack slot in the slot map.
    /// No StoreSlot is emitted — writeback happens lazily at side exits
    /// (via snapshot entries) and at LoopEnd.
    fn write_slot(&mut self, slot: u16, tref: TRef, ty: IrType) {
        self.slot_map.set(slot, tref, ty);
    }

    /// Check abort conditions (trace too long, too many exits).
    fn check_limits(&self) -> Option<AbortReason> {
        if self.len >= MAX_TRACE_LEN {
            return Some(AbortReason::TooLong);
        }
        if self.snapshots.len() >= MAX_SNAPSHOTS {
            return Some(AbortReason::TooManyExits);
        }
        if self.call_depth > MAX_CALL_DEPTH {
            return Some(AbortReason::MaxCallDepth);
        }
        None
    }

    /// Detect the `IrType` from a live `LuaValue`'s type tag.
    fn detect_type(val: &LuaValue) -> IrType {
        if val.ttisinteger() {
            IrType::Int
        } else if val.ttisfloat() {
            IrType::Float
        } else if val.is_table() {
            IrType::Table
        } else if val.is_string() {
            IrType::String
        } else if val.is_boolean() {
            IrType::Bool
        } else if val.is_nil() {
            IrType::Nil
        } else {
            IrType::Function
        }
    }

    /// Try to detect an upvalue's current runtime type from the active closure.
    ///
    /// Recording runs before instruction dispatch, so destination registers for
    /// `GetUpval` still hold old values. Reading type from R[A] is incorrect.
    fn detect_upvalue_type(base: usize, upval_idx: u16, stack: &[LuaValue]) -> Option<IrType> {
        if base == 0 {
            return None;
        }
        let func = stack.get(base - 1)?;
        let lua_func = func.as_lua_function()?;
        let upval_ptr = lua_func.upvalues().get(upval_idx as usize)?;
        let upval_val = upval_ptr.as_ref().data.get_value_ref();
        Some(Self::detect_type(upval_val))
    }

    // ── Main entry point ──────────────────────────────────────────────

    /// Record one interpreter instruction.
    ///
    /// Called from the interpreter's dispatch loop after executing each
    /// instruction.  `pc` is the PC *before* dispatch (the instruction's
    /// own PC), `base` is the current stack base, `stack` is the full
    /// VM stack slice (for reading values / detecting types).
    ///
    /// Returns `RecordResult` telling the interpreter what to do next.
    pub fn record_instruction(
        &mut self,
        instr: Instruction,
        pc: u32,
        base: usize,
        stack: &[LuaValue],
    ) -> RecordResult {
        // Check if we've looped back to the trace head.
        if pc == self.head_pc && base == self.head_base && self.call_depth == 0 {
            self.head_visits += 1;
            if self.head_visits == 1 {
                // First visit — start of recording.  Record the prologue
                // (one full loop body) normally without LoopStart.
            } else if self.head_visits == 2 {
                // Second visit — prologue is done.  Emit LoopStart + Phi.
                self.loop_started = true;
                self.emit(TraceIr::LoopStart);
                // Emit Phi placeholders for all live slots and update slot_map
                // to point to Phi results.
                let live_slots: Vec<(u16, TRef, IrType)> = self.slot_map.entries
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| e.map(|(tref, ty)| (i as u16, tref, ty)))
                    .collect();
                for (slot, entry_tref, ty) in live_slots {
                    let ops_idx = self.ops.len();
                    let phi_tref = self.emit(TraceIr::Phi {
                        slot,
                        entry: entry_tref,
                        backedge: TRef::NONE, // patched at LoopEnd
                    });
                    self.phi_entries.push((ops_idx, slot, entry_tref));
                    self.slot_map.set(slot, phi_tref, ty);
                }
            } else {
                // Third visit — loop body recorded.  Patch Phi backedges.
                for (ops_idx, slot, _entry) in &self.phi_entries {
                    if let Some((backedge_tref, _ty)) = self.slot_map.get(*slot) {
                        if let TraceIr::Phi { backedge, .. } = &mut self.ops[*ops_idx] {
                            *backedge = backedge_tref;
                        }
                    }
                }
                // Take a final snapshot for safety-exit writeback.
                self.snapshot(pc, base);
                self.emit(TraceIr::LoopEnd);
                return RecordResult::LoopClosed;
            }
        }

        // Abort if limits exceeded.
        if let Some(reason) = self.check_limits() {
            return RecordResult::Abort(reason);
        }

        let op = instr.get_opcode();
        match op {
            // ── Data movement ─────────────────────────────────────────
            OpCode::Move => {
                self.record_move(instr, pc, base, stack)
            }
            OpCode::LoadI => {
                self.record_loadi(instr)
            }
            OpCode::LoadF => {
                self.record_loadf(instr)
            }
            OpCode::LoadK => {
                self.record_loadk(instr, pc, base, stack)
            }
            OpCode::LoadNil => {
                self.record_loadnil(instr)
            }
            OpCode::LoadTrue | OpCode::LoadFalse | OpCode::LFalseSkip => {
                self.record_loadbool(instr, op)
            }

            // ── Arithmetic: register-register ─────────────────────────
            OpCode::Add => self.record_arith_rr(instr, pc, base, stack),
            OpCode::Sub => self.record_arith_rr(instr, pc, base, stack),
            OpCode::Mul => self.record_arith_rr(instr, pc, base, stack),
            OpCode::Div => self.record_arith_rr(instr, pc, base, stack),
            OpCode::IDiv => self.record_arith_rr(instr, pc, base, stack),
            OpCode::Mod => self.record_arith_rr(instr, pc, base, stack),
            OpCode::Pow => self.record_arith_rr(instr, pc, base, stack),

            // ── Arithmetic: register-immediate ────────────────────────
            OpCode::AddI => self.record_arith_ri(instr, pc, base, stack),

            // ── Arithmetic: register-constant ─────────────────────────
            OpCode::AddK | OpCode::SubK | OpCode::MulK | OpCode::DivK
            | OpCode::IDivK | OpCode::ModK | OpCode::PowK => {
                self.record_arith_rk(instr, pc, base, stack)
            }

            // ── Bitwise ───────────────────────────────────────────────
            OpCode::BAnd | OpCode::BOr | OpCode::BXor
            | OpCode::Shl | OpCode::Shr => {
                self.record_bitwise_rr(instr, pc, base, stack)
            }
            OpCode::BAndK | OpCode::BOrK | OpCode::BXorK => {
                self.record_bitwise_rk(instr, pc, base, stack)
            }
            OpCode::ShlI | OpCode::ShrI => {
                self.record_shift_ri(instr, pc, base, stack)
            }

            // ── Unary ─────────────────────────────────────────────────
            OpCode::Unm => self.record_unm(instr, pc, base, stack),
            OpCode::BNot => self.record_bnot(instr, pc, base, stack),
            OpCode::Len => self.record_len(instr, pc, base, stack),
            OpCode::Not => self.record_not(instr, pc, base, stack),

            // ── Table access ──────────────────────────────────────────
            OpCode::GetTable => self.record_gettable(instr, pc, base, stack),
            OpCode::GetI => self.record_geti(instr, pc, base, stack),
            OpCode::GetField => self.record_getfield(instr, pc, base, stack),
            OpCode::SetTable => self.record_settable(instr, pc, base, stack),
            OpCode::SetI => self.record_seti(instr, pc, base, stack),
            OpCode::SetField => self.record_setfield(instr, pc, base, stack),
            OpCode::GetTabUp => self.record_gettabup(instr, pc, base, stack),

            // ── Upvalue access ────────────────────────────────────────
            OpCode::GetUpval => self.record_getupval(instr, pc, base, stack),
            OpCode::SetUpval => self.record_setupval(instr, pc, base, stack),

            // ── Comparisons ───────────────────────────────────────────
            OpCode::EqI | OpCode::LtI | OpCode::LeI
            | OpCode::GtI | OpCode::GeI => {
                self.record_cmp_imm(instr, pc, base, stack)
            }
            OpCode::Eq | OpCode::Lt | OpCode::Le => {
                self.record_cmp_rr(instr, pc, base, stack)
            }

            // ── Tests ─────────────────────────────────────────────────
            OpCode::Test => self.record_test(instr, pc, base, stack),
            OpCode::TestSet => self.record_testset(instr, pc, base, stack),

            // ── Jumps ─────────────────────────────────────────────────
            OpCode::Jmp => RecordResult::Continue, // no-op in trace
            OpCode::ForLoop => RecordResult::Continue, // handled at top
            OpCode::ForPrep => RecordResult::Continue, // no-op inside trace

            // ── Calls ─────────────────────────────────────────────────
            OpCode::Call => self.record_call(instr, pc, base, stack),
            OpCode::Return | OpCode::Return0 | OpCode::Return1 => {
                self.record_return(instr, pc, base, stack)
            }
            OpCode::TailCall => {
                RecordResult::Abort(AbortReason::NYI("tailcall"))
            }

            // ── Generic for ───────────────────────────────────────────
            OpCode::TForCall => self.record_tforcall(instr, pc, base, stack),
            OpCode::TForLoop => RecordResult::Continue,
            OpCode::TForPrep => RecordResult::Continue,

            // ── MmBin (metamethod fallback — skip, already handled) ───
            OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK => {
                RecordResult::Continue
            }

            // ── Everything else: abort ─────────────────────────────────
            _ => RecordResult::Abort(AbortReason::UnsupportedOp(op_name(op))),
        }
    }

    /// Finish recording and produce a `Trace`.
    pub fn finish(self) -> Trace {
        Trace {
            id: self.trace_id,
            ops: self.ops,
            snapshots: self.snapshots,
            chunk_ptr: self.chunk_ptr,
            head_pc: self.head_pc,
            head_base: self.head_base,
        }
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — data movement
    // ══════════════════════════════════════════════════════════════════

    fn record_move(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let val = &stack[base + b as usize];
        let ty = Self::detect_type(val);
        let src = self.ensure_slot(b, ty, pc, base);
        let r = self.emit(TraceIr::Move { src });
        self.write_slot(a, r, ty);
        RecordResult::Continue
    }

    fn record_loadi(&mut self, instr: Instruction) -> RecordResult {
        let a = instr.get_a() as u16;
        let sbx = instr.get_sbx() as i64;
        let r = self.emit(TraceIr::KInt(sbx));
        self.write_slot(a, r, IrType::Int);
        RecordResult::Continue
    }

    fn record_loadf(&mut self, instr: Instruction) -> RecordResult {
        let a = instr.get_a() as u16;
        let sbx = instr.get_sbx() as f64;
        let r = self.emit(TraceIr::KFloat(sbx));
        self.write_slot(a, r, IrType::Float);
        RecordResult::Continue
    }

    fn record_loadk(&mut self, instr: Instruction, _pc: u32, _base: usize, _stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        // Read the constant from the chunk's constant pool (NOT from the
        // stack, because the recorder runs BEFORE instruction dispatch).
        let bx = instr.get_bx() as usize;
        let chunk = unsafe { &*(self.chunk_ptr as *const crate::lua_value::Chunk) };
        let Some(kv) = chunk.constants.get(bx) else {
            return RecordResult::Abort(AbortReason::NYI("loadk const oob"));
        };
        let (r, ty) = if kv.ttisinteger() {
            let iv = unsafe { kv.value.i };
            (self.emit(TraceIr::KInt(iv)), IrType::Int)
        } else if kv.ttisfloat() {
            let fv = unsafe { kv.value.n };
            (self.emit(TraceIr::KFloat(fv)), IrType::Float)
        } else {
            return RecordResult::Abort(AbortReason::NYI("loadk non-numeric"));
        };
        self.write_slot(a, r, ty);
        RecordResult::Continue
    }

    fn record_loadnil(&mut self, instr: Instruction) -> RecordResult {
        // LoadNil sets R[A]..R[A+B] = nil. We just clear them in slot_map.
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        for slot in a..=a + b {
            let r = self.emit(TraceIr::KInt(0)); // placeholder
            self.write_slot(slot, r, IrType::Nil);
        }
        RecordResult::Continue
    }

    fn record_loadbool(&mut self, instr: Instruction, op: OpCode) -> RecordResult {
        let a = instr.get_a() as u16;
        let val = match op {
            OpCode::LoadTrue => 1i64,
            _ => 0i64, // LoadFalse, LFalseSkip
        };
        let r = self.emit(TraceIr::KInt(val));
        self.write_slot(a, r, IrType::Bool);
        RecordResult::Continue
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — arithmetic
    // ══════════════════════════════════════════════════════════════════

    /// Record R[A] = R[B] op R[C] for arithmetic opcodes.
    fn record_arith_rr(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let c = instr.get_c() as u16;
        let op = instr.get_opcode();

        let val_b = &stack[base + b as usize];
        let val_c = &stack[base + c as usize];
        let ty_b = Self::detect_type(val_b);
        let ty_c = Self::detect_type(val_c);

        let vb = self.ensure_slot(b, ty_b, pc, base);
        let vc = self.ensure_slot(c, ty_c, pc, base);

        // Determine result type and operands
        let (lhs, rhs, res_ty) = self.coerce_arith(vb, ty_b, vc, ty_c, op);

        let ir = match (op, res_ty) {
            (OpCode::Add, IrType::Int)   => TraceIr::AddInt   { lhs, rhs },
            (OpCode::Add, IrType::Float) => TraceIr::AddFloat { lhs, rhs },
            (OpCode::Sub, IrType::Int)   => TraceIr::SubInt   { lhs, rhs },
            (OpCode::Sub, IrType::Float) => TraceIr::SubFloat { lhs, rhs },
            (OpCode::Mul, IrType::Int)   => TraceIr::MulInt   { lhs, rhs },
            (OpCode::Mul, IrType::Float) => TraceIr::MulFloat { lhs, rhs },
            (OpCode::Div, _)             => TraceIr::DivFloat { lhs, rhs }, // always float
            (OpCode::IDiv, IrType::Int)  => TraceIr::IDivInt  { lhs, rhs },
            (OpCode::Mod, IrType::Int)   => TraceIr::ModInt   { lhs, rhs },
            (OpCode::Pow, _)             => TraceIr::PowFloat { lhs, rhs }, // always float
            _ => return RecordResult::Abort(AbortReason::NYI("arith combo")),
        };
        let r = self.fold_or_emit(ir);
        self.write_slot(a, r, res_ty);
        RecordResult::Continue
    }

    /// Record R[A] = R[B] + sC (AddI).
    fn record_arith_ri(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let sc = instr.get_sc() as i64;

        let val_b = &stack[base + b as usize];
        let ty_b = Self::detect_type(val_b);
        let vb = self.ensure_slot(b, ty_b, pc, base);

        let (res, res_ty) = if ty_b == IrType::Int {
            let kc = self.emit(TraceIr::KInt(sc));
            (self.emit(TraceIr::AddInt { lhs: vb, rhs: kc }), IrType::Int)
        } else {
            let fb = self.coerce_one(vb, ty_b);
            let kc = self.emit(TraceIr::KFloat(sc as f64));
            (self.emit(TraceIr::AddFloat { lhs: fb, rhs: kc }), IrType::Float)
        };
        self.write_slot(a, res, res_ty);
        RecordResult::Continue
    }

    /// Record R[A] = R[B] op K[C] for *K opcodes.
    fn record_arith_rk(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let c = instr.get_c() as usize;
        let op = instr.get_opcode();

        let val_b = &stack[base + b as usize];
        let ty_b = Self::detect_type(val_b);
        let vb = self.ensure_slot(b, ty_b, pc, base);

        // Read the constant directly from the chunk's constant pool.
        let chunk = unsafe { &*(self.chunk_ptr as *const crate::lua_value::Chunk) };
        let Some(kv) = chunk.constants.get(c) else {
            return RecordResult::Abort(AbortReason::NYI("rk const oob"));
        };

        // Determine constant type from the constant pool value.
        let (kval, ty_k) = if kv.ttisinteger() {
            let ki = unsafe { kv.value.i };
            (self.emit(TraceIr::KInt(ki)), IrType::Int)
        } else if kv.ttisfloat() {
            let kf = unsafe { kv.value.n };
            (self.emit(TraceIr::KFloat(kf)), IrType::Float)
        } else {
            return RecordResult::Abort(AbortReason::NYI("rk non-numeric const"));
        };

        // Map *K opcodes to their base opcodes for coerce_arith.
        let base_op = match op {
            OpCode::AddK => OpCode::Add,
            OpCode::SubK => OpCode::Sub,
            OpCode::MulK => OpCode::Mul,
            OpCode::DivK => OpCode::Div,
            OpCode::IDivK => OpCode::IDiv,
            OpCode::ModK => OpCode::Mod,
            OpCode::PowK => OpCode::Pow,
            _ => return RecordResult::Abort(AbortReason::NYI("rk combo")),
        };

        let (lhs, rhs, res_ty) = self.coerce_arith(vb, ty_b, kval, ty_k, base_op);

        let ir = match (op, res_ty) {
            (OpCode::AddK, IrType::Int)   => TraceIr::AddInt   { lhs, rhs },
            (OpCode::AddK, IrType::Float) => TraceIr::AddFloat { lhs, rhs },
            (OpCode::SubK, IrType::Int)   => TraceIr::SubInt   { lhs, rhs },
            (OpCode::SubK, IrType::Float) => TraceIr::SubFloat { lhs, rhs },
            (OpCode::MulK, IrType::Int)   => TraceIr::MulInt   { lhs, rhs },
            (OpCode::MulK, IrType::Float) => TraceIr::MulFloat { lhs, rhs },
            (OpCode::DivK, _)             => TraceIr::DivFloat { lhs, rhs },
            (OpCode::IDivK, IrType::Int)  => TraceIr::IDivInt  { lhs, rhs },
            (OpCode::ModK, IrType::Int)   => TraceIr::ModInt   { lhs, rhs },
            (OpCode::PowK, _)             => TraceIr::PowFloat { lhs, rhs },
            _ => return RecordResult::Abort(AbortReason::NYI("rk combo")),
        };
        let r = self.fold_or_emit(ir);
        self.write_slot(a, r, res_ty);
        RecordResult::Continue
    }

    // ── Coercion helpers ──────────────────────────────────────────────

    /// Coerce one value to float if needed.
    fn coerce_one(&mut self, val: TRef, ty: IrType) -> TRef {
        if ty == IrType::Int {
            self.fold_or_emit(TraceIr::IntToFloat { src: val })
        } else {
            val
        }
    }

    /// Determine result type and coerce operands for binary arithmetic.
    fn coerce_arith(&mut self, lhs: TRef, lt: IrType, rhs: TRef, rt: IrType, op: OpCode) -> (TRef, TRef, IrType) {
        // Div and Pow always produce float
        if op == OpCode::Div || op == OpCode::Pow {
            let fl = self.coerce_one(lhs, lt);
            let fr = self.coerce_one(rhs, rt);
            return (fl, fr, IrType::Float);
        }
        // If both int → int result
        if lt == IrType::Int && rt == IrType::Int {
            return (lhs, rhs, IrType::Int);
        }
        // Otherwise → float
        let fl = self.coerce_one(lhs, lt);
        let fr = self.coerce_one(rhs, rt);
        (fl, fr, IrType::Float)
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — bitwise & shifts
    // ══════════════════════════════════════════════════════════════════

    fn record_bitwise_rr(&mut self, instr: Instruction, pc: u32, base: usize, _stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let c = instr.get_c() as u16;
        let op = instr.get_opcode();
        let vb = self.ensure_slot(b, IrType::Int, pc, base);
        let vc = self.ensure_slot(c, IrType::Int, pc, base);
        let ir = match op {
            OpCode::BAnd => TraceIr::BAndInt { lhs: vb, rhs: vc },
            OpCode::BOr  => TraceIr::BOrInt  { lhs: vb, rhs: vc },
            OpCode::BXor => TraceIr::BXorInt { lhs: vb, rhs: vc },
            OpCode::Shl  => TraceIr::ShlInt  { lhs: vb, rhs: vc },
            OpCode::Shr  => TraceIr::ShrInt  { lhs: vb, rhs: vc },
            _ => unreachable!(),
        };
        let r = self.fold_or_emit(ir);
        self.write_slot(a, r, IrType::Int);
        RecordResult::Continue
    }

    fn record_bitwise_rk(&mut self, instr: Instruction, pc: u32, base: usize, _stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let c = instr.get_c() as usize;
        let op = instr.get_opcode();
        let vb = self.ensure_slot(b, IrType::Int, pc, base);

        // Read the constant directly from the chunk's constant pool.
        let chunk = unsafe { &*(self.chunk_ptr as *const crate::lua_value::Chunk) };
        let Some(kv) = chunk.constants.get(c) else {
            return RecordResult::Abort(AbortReason::NYI("bitwise_rk const oob"));
        };
        let ki = if kv.ttisinteger() {
            unsafe { kv.value.i }
        } else {
            return RecordResult::Abort(AbortReason::NYI("bitwise_rk non-int const"));
        };
        let kval = self.emit(TraceIr::KInt(ki));

        let ir = match op {
            OpCode::BAndK => TraceIr::BAndInt { lhs: vb, rhs: kval },
            OpCode::BOrK  => TraceIr::BOrInt  { lhs: vb, rhs: kval },
            OpCode::BXorK => TraceIr::BXorInt { lhs: vb, rhs: kval },
            _ => unreachable!(),
        };
        let r = self.fold_or_emit(ir);
        self.write_slot(a, r, IrType::Int);
        RecordResult::Continue
    }

    fn record_shift_ri(&mut self, instr: Instruction, pc: u32, base: usize, _stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let sc = instr.get_sc() as i64;
        let op = instr.get_opcode();
        let vb = self.ensure_slot(b, IrType::Int, pc, base);
        let kc = self.emit(TraceIr::KInt(sc));
        let ir = match op {
            OpCode::ShlI => TraceIr::ShlInt { lhs: kc, rhs: vb }, // ShlI: sC << R[B]
            OpCode::ShrI => TraceIr::ShrInt { lhs: vb, rhs: kc }, // ShrI: R[B] >> sC
            _ => unreachable!(),
        };
        let r = self.fold_or_emit(ir);
        self.write_slot(a, r, IrType::Int);
        RecordResult::Continue
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — unary ops
    // ══════════════════════════════════════════════════════════════════

    fn record_unm(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let val = &stack[base + b as usize];
        let ty = Self::detect_type(val);
        let vb = self.ensure_slot(b, ty, pc, base);
        let (ir, res_ty) = match ty {
            IrType::Int   => (TraceIr::NegInt   { src: vb }, IrType::Int),
            IrType::Float => (TraceIr::NegFloat { src: vb }, IrType::Float),
            _ => return RecordResult::Abort(AbortReason::NYI("unm non-numeric")),
        };
        let r = self.fold_or_emit(ir);
        self.write_slot(a, r, res_ty);
        RecordResult::Continue
    }

    fn record_bnot(&mut self, instr: Instruction, pc: u32, base: usize, _stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let vb = self.ensure_slot(b, IrType::Int, pc, base);
        let r = self.fold_or_emit(TraceIr::BNotInt { src: vb });
        self.write_slot(a, r, IrType::Int);
        RecordResult::Continue
    }

    fn record_len(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let val = &stack[base + b as usize];
        if !val.is_table() {
            return RecordResult::Abort(AbortReason::NYI("len non-table"));
        }
        let vb = self.ensure_slot(b, IrType::Table, pc, base);
        let r = self.emit(TraceIr::TabLen { table: vb });
        self.write_slot(a, r, IrType::Int);
        RecordResult::Continue
    }

    fn record_not(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        // not x: result is always bool. We record the truthiness test.
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let val = &stack[base + b as usize];
        let ty = Self::detect_type(val);
        let vb = self.ensure_slot(b, ty, pc, base);
        // `not` just produces a boolean. Derive from the INPUT value
        // (register B, which is valid pre-execution).
        let rv = if val.is_truthy() { 0i64 } else { 1i64 }; // not(truthy)=false, not(falsy)=true
        let r = self.emit(TraceIr::KInt(rv));
        self.write_slot(a, r, IrType::Bool);
        let _ = vb; // guard ensures consistent type
        RecordResult::Continue
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — table access
    // ══════════════════════════════════════════════════════════════════

    fn record_gettable(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let c = instr.get_c() as u16;
        let vt = self.ensure_slot(b, IrType::Table, pc, base);
        let vk = self.ensure_slot(c, IrType::Int, pc, base);
        let r = self.emit(TraceIr::TabGetI { table: vt, index: vk });
        let result = &stack[base + a as usize];
        let res_ty = Self::detect_type(result);
        self.write_slot(a, r, res_ty);
        RecordResult::Continue
    }

    fn record_geti(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let c = instr.get_c() as i64; // immediate integer key
        let vt = self.ensure_slot(b, IrType::Table, pc, base);
        let vk = self.emit(TraceIr::KInt(c));
        let r = self.emit(TraceIr::TabGetI { table: vt, index: vk });
        let result = &stack[base + a as usize];
        let res_ty = Self::detect_type(result);
        self.write_slot(a, r, res_ty);
        RecordResult::Continue
    }

    fn record_getfield(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        // C indexes into constant table — the key is a string.
        // We store the raw pointer to the interned string key.
        let vt = self.ensure_slot(b, IrType::Table, pc, base);
        // For now, read the result type from the stack.
        let result = &stack[base + a as usize];
        let res_ty = Self::detect_type(result);
        // key_ptr: we pass 0 as placeholder — the compiler will read from bytecode.
        let r = self.emit(TraceIr::TabGetS { table: vt, key_ptr: 0 });
        self.write_slot(a, r, res_ty);
        RecordResult::Continue
    }

    fn record_settable(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let c = instr.get_c() as u16;
        let vt = self.ensure_slot(a, IrType::Table, pc, base);
        let vk = self.ensure_slot(b, IrType::Int, pc, base);
        let val = &stack[base + c as usize];
        let ty = Self::detect_type(val);
        let vc = self.ensure_slot(c, ty, pc, base);
        self.emit(TraceIr::TabSetI { table: vt, index: vk, val: vc });
        RecordResult::Continue
    }

    fn record_seti(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as i64;
        let c = instr.get_c() as u16;
        let vt = self.ensure_slot(a, IrType::Table, pc, base);
        let vk = self.emit(TraceIr::KInt(b));
        let val = &stack[base + c as usize];
        let ty = Self::detect_type(val);
        let vc = self.ensure_slot(c, ty, pc, base);
        self.emit(TraceIr::TabSetI { table: vt, index: vk, val: vc });
        RecordResult::Continue
    }

    fn record_setfield(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let c = instr.get_c() as u16;
        let vt = self.ensure_slot(a, IrType::Table, pc, base);
        let val = &stack[base + c as usize];
        let ty = Self::detect_type(val);
        let vc = self.ensure_slot(c, ty, pc, base);
        self.emit(TraceIr::TabSetS { table: vt, key_ptr: 0, val: vc });
        RecordResult::Continue
    }

    fn record_gettabup(&mut self, instr: Instruction, _pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        // GetTabUp: R[A] = UpValue[B][K[C]]
        // Load the upvalue (which should be a table — typically _ENV).
        let uv = self.emit(TraceIr::LoadUpval { upval_idx: b });
        // The key K[C] is a string constant — treat as field access.
        let result = &stack[base + a as usize];
        let res_ty = Self::detect_type(result);
        let r = self.emit(TraceIr::TabGetS { table: uv, key_ptr: 0 });
        self.write_slot(a, r, res_ty);
        RecordResult::Continue
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — upvalue access
    // ══════════════════════════════════════════════════════════════════

    fn record_getupval(&mut self, instr: Instruction, _pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let r = self.emit(TraceIr::LoadUpval { upval_idx: b });
        let res_ty = Self::detect_upvalue_type(base, b, stack)
            .unwrap_or_else(|| Self::detect_type(&stack[base + a as usize]));
        self.write_slot(a, r, res_ty);
        RecordResult::Continue
    }

    fn record_setupval(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let val = &stack[base + a as usize];
        let ty = Self::detect_type(val);
        let va = self.ensure_slot(a, ty, pc, base);
        self.emit(TraceIr::StoreUpval { upval_idx: b, val: va, ty });
        RecordResult::Continue
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — comparisons
    // ══════════════════════════════════════════════════════════════════

    fn record_cmp_imm(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        // EqI / LtI / LeI / GtI / GeI — compare R[A] with sB (immediate)
        let a = instr.get_a() as u16;
        let op = instr.get_opcode();
        let val_a = &stack[base + a as usize];
        let ty = Self::detect_type(val_a);
        let va = self.ensure_slot(a, ty, pc, base);
        let sb = instr.get_sb() as i64;

        // Determine the observed comparison result at recording time
        let cond = if ty == IrType::Float {
            let fval = unsafe { val_a.value.n };
            let fsb = sb as f64;
            match op {
                OpCode::EqI => fval == fsb,
                OpCode::LtI => fval < fsb,
                OpCode::LeI => fval <= fsb,
                OpCode::GtI => fval > fsb,
                OpCode::GeI => fval >= fsb,
                _ => unreachable!(),
            }
        } else {
            match op {
                OpCode::EqI => val_a.ivalue() == sb,
                OpCode::LtI => val_a.ivalue() < sb,
                OpCode::LeI => val_a.ivalue() <= sb,
                OpCode::GtI => val_a.ivalue() > sb,
                OpCode::GeI => val_a.ivalue() >= sb,
                _ => unreachable!(),
            }
        };
        // Guard the comparison matches the observed result
        let cmp = match (op, cond) {
            (OpCode::LtI, true) => CmpOp::Lt,
            (OpCode::LtI, false) => CmpOp::Ge,
            (OpCode::LeI, true) => CmpOp::Le,
            (OpCode::LeI, false) => CmpOp::Gt,
            (OpCode::GtI, true) => CmpOp::Gt,
            (OpCode::GtI, false) => CmpOp::Le,
            (OpCode::GeI, true) => CmpOp::Ge,
            (OpCode::GeI, false) => CmpOp::Lt,
            (OpCode::EqI, true) => CmpOp::Eq,
            (OpCode::EqI, false) => CmpOp::Ne,
            _ => unreachable!(),
        };
        self.snapshot(pc, base);
        let snap_id = self.snapshots.len() as u32 - 1;
        if ty == IrType::Float {
            let rhs = self.emit(TraceIr::KFloat(sb as f64));
            self.emit(TraceIr::GuardCmpF {
                lhs: va,
                rhs,
                cmp,
                snap_id,
            });
        } else {
            self.emit(TraceIr::GuardCmpI {
                lhs: va,
                rhs_imm: sb,
                cmp,
                snap_id,
            });
        }
        RecordResult::Continue
    }

    fn record_cmp_rr(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        // Eq / Lt / Le — compare R[A] with R[B]
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let val_a = &stack[base + a as usize];
        let val_b = &stack[base + b as usize];
        let ty_a = Self::detect_type(val_a);
        let ty_b = Self::detect_type(val_b);
        let va = self.ensure_slot(a, ty_a, pc, base);
        let vb = self.ensure_slot(b, ty_b, pc, base);

        let op = instr.get_opcode();
        let is_float = ty_a == IrType::Float || ty_b == IrType::Float;
        let cond = if is_float {
            let fa = if ty_a == IrType::Float { unsafe { val_a.value.n } } else { val_a.ivalue() as f64 };
            let fb = if ty_b == IrType::Float { unsafe { val_b.value.n } } else { val_b.ivalue() as f64 };
            match op {
                OpCode::Eq => fa == fb,
                OpCode::Lt => fa < fb,
                OpCode::Le => fa <= fb,
                _ => unreachable!(),
            }
        } else {
            match op {
                OpCode::Eq => val_a.ivalue() == val_b.ivalue(),
                OpCode::Lt => val_a.ivalue() < val_b.ivalue(),
                OpCode::Le => val_a.ivalue() <= val_b.ivalue(),
                _ => unreachable!(),
            }
        };
        let cmp = match (op, cond) {
            (OpCode::Lt, true) => CmpOp::Lt,
            (OpCode::Lt, false) => CmpOp::Ge,
            (OpCode::Le, true) => CmpOp::Le,
            (OpCode::Le, false) => CmpOp::Gt,
            (OpCode::Eq, true) => CmpOp::Eq,
            (OpCode::Eq, false) => CmpOp::Ne,
            _ => unreachable!(),
        };
        self.snapshot(pc, base);
        let snap_id = self.snapshots.len() as u32 - 1;
        if is_float {
            let fa = self.coerce_one(va, ty_a);
            let fb = self.coerce_one(vb, ty_b);
            self.emit(TraceIr::GuardCmpF {
                lhs: fa,
                rhs: fb,
                cmp,
                snap_id,
            });
        } else {
            self.emit(TraceIr::GuardCmpRR {
                lhs: va,
                rhs: vb,
                cmp,
                snap_id,
            });
        }
        RecordResult::Continue
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — tests
    // ══════════════════════════════════════════════════════════════════

    fn record_test(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let val_a = &stack[base + a as usize];
        let ty = Self::detect_type(val_a);
        let va = self.ensure_slot(a, ty, pc, base);

        self.snapshot(pc, base);
        let snap_id = self.snapshots.len() as u32 - 1;

        let k = instr.get_k();
        self.emit(TraceIr::GuardTruthy {
            val: va,
            expected: !k,
            snap_id,
        });
        RecordResult::Continue
    }

    fn record_testset(&mut self, instr: Instruction, pc: u32, base: usize, stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as u16;
        let b = instr.get_b() as u16;
        let val_b = &stack[base + b as usize];
        let ty = Self::detect_type(val_b);
        let vb = self.ensure_slot(b, ty, pc, base);

        self.snapshot(pc, base);
        let snap_id = self.snapshots.len() as u32 - 1;

        let k = instr.get_k();
        self.emit(TraceIr::GuardTruthy {
            val: vb,
            expected: !k,
            snap_id,
        });

        // If the guard succeeds, R[A] = R[B]
        let r = self.emit(TraceIr::Move { src: vb });
        self.write_slot(a, r, ty);
        RecordResult::Continue
    }

    // ══════════════════════════════════════════════════════════════════
    // record_* methods — calls & returns
    // ══════════════════════════════════════════════════════════════════

    fn record_call(&mut self, instr: Instruction, pc: u32, base: usize, _stack: &[LuaValue]) -> RecordResult {
        let a = instr.get_a() as usize;
        let b = instr.get_b() as u8;
        let c = instr.get_c() as i8;

        // For now we only record generic calls as opaque operations.
        // Future: detect builtins (math.sqrt etc.) and inline them.
        //
        // Increment call_depth so we know we're inside a sub-frame.
        self.call_depth += 1;
        if self.call_depth > MAX_CALL_DEPTH {
            return RecordResult::Abort(AbortReason::MaxCallDepth);
        }

        self.snapshot(pc, base);
        self.emit(TraceIr::CallGeneric {
            func_slot: a as u16,
            nargs: b,
            nresults: c,
        });
        RecordResult::Continue
    }

    fn record_return(&mut self, _instr: Instruction, _pc: u32, _base: usize, _stack: &[LuaValue]) -> RecordResult {
        if self.call_depth == 0 {
            // Returning from the trace's root frame → abort (trace ends
            // at the loop back-edge, not a return).
            return RecordResult::Abort(AbortReason::NYI("return from root frame"));
        }
        self.call_depth -= 1;
        // Return doesn't emit IR — the call site handles results.
        RecordResult::Continue
    }

    fn record_tforcall(&mut self, _instr: Instruction, _pc: u32, _base: usize, _stack: &[LuaValue]) -> RecordResult {
        // TForCall calls the iterator function.  For now we treat
        // as NYI unless we inline it in the future.
        RecordResult::Abort(AbortReason::NYI("tforcall"))
    }
}

// ── Op name helper (for abort messages) ───────────────────────────────────────

fn op_name(op: OpCode) -> &'static str {
    match op {
        OpCode::Concat => "concat",
        OpCode::Close => "close",
        OpCode::Tbc => "tbc",
        OpCode::NewTable => "newtable",
        OpCode::Self_ => "self",
        OpCode::SetTabUp => "settabup",
        OpCode::Closure => "closure",
        OpCode::Vararg | OpCode::GetVarg => "vararg",
        OpCode::SetList => "setlist",
        OpCode::EqK => "eqk",
        OpCode::ErrNNil => "errnil",
        OpCode::VarargPrep => "varargprep",
        OpCode::LoadKX => "loadkx",
        _ => "unknown",
    }
}

