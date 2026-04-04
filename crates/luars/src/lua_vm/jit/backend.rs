use super::helper_plan::{HelperPlan, HelperPlanDispatchSummary, HelperPlanStep};
use super::ir::TraceIr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BackendCompileOutcome {
    NotYetSupported,
    Compiled(CompiledTrace),
}

pub(crate) trait TraceBackend {
    fn compile(&mut self, ir: &TraceIr, helper_plan: &HelperPlan) -> BackendCompileOutcome;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompiledTraceStepKind {
    LoadMove,
    UpvalueAccess,
    UpvalueMutation,
    Cleanup,
    TableAccess,
    Arithmetic,
    Call,
    MetamethodFallback,
    ClosureCreation,
    LoopPrep,
    Guard,
    Branch,
    LoopBackedge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompiledTrace {
    pub root_pc: u32,
    pub loop_tail_pc: u32,
    pub steps: Vec<CompiledTraceStepKind>,
}

impl CompiledTrace {
    pub(crate) fn from_helper_plan(helper_plan: &HelperPlan) -> Option<Self> {
        let mut steps = Vec::with_capacity(helper_plan.steps.len());
        let mut has_helper_call = false;

        for step in &helper_plan.steps {
            let kind = match step {
                HelperPlanStep::LoadMove { .. } => CompiledTraceStepKind::LoadMove,
                HelperPlanStep::UpvalueAccess { .. } => CompiledTraceStepKind::UpvalueAccess,
                HelperPlanStep::UpvalueMutation { .. } => CompiledTraceStepKind::UpvalueMutation,
                HelperPlanStep::Cleanup { .. } => CompiledTraceStepKind::Cleanup,
                HelperPlanStep::TableAccess { .. } => CompiledTraceStepKind::TableAccess,
                HelperPlanStep::Arithmetic { .. } => CompiledTraceStepKind::Arithmetic,
                HelperPlanStep::Call { .. } => {
                    has_helper_call = true;
                    CompiledTraceStepKind::Call
                }
                HelperPlanStep::MetamethodFallback { .. } => {
                    has_helper_call = true;
                    CompiledTraceStepKind::MetamethodFallback
                }
                HelperPlanStep::ClosureCreation { .. } => CompiledTraceStepKind::ClosureCreation,
                HelperPlanStep::LoopPrep { .. } => CompiledTraceStepKind::LoopPrep,
                HelperPlanStep::Guard { .. } => CompiledTraceStepKind::Guard,
                HelperPlanStep::Branch { .. } => CompiledTraceStepKind::Branch,
                HelperPlanStep::LoopBackedge { .. } => CompiledTraceStepKind::LoopBackedge,
            };
            steps.push(kind);
        }

        if !has_helper_call {
            return None;
        }

        Some(Self {
            root_pc: helper_plan.root_pc,
            loop_tail_pc: helper_plan.loop_tail_pc,
            steps,
        })
    }

    pub(crate) fn execute(&self) -> HelperPlanDispatchSummary {
        let mut summary = HelperPlanDispatchSummary::default();

        for step in &self.steps {
            summary.steps_executed = summary.steps_executed.saturating_add(1);
            match step {
                CompiledTraceStepKind::Call => {
                    summary.call_steps = summary.call_steps.saturating_add(1);
                }
                CompiledTraceStepKind::MetamethodFallback => {
                    summary.metamethod_steps = summary.metamethod_steps.saturating_add(1);
                }
                CompiledTraceStepKind::Guard => {
                    summary.guards_observed = summary.guards_observed.saturating_add(1);
                }
                CompiledTraceStepKind::LoadMove
                | CompiledTraceStepKind::UpvalueAccess
                | CompiledTraceStepKind::UpvalueMutation
                | CompiledTraceStepKind::Cleanup
                | CompiledTraceStepKind::TableAccess
                | CompiledTraceStepKind::Arithmetic
                | CompiledTraceStepKind::ClosureCreation
                | CompiledTraceStepKind::LoopPrep
                | CompiledTraceStepKind::Branch
                | CompiledTraceStepKind::LoopBackedge => {}
            }
        }

        summary
    }
}

#[derive(Default)]
pub(crate) struct NullTraceBackend;

impl TraceBackend for NullTraceBackend {
    fn compile(&mut self, _ir: &TraceIr, helper_plan: &HelperPlan) -> BackendCompileOutcome {
        match CompiledTrace::from_helper_plan(helper_plan) {
            Some(compiled_trace) => BackendCompileOutcome::Compiled(compiled_trace),
            None => BackendCompileOutcome::NotYetSupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackendCompileOutcome, CompiledTrace, CompiledTraceStepKind, NullTraceBackend,
        TraceBackend,
    };
    use crate::lua_vm::jit::helper_plan::{HelperPlan, HelperPlanStep};
    use crate::lua_vm::jit::ir::{TraceIr, TraceIrOperand};

    #[test]
    fn null_backend_reports_not_yet_supported() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 0,
            insts: Vec::new(),
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 0,
            loop_tail_pc: 0,
            steps: Vec::new(),
            guard_count: 0,
        };

        assert_eq!(
            backend.compile(&ir, &helper_plan),
            BackendCompileOutcome::NotYetSupported
        );
    }

    #[test]
    fn backend_compiles_helper_call_trace() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 2,
            insts: Vec::new(),
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 0,
            loop_tail_pc: 2,
            steps: vec![
                HelperPlanStep::Call {
                    reads: vec![TraceIrOperand::Register(0)],
                    writes: vec![TraceIrOperand::Register(0)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(1)],
                },
                HelperPlanStep::LoopBackedge {
                    reads: vec![TraceIrOperand::JumpTarget(0)],
                    writes: Vec::new(),
                },
            ],
            guard_count: 0,
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.steps,
                    vec![
                        CompiledTraceStepKind::Call,
                        CompiledTraceStepKind::MetamethodFallback,
                        CompiledTraceStepKind::LoopBackedge,
                    ]
                );
                let summary = compiled.execute();
                assert_eq!(summary.call_steps, 1);
                assert_eq!(summary.metamethod_steps, 1);
            }
            BackendCompileOutcome::NotYetSupported => {
                panic!("expected helper-call trace to compile")
            }
        }
    }

    #[test]
    fn compiled_trace_requires_helper_call_family() {
        let helper_plan = HelperPlan {
            root_pc: 0,
            loop_tail_pc: 1,
            steps: vec![HelperPlanStep::LoadMove {
                reads: Vec::new(),
                writes: Vec::new(),
            }],
            guard_count: 0,
        };

        assert_eq!(CompiledTrace::from_helper_plan(&helper_plan), None);
    }
}