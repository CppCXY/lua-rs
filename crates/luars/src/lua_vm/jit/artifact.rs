use crate::{Chunk, LuaState};

use super::{
    JitPolicy, LoweredTraceInstruction, TraceAbortReason, TraceCompilationUnit,
    TraceDispatchResult, TraceExitAction, TraceGuard, TraceId, TracePlan, TraceRunResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTraceExit {
    pub target_pc: usize,
    pub snapshot_index: usize,
    pub side_trace: Option<TraceId>,
    pub actions: Vec<TraceExitAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTraceGuard {
    pub guard: TraceGuard,
    pub exit: CompiledTraceExit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTraceStep {
    pub instruction: LoweredTraceInstruction,
    pub guards: Vec<CompiledTraceGuard>,
    pub loop_exit: Option<CompiledTraceExit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTraceArtifact {
    pub unit: TraceCompilationUnit,
    pub steps: Vec<CompiledTraceStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTraceStub {
    pub unit: TraceCompilationUnit,
}

#[cfg(feature = "jit-cranelift")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraneliftTraceArtifact {
    pub compiled: CompiledTraceArtifact,
    pub entry: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceArtifact {
    Replay(TracePlan),
    Compiled(CompiledTraceArtifact),
    #[cfg(feature = "jit-cranelift")]
    Cranelift(CraneliftTraceArtifact),
    NativePlaceholder(NativeTraceStub),
}

impl TraceArtifact {
    pub fn id(&self) -> TraceId {
        match self {
            Self::Replay(plan) => plan.id,
            Self::Compiled(compiled) => compiled.unit.plan.id,
            #[cfg(feature = "jit-cranelift")]
            Self::Cranelift(artifact) => artifact.compiled.unit.plan.id,
            Self::NativePlaceholder(stub) => stub.unit.plan.id,
        }
    }

    pub fn plan(&self) -> &TracePlan {
        match self {
            Self::Replay(plan) => plan,
            Self::Compiled(compiled) => &compiled.unit.plan,
            #[cfg(feature = "jit-cranelift")]
            Self::Cranelift(artifact) => &artifact.compiled.unit.plan,
            Self::NativePlaceholder(stub) => &stub.unit.plan,
        }
    }

    pub fn execute(
        &self,
        lua_state: &mut LuaState,
        chunk: &Chunk,
        base: usize,
        policy: JitPolicy,
    ) -> Result<TraceRunResult, TraceAbortReason> {
        match self {
            Self::Replay(plan) => {
                super::replay::execute_trace(lua_state, chunk, plan, base, policy)
            }
            Self::Compiled(compiled) => {
                super::replay::execute_compiled_trace(lua_state, chunk, compiled, base, policy)
            }
            #[cfg(feature = "jit-cranelift")]
            Self::Cranelift(artifact) => super::cranelift::execute_cranelift_trace(
                lua_state, chunk, artifact, base, policy,
            ),
            Self::NativePlaceholder(_) => Err(TraceAbortReason::NotImplemented),
        }
    }

    pub fn execute_tree(
        &self,
        lua_state: &mut LuaState,
        chunk: &Chunk,
        base: usize,
        policy: JitPolicy,
    ) -> Result<TraceDispatchResult, TraceAbortReason> {
        match self {
            Self::Replay(plan) => {
                super::replay::execute_trace_tree(lua_state, chunk, plan, base, policy)
            }
            Self::Compiled(compiled) => {
                super::replay::execute_compiled_trace_tree(lua_state, chunk, compiled, base, policy)
            }
            #[cfg(feature = "jit-cranelift")]
            Self::Cranelift(artifact) => super::replay::execute_cranelift_trace_tree(
                lua_state,
                chunk,
                artifact,
                base,
                policy,
            ),
            Self::NativePlaceholder(_) => Err(TraceAbortReason::NotImplemented),
        }
    }

    pub fn link_side_trace(&mut self, snapshot_index: usize, side_trace_id: TraceId) -> bool {
        match self {
            Self::Replay(plan) => link_plan_exit(&mut plan.exits, snapshot_index, side_trace_id),
            Self::Compiled(compiled) => {
                let mut linked = link_plan_exit(
                    &mut compiled.unit.plan.exits,
                    snapshot_index,
                    side_trace_id,
                );
                linked |= link_compiled_steps(&mut compiled.steps, snapshot_index, side_trace_id);
                linked
            }
            #[cfg(feature = "jit-cranelift")]
            Self::Cranelift(artifact) => {
                let mut linked = link_plan_exit(
                    &mut artifact.compiled.unit.plan.exits,
                    snapshot_index,
                    side_trace_id,
                );
                linked |= link_compiled_steps(
                    &mut artifact.compiled.steps,
                    snapshot_index,
                    side_trace_id,
                );
                linked
            }
            Self::NativePlaceholder(stub) => {
                link_plan_exit(&mut stub.unit.plan.exits, snapshot_index, side_trace_id)
            }
        }
    }
}

fn link_plan_exit(exits: &mut [super::TraceExit], snapshot_index: usize, side_trace_id: TraceId) -> bool {
    if let Some(exit) = exits
        .iter_mut()
        .find(|exit| exit.snapshot_index == snapshot_index)
    {
        exit.side_trace = Some(side_trace_id);
        true
    } else {
        false
    }
}

fn link_compiled_steps(
    steps: &mut [CompiledTraceStep],
    snapshot_index: usize,
    side_trace_id: TraceId,
) -> bool {
    let mut linked = false;
    for step in steps {
        for guard in &mut step.guards {
            if guard.exit.snapshot_index == snapshot_index {
                guard.exit.side_trace = Some(side_trace_id);
                linked = true;
            }
        }
        if let Some(loop_exit) = &mut step.loop_exit {
            if loop_exit.snapshot_index == snapshot_index {
                loop_exit.side_trace = Some(side_trace_id);
                linked = true;
            }
        }
    }
    linked
}
