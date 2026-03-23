use crate::{
    OpCode,
    lua_vm::{TmKind, jit::TraceAbortReason},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceAnchorKind {
    LoopBackedge,
    ForLoop,
}

#[derive(Debug, Clone, Copy)]
pub struct RecordingRequest {
    pub chunk_key: usize,
    pub anchor_pc: usize,
    pub current_pc: usize,
    pub base: usize,
    pub frame_depth: usize,
    pub anchor_kind: TraceAnchorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingResult {
    Compiled(TraceId),
    Abort(TraceAbortReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSnapshotKind {
    Entry,
    SideExit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSnapshot {
    pub kind: TraceSnapshotKind,
    pub pc: usize,
    pub resume_pc: usize,
    pub base: usize,
    pub frame_depth: usize,
    pub live_regs: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceFallback {
    MmBin {
        tm: TmKind,
    },
    MmBinI {
        imm: i64,
        tm: TmKind,
        flip: bool,
    },
    MmBinK {
        constant_index: usize,
        tm: TmKind,
        flip: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceInstruction {
    pub pc: usize,
    pub opcode: OpCode,
    pub line: Option<u32>,
    pub fallback: Option<TraceFallback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceGuardMode {
    Precondition,
    Control,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceGuardKind {
    Eq,
    Lt,
    Le,
    Truthy,
    Falsey,
    IsNumber,
    IsIntegerLike,
    IsComparableLtLe,
    IsEqSafeComparable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceGuardOperands {
    Register { reg: u8 },
    Registers { lhs: u8, rhs: u8 },
    RegisterImmediate { reg: u8, imm: i64 },
    ImmediateRegister { imm: i64, reg: u8 },
    RegisterConstant { reg: u8, constant_index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceGuard {
    pub pc: usize,
    pub mode: TraceGuardMode,
    pub kind: TraceGuardKind,
    pub operands: TraceGuardOperands,
    pub continue_when: bool,
    pub exit_snapshot_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceExitKind {
    LoopExit,
    GuardExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceExitAction {
    CopyReg { dst: u8, src: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceExit {
    pub kind: TraceExitKind,
    pub source_pc: usize,
    pub target_pc: usize,
    pub snapshot_index: usize,
    pub actions: Vec<TraceExitAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracePlan {
    pub id: TraceId,
    pub chunk_key: usize,
    pub anchor_pc: usize,
    pub end_pc: usize,
    pub anchor_kind: TraceAnchorKind,
    pub instructions: Vec<TraceInstruction>,
    pub snapshots: Vec<TraceSnapshot>,
    pub guards: Vec<TraceGuard>,
    pub exits: Vec<TraceExit>,
}
