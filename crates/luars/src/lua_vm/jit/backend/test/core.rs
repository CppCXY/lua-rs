use super::*;

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
        summary: HelperPlanDispatchSummary::default(),
    };

    assert_eq!(
        backend.compile_test(&ir, &helper_plan),
        BackendCompileOutcome::NotYetSupported
    );
}

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
        summary: HelperPlanDispatchSummary {
            steps_executed: 3,
            guards_observed: 0,
            call_steps: 1,
            metamethod_steps: 1,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
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
fn compiled_trace_requires_helper_call_family() {
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 1,
        insts: Vec::new(),
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 1,
        steps: vec![HelperPlanStep::LoadMove {
            reads: Vec::new(),
            writes: Vec::new(),
        }],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 1,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    assert_eq!(CompiledTrace::from_helper_plan(&ir, &helper_plan), None);
}

