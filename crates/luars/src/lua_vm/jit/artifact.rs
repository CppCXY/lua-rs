use crate::{Chunk, LuaState};

use super::{
    JitPolicy, LoweredTraceInstruction, TraceAbortReason, TraceCompilationUnit, TraceExitAction,
    TraceGuard, TraceId, TracePlan,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTraceExit {
    pub target_pc: usize,
    pub snapshot_index: usize,
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
    ) -> Result<usize, TraceAbortReason> {
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
}
