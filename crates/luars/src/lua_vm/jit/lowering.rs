use crate::OpCode;

use super::{
    TraceAnchorKind, TraceExit, TraceFallback, TraceGuard, TraceId, TracePlan, TraceSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoweredTraceAnchor {
    pub pc: usize,
    pub end_pc: usize,
    pub kind: TraceAnchorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoweredTraceInstruction {
    pub pc: usize,
    pub opcode: OpCode,
    pub line: Option<u32>,
    pub fallback: Option<TraceFallback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredTrace {
    pub id: TraceId,
    pub chunk_key: usize,
    pub anchor: LoweredTraceAnchor,
    pub instructions: Vec<LoweredTraceInstruction>,
    pub snapshots: Vec<TraceSnapshot>,
    pub guards: Vec<TraceGuard>,
    pub exits: Vec<TraceExit>,
}

impl LoweredTrace {
    pub fn lower(plan: &TracePlan) -> Self {
        Self {
            id: plan.id,
            chunk_key: plan.chunk_key,
            anchor: LoweredTraceAnchor {
                pc: plan.anchor_pc,
                end_pc: plan.end_pc,
                kind: plan.anchor_kind,
            },
            instructions: plan
                .instructions
                .iter()
                .map(|instruction| LoweredTraceInstruction {
                    pc: instruction.pc,
                    opcode: instruction.opcode,
                    line: instruction.line,
                    fallback: instruction.fallback,
                })
                .collect(),
            snapshots: plan.snapshots.clone(),
            guards: plan.guards.clone(),
            exits: plan.exits.clone(),
        }
    }
}
