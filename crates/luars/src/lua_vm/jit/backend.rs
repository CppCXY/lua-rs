use std::fmt::Debug;

use super::{LoweredTrace, TracePlan, artifact::TraceArtifact};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceCompilationUnit {
    pub plan: TracePlan,
    pub lowered: LoweredTrace,
}

impl TraceCompilationUnit {
    pub fn new(plan: TracePlan) -> Self {
        let lowered = LoweredTrace::lower(&plan);
        Self { plan, lowered }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceBackendError {
    UnsupportedTrace,
    CompileFailed,
}

pub trait TraceBackend: Debug {
    fn name(&self) -> &'static str;

    fn compile(
        &self,
        unit: &TraceCompilationUnit,
    ) -> Result<Option<TraceArtifact>, TraceBackendError>;
}

#[derive(Debug, Default)]
pub struct NoopTraceBackend;

impl TraceBackend for NoopTraceBackend {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn compile(
        &self,
        _unit: &TraceCompilationUnit,
    ) -> Result<Option<TraceArtifact>, TraceBackendError> {
        Ok(None)
    }
}
