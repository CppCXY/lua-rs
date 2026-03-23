use std::fmt::Debug;

use super::{
    LoweredTrace, TraceExitKind, TracePlan,
    artifact::{
        CompiledTraceArtifact, CompiledTraceExit, CompiledTraceGuard, CompiledTraceStep,
        TraceArtifact,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceBackendKind {
    Noop,
    Lowered,
    #[cfg(feature = "jit-cranelift")]
    Cranelift,
}

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

    fn kind(&self) -> TraceBackendKind;
}

#[derive(Debug, Default)]
pub struct NoopTraceBackend;

impl TraceBackend for NoopTraceBackend {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn kind(&self) -> TraceBackendKind {
        TraceBackendKind::Noop
    }

    fn compile(
        &self,
        _unit: &TraceCompilationUnit,
    ) -> Result<Option<TraceArtifact>, TraceBackendError> {
        Ok(None)
    }
}

#[derive(Debug, Default)]
pub struct LoweredTraceBackend;

impl TraceBackend for LoweredTraceBackend {
    fn name(&self) -> &'static str {
        "lowered"
    }

    fn kind(&self) -> TraceBackendKind {
        TraceBackendKind::Lowered
    }

    fn compile(
        &self,
        unit: &TraceCompilationUnit,
    ) -> Result<Option<TraceArtifact>, TraceBackendError> {
        let steps = unit
            .lowered
            .instructions
            .iter()
            .map(|instruction| {
                let guards = unit
                    .lowered
                    .guards
                    .iter()
                    .filter(|guard| guard.pc == instruction.pc)
                    .map(|guard| {
                        let exit = unit
                            .lowered
                            .exits
                            .iter()
                            .find(|exit| exit.snapshot_index == guard.exit_snapshot_index)
                            .ok_or(TraceBackendError::UnsupportedTrace)?;
                        Ok(CompiledTraceGuard {
                            guard: *guard,
                            exit: CompiledTraceExit {
                                target_pc: exit.target_pc,
                                snapshot_index: exit.snapshot_index,
                                actions: exit.actions.clone(),
                            },
                        })
                    })
                    .collect::<Result<Vec<_>, TraceBackendError>>()?;

                let loop_exit = unit
                    .lowered
                    .exits
                    .iter()
                    .find(|exit| {
                        exit.kind == TraceExitKind::LoopExit
                            && exit.source_pc == instruction.pc
                            && exit.target_pc != instruction.pc
                    })
                    .map(|exit| CompiledTraceExit {
                        target_pc: exit.target_pc,
                        snapshot_index: exit.snapshot_index,
                        actions: exit.actions.clone(),
                    });

                Ok(CompiledTraceStep {
                    instruction: *instruction,
                    guards,
                    loop_exit,
                })
            })
            .collect::<Result<Vec<_>, TraceBackendError>>()?;

        Ok(Some(TraceArtifact::Compiled(CompiledTraceArtifact {
            unit: unit.clone(),
            steps,
        })))
    }
}

pub fn make_trace_backend(kind: TraceBackendKind) -> Box<dyn TraceBackend> {
    match kind {
        TraceBackendKind::Noop => Box::new(NoopTraceBackend),
        TraceBackendKind::Lowered => Box::new(LoweredTraceBackend),
        #[cfg(feature = "jit-cranelift")]
        TraceBackendKind::Cranelift => Box::new(super::cranelift::CraneliftTraceBackend),
    }
}
