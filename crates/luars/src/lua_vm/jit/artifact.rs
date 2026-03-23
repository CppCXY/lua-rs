use crate::{Chunk, LuaState};

use super::{JitPolicy, TraceAbortReason, TraceCompilationUnit, TraceId, TracePlan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTraceStub {
    pub unit: TraceCompilationUnit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceArtifact {
    Replay(TracePlan),
    NativePlaceholder(NativeTraceStub),
}

impl TraceArtifact {
    pub fn id(&self) -> TraceId {
        match self {
            Self::Replay(plan) => plan.id,
            Self::NativePlaceholder(stub) => stub.unit.plan.id,
        }
    }

    pub fn plan(&self) -> &TracePlan {
        match self {
            Self::Replay(plan) => plan,
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
            Self::NativePlaceholder(_) => Err(TraceAbortReason::NotImplemented),
        }
    }
}
