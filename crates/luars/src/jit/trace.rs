//! Trace IR definitions for the tracing JIT compiler.
//!
//! A **trace** is a linear sequence of IR instructions recorded while the
//! interpreter executes a hot loop.  The trace captures one specific
//! execution path through the code, specialised on the types actually
//! observed at recording time.
//!
//! # Design
//!
//! Inspired by LuaJIT's trace IR, adapted for Lua 5.5 and Cranelift:
//!
//! * Linear IR — each instruction has a unique index used as a value
//!   reference (`TRef`).  Later instructions refer to earlier results.
//! * Side exits via **guards** — every type assumption and comparison is
//!   backed by a guard that branches to a *side exit* on failure.  Side
//!   exits are numbered; each has an associated `Snapshot` that tells the
//!   runtime how to rebuild VM state and resume the interpreter.
//! * Loop structure — a trace always represents a single loop iteration.
//!   `LoopStart` marks where the loop body begins (after the "lead-in"
//!   prologue).  `LoopEnd` marks the backedge.  Values that flow across
//!   the backedge get `Phi` nodes at `LoopStart`.

// ── Value references ──────────────────────────────────────────────────────────

/// Reference to a trace IR instruction result.
///
/// The index is into `Trace::ops`.  Constants and `LoadSlot` instructions
/// are referenced just like any other instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TRef(pub u32);

impl TRef {
    pub const NONE: TRef = TRef(u32::MAX);
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

// ── Type tags ─────────────────────────────────────────────────────────────────

/// Concrete type observed during trace recording.
///
/// These correspond to Lua's `tt` byte values and are used inside guards
/// to specialise the compiled code for a single type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrType {
    Int,      // LUA_VNUMINT  = 0x03
    Float,    // LUA_VNUMFLT  = 0x13
    Table,    // LUA_VTABLE   = 0x45
    String,   // LUA_VSHRSTR  = 0x44 or LUA_VLNGSTR = 0x54
    Bool,     // LUA_VTRUE=0x11 or LUA_VFALSE=0x01
    Nil,      // LUA_VNIL     = 0x00
    Function, // LUA_VLCL=0x66 / LUA_VCCL=0x46 / LUA_VLCF=0x16
}

// ── Comparison ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

// ── Builtin functions ─────────────────────────────────────────────────────────

/// Builtin functions that the JIT can compile to direct machine instructions
/// or libm calls, avoiding the full Lua function-call overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinFn {
    // math.*
    MathSqrt,
    MathAbs,
    MathFloor,
    MathCeil,
    MathSin,
    MathCos,
    MathExp,
    MathLog,
    MathMax2,
    MathMin2,
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

/// Snapshot of interpreter state at a guard point.
///
/// When a guard fails (side exit), the runtime uses the snapshot to
/// reconstruct the VM state and resume the interpreter at the correct PC.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Program counter to resume at.
    pub pc: u32,
    /// Stack base at snapshot time.
    pub base: usize,
    /// Call depth at snapshot time (relative to trace entry).
    pub depth: u32,
    /// Which stack slots need to be written back from trace values.
    pub entries: Vec<SnapEntry>,
}

/// One entry in a snapshot — maps a stack slot to its value source.
#[derive(Clone, Copy, Debug)]
pub struct SnapEntry {
    /// Stack slot index (relative to base).
    pub slot: u16,
    /// Where the value comes from.
    pub val: SnapValue,
}

/// How a snapshot slot gets its value.
#[derive(Clone, Copy, Debug)]
pub enum SnapValue {
    /// Value is the result of trace instruction `TRef`.
    Ref(TRef),
    /// Value was not modified by the trace — keep the current VM value.
    Inherit,
}

// ── Trace IR instructions ─────────────────────────────────────────────────────

/// A single trace IR instruction.
///
/// The instruction set is intentionally small.  Each instruction maps
/// closely to one or two Cranelift IR nodes.  Complex Lua operations
/// (e.g. generic metamethod dispatch) are not represented — the trace
/// aborts if it encounters them.
#[derive(Clone, Debug)]
pub enum TraceIr {
    // ── Guards ─────────────────────────────────────────────────────
    /// Assert that a stack slot has the expected type.
    /// On failure → side exit `snap_id`.
    GuardType {
        slot: u16,
        expected: IrType,
        snap_id: u32,
    },

    /// Assert that a table has no metamethod for an operation.
    /// On failure → side exit.
    GuardNoMetamethod {
        table: TRef,
        event: u8, // TmKind as u8
        snap_id: u32,
    },

    /// Guard that a value is truthy (not nil, not false).
    /// `expected = true` means "guard that it IS truthy".
    GuardTruthy {
        val: TRef,
        expected: bool,
        snap_id: u32,
    },

    /// Guard an integer comparison R[lhs] <cmp> immediate.
    GuardCmpI {
        lhs: TRef,
        rhs_imm: i64,
        cmp: CmpOp,
        snap_id: u32,
    },

    /// Guard a register-register comparison R[lhs] <cmp> R[rhs].
    GuardCmpRR {
        lhs: TRef,
        rhs: TRef,
        cmp: CmpOp,
        snap_id: u32,
    },

    // ── Constants ──────────────────────────────────────────────────
    /// Integer constant.
    KInt(i64),

    /// Float constant.
    KFloat(f64),

    // ── Stack access ───────────────────────────────────────────────
    /// Load the value payload from a VM stack slot.
    /// The type is determined by the preceding `GuardType`.
    LoadSlot { slot: u16 },

    /// Write a value back to a VM stack slot (with correct type tag).
    StoreSlot { slot: u16, val: TRef, ty: IrType },

    // ── Upvalue access ─────────────────────────────────────────────
    /// Load a value from an upvalue.
    LoadUpval { upval_idx: u16 },

    /// Store a value to an upvalue.
    StoreUpval { upval_idx: u16, val: TRef, ty: IrType },

    // ── Integer arithmetic ─────────────────────────────────────────
    AddInt { lhs: TRef, rhs: TRef },
    SubInt { lhs: TRef, rhs: TRef },
    MulInt { lhs: TRef, rhs: TRef },
    IDivInt { lhs: TRef, rhs: TRef },
    ModInt { lhs: TRef, rhs: TRef },
    NegInt { src: TRef },

    // ── Float arithmetic ───────────────────────────────────────────
    AddFloat { lhs: TRef, rhs: TRef },
    SubFloat { lhs: TRef, rhs: TRef },
    MulFloat { lhs: TRef, rhs: TRef },
    DivFloat { lhs: TRef, rhs: TRef },
    PowFloat { lhs: TRef, rhs: TRef },
    NegFloat { src: TRef },

    /// Coerce integer to float.
    IntToFloat { src: TRef },

    // ── Bitwise ────────────────────────────────────────────────────
    BAndInt { lhs: TRef, rhs: TRef },
    BOrInt { lhs: TRef, rhs: TRef },
    BXorInt { lhs: TRef, rhs: TRef },
    BNotInt { src: TRef },
    ShlInt { lhs: TRef, rhs: TRef },
    ShrInt { lhs: TRef, rhs: TRef },

    // ── Table operations ───────────────────────────────────────────
    /// Array read:  `t[idx]` where idx is a positive integer.
    TabGetI { table: TRef, index: TRef },

    /// Array write: `t[idx] = val`
    TabSetI { table: TRef, index: TRef, val: TRef },

    /// Field read by interned string key: `t.name`
    TabGetS { table: TRef, key_ptr: usize },

    /// Field write: `t.name = val`
    TabSetS { table: TRef, key_ptr: usize, val: TRef },

    /// Table length: `#t`
    TabLen { table: TRef },

    // ── Function calls ─────────────────────────────────────────────
    /// Call a recognised builtin (math.sqrt, etc.).
    /// The function identity was guarded at recording time.
    CallBuiltin { func: BuiltinFn, arg: TRef },

    /// Generic call — record but abort for now (NYI).
    /// Placeholder for future function inlining.
    CallGeneric { func_slot: u16, nargs: u8, nresults: i8 },

    // ── Data movement ──────────────────────────────────────────────
    /// Copy one trace value (used for register moves).
    Move { src: TRef },

    /// Concatenate values (NYI — currently aborts recording).
    Concat { base: u16, count: u16 },

    // ── Loop structure ─────────────────────────────────────────────
    /// Marks the beginning of the loop body.
    /// All `Phi` nodes must immediately follow this marker.
    LoopStart,

    /// Phi node: merges the `entry` value (from before the loop) with
    /// the `backedge` value (from the end of the previous iteration).
    Phi { slot: u16, entry: TRef, backedge: TRef },

    /// End of trace — unconditional jump back to `LoopStart`.
    LoopEnd,
}

// ── Complete trace ────────────────────────────────────────────────────────────

/// A fully-recorded trace, ready for compilation.
pub struct Trace {
    /// Unique trace ID (monotonically increasing).
    pub id: u32,
    /// The IR instruction sequence.
    pub ops: Vec<TraceIr>,
    /// Snapshots for each side exit, indexed by `snap_id`.
    pub snapshots: Vec<Snapshot>,
    /// Raw pointer to the Chunk where the trace head lives.
    pub chunk_ptr: *const u8,
    /// Bytecode PC of the trace head (the backward-jump target).
    pub head_pc: u32,
    /// Stack base offset at trace start.
    pub head_base: usize,
}

// Safety: Trace is only used within a single thread (the VM thread)
// and chunk_ptr validity is guaranteed by the Rc<Chunk> lifetime.
unsafe impl Send for Trace {}
unsafe impl Sync for Trace {}

// ── Recording state ───────────────────────────────────────────────────────────

/// Current state of the trace recorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordState {
    /// Not recording — interpreter runs at full speed.
    Idle,
    /// Currently recording a trace.
    Recording,
}

/// Result of processing one interpreter instruction during recording.
#[derive(Debug)]
pub enum RecordResult {
    /// Continue recording the next instruction.
    Continue,
    /// The trace has looped back to its head — ready to compile.
    LoopClosed,
    /// Recording was aborted.
    Abort(AbortReason),
}

/// Why a recording was aborted.
#[derive(Debug)]
pub enum AbortReason {
    /// Max trace length exceeded.
    TooLong,
    /// Unsupported opcode encountered.
    UnsupportedOp(&'static str),
    /// Too many side exits / snapshots.
    TooManyExits,
    /// Exceeded max inlined call depth.
    MaxCallDepth,
    /// Feature not yet implemented.
    NYI(&'static str),
    /// Blacklisted trace head (failed too many times).
    Blacklisted,
}
