use super::ir::{
    TraceIr, TraceIrInstKind, TraceIrOperand, is_fused_arithmetic_metamethod_fallback,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HelperPlanDispatchSummary {
    pub steps_executed: u32,
    pub guards_observed: u32,
    pub call_steps: u32,
    pub metamethod_steps: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HelperPlanStep {
    LoadMove {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
    UpvalueAccess {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
    UpvalueMutation {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
    Cleanup {
        reads: Vec<TraceIrOperand>,
    },
    TableAccess {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
    Arithmetic {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
    Call {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
    MetamethodFallback {
        reads: Vec<TraceIrOperand>,
    },
    ClosureCreation {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
    LoopPrep {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
    Guard {
        reads: Vec<TraceIrOperand>,
    },
    Branch {
        reads: Vec<TraceIrOperand>,
    },
    LoopBackedge {
        reads: Vec<TraceIrOperand>,
        writes: Vec<TraceIrOperand>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HelperPlan {
    pub root_pc: u32,
    pub loop_tail_pc: u32,
    pub steps: Vec<HelperPlanStep>,
    pub guard_count: u16,
    pub(crate) summary: HelperPlanDispatchSummary,
}

impl HelperPlan {
    pub(crate) fn lower(ir: &TraceIr) -> Self {
        let steps = ir
            .insts
            .iter()
            .enumerate()
            .filter(|(index, _)| !is_fused_arithmetic_metamethod_fallback(&ir.insts, *index))
            .map(|(_, inst)| match inst.kind {
                TraceIrInstKind::LoadMove => HelperPlanStep::LoadMove {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
                TraceIrInstKind::UpvalueAccess => HelperPlanStep::UpvalueAccess {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
                TraceIrInstKind::UpvalueMutation => HelperPlanStep::UpvalueMutation {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
                TraceIrInstKind::Cleanup => HelperPlanStep::Cleanup {
                    reads: inst.reads.clone(),
                },
                TraceIrInstKind::TableAccess => HelperPlanStep::TableAccess {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
                TraceIrInstKind::Arithmetic => HelperPlanStep::Arithmetic {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
                TraceIrInstKind::Call => HelperPlanStep::Call {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
                TraceIrInstKind::MetamethodFallback => HelperPlanStep::MetamethodFallback {
                    reads: inst.reads.clone(),
                },
                TraceIrInstKind::ClosureCreation => HelperPlanStep::ClosureCreation {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
                TraceIrInstKind::LoopPrep => HelperPlanStep::LoopPrep {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
                TraceIrInstKind::Guard => HelperPlanStep::Guard {
                    reads: inst.reads.clone(),
                },
                TraceIrInstKind::Branch => HelperPlanStep::Branch {
                    reads: inst.reads.clone(),
                },
                TraceIrInstKind::LoopBackedge => HelperPlanStep::LoopBackedge {
                    reads: inst.reads.clone(),
                    writes: inst.writes.clone(),
                },
            })
            .collect::<Vec<_>>();

        let summary = summarize_steps(&steps);

        Self {
            root_pc: ir.root_pc,
            loop_tail_pc: ir.loop_tail_pc,
            steps,
            guard_count: ir.guards.len() as u16,
            summary,
        }
    }

    #[cfg(test)]
    pub(crate) fn dispatch(&self) -> HelperPlanDispatchSummary {
        self.summary
    }
}

fn execute_load_move_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
}

fn execute_upvalue_access_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
}

fn execute_upvalue_mutation_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
}

fn execute_cleanup_helper(_reads: &[TraceIrOperand], summary: &mut HelperPlanDispatchSummary) {
    record_helper_step(summary);
}

fn execute_table_access_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
}

fn execute_arithmetic_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
}

fn execute_guard_helper(_reads: &[TraceIrOperand], summary: &mut HelperPlanDispatchSummary) {
    record_helper_step(summary);
    summary.guards_observed = summary.guards_observed.saturating_add(1);
}

fn execute_call_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
    summary.call_steps = summary.call_steps.saturating_add(1);
}

fn execute_metamethod_helper(_reads: &[TraceIrOperand], summary: &mut HelperPlanDispatchSummary) {
    record_helper_step(summary);
    summary.metamethod_steps = summary.metamethod_steps.saturating_add(1);
}

fn execute_closure_creation_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
}

fn execute_loop_prep_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
}

fn execute_branch_helper(_reads: &[TraceIrOperand], summary: &mut HelperPlanDispatchSummary) {
    record_helper_step(summary);
}

fn execute_loop_backedge_helper(
    _reads: &[TraceIrOperand],
    _writes: &[TraceIrOperand],
    summary: &mut HelperPlanDispatchSummary,
) {
    record_helper_step(summary);
}

fn record_helper_step(summary: &mut HelperPlanDispatchSummary) {
    summary.steps_executed = summary.steps_executed.saturating_add(1);
}

fn summarize_steps(steps: &[HelperPlanStep]) -> HelperPlanDispatchSummary {
    let mut summary = HelperPlanDispatchSummary::default();

    for step in steps {
        match step {
            HelperPlanStep::LoadMove { reads, writes } => {
                execute_load_move_helper(reads, writes, &mut summary);
            }
            HelperPlanStep::UpvalueAccess { reads, writes } => {
                execute_upvalue_access_helper(reads, writes, &mut summary);
            }
            HelperPlanStep::UpvalueMutation { reads, writes } => {
                execute_upvalue_mutation_helper(reads, writes, &mut summary);
            }
            HelperPlanStep::Cleanup { reads } => {
                execute_cleanup_helper(reads, &mut summary);
            }
            HelperPlanStep::TableAccess { reads, writes } => {
                execute_table_access_helper(reads, writes, &mut summary);
            }
            HelperPlanStep::Arithmetic { reads, writes } => {
                execute_arithmetic_helper(reads, writes, &mut summary);
            }
            HelperPlanStep::Call { reads, writes } => {
                execute_call_helper(reads, writes, &mut summary);
            }
            HelperPlanStep::MetamethodFallback { reads } => {
                execute_metamethod_helper(reads, &mut summary);
            }
            HelperPlanStep::ClosureCreation { reads, writes } => {
                execute_closure_creation_helper(reads, writes, &mut summary);
            }
            HelperPlanStep::LoopPrep { reads, writes } => {
                execute_loop_prep_helper(reads, writes, &mut summary);
            }
            HelperPlanStep::Guard { reads } => {
                execute_guard_helper(reads, &mut summary);
            }
            HelperPlanStep::Branch { reads } => {
                execute_branch_helper(reads, &mut summary);
            }
            HelperPlanStep::LoopBackedge { reads, writes } => {
                execute_loop_backedge_helper(reads, writes, &mut summary);
            }
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use crate::Instruction;
    use crate::OpCode;
    use crate::lua_vm::jit::helper_plan::{HelperPlan, HelperPlanDispatchSummary, HelperPlanStep};
    use crate::lua_vm::jit::ir::{
        TraceIr, TraceIrGuard, TraceIrGuardKind, TraceIrInst, TraceIrInstKind, TraceIrOperand,
    };

    #[test]
    fn lower_ir_to_helper_plan() {
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 4,
            insts: vec![
                TraceIrInst {
                    pc: 0,
                    opcode: OpCode::Move,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::LoadMove,
                    reads: vec![TraceIrOperand::Register(1)],
                    writes: vec![TraceIrOperand::Register(0)],
                },
                TraceIrInst {
                    pc: 1,
                    opcode: OpCode::Test,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::Guard,
                    reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Bool(false)],
                    writes: vec![],
                },
                TraceIrInst {
                    pc: 4,
                    opcode: OpCode::Jmp,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(0)],
                    writes: vec![],
                },
            ],
            guards: vec![TraceIrGuard {
                guard_pc: 1,
                branch_pc: 2,
                exit_pc: 4,
                taken_on_trace: false,
                kind: TraceIrGuardKind::SideExit,
            }],
        };

        let plan = HelperPlan::lower(&ir);
        assert_eq!(plan.root_pc, 0);
        assert_eq!(plan.loop_tail_pc, 4);
        assert_eq!(plan.guard_count, 1);
        assert_eq!(plan.steps.len(), 3);
        match &plan.steps[0] {
            HelperPlanStep::LoadMove { reads, writes } => {
                assert_eq!(reads, &vec![TraceIrOperand::Register(1)]);
                assert_eq!(writes, &vec![TraceIrOperand::Register(0)]);
            }
            _ => panic!("expected load/move step"),
        }

        assert_eq!(
            plan.dispatch(),
            HelperPlanDispatchSummary {
                steps_executed: 3,
                guards_observed: 1,
                call_steps: 0,
                metamethod_steps: 0,
            }
        );
    }

    #[test]
    fn dispatch_counts_call_and_metamethod_steps() {
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 2,
            insts: vec![
                TraceIrInst {
                    pc: 0,
                    opcode: OpCode::Call,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::Call,
                    reads: vec![TraceIrOperand::Register(0)],
                    writes: vec![TraceIrOperand::Register(0)],
                },
                TraceIrInst {
                    pc: 1,
                    opcode: OpCode::MmBin,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(1)],
                    writes: vec![],
                },
                TraceIrInst {
                    pc: 2,
                    opcode: OpCode::Jmp,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(0)],
                    writes: vec![],
                },
            ],
            guards: Vec::new(),
        };

        let plan = HelperPlan::lower(&ir);
        assert_eq!(
            plan.dispatch(),
            HelperPlanDispatchSummary {
                steps_executed: 3,
                guards_observed: 0,
                call_steps: 1,
                metamethod_steps: 1,
            }
        );
    }

    #[test]
    fn lower_skips_fused_arithmetic_metamethod_companion_steps() {
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 2,
            insts: vec![
                TraceIrInst {
                    pc: 0,
                    opcode: OpCode::AddK,
                    raw_instruction: Instruction::create_abck(OpCode::AddK, 4, 4, 0, false)
                        .as_u32(),
                    kind: TraceIrInstKind::Arithmetic,
                    reads: vec![
                        TraceIrOperand::Register(4),
                        TraceIrOperand::ConstantIndex(0),
                    ],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                TraceIrInst {
                    pc: 1,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 6, false)
                        .as_u32(),
                    kind: TraceIrInstKind::MetamethodFallback,
                    reads: vec![
                        TraceIrOperand::Register(4),
                        TraceIrOperand::ConstantIndex(0),
                    ],
                    writes: vec![],
                },
                TraceIrInst {
                    pc: 2,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 3).as_u32(),
                    kind: TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(0)],
                    writes: vec![],
                },
            ],
            guards: Vec::new(),
        };

        let plan = HelperPlan::lower(&ir);

        assert_eq!(plan.steps.len(), 2);
        assert!(matches!(plan.steps[0], HelperPlanStep::Arithmetic { .. }));
        assert!(matches!(plan.steps[1], HelperPlanStep::LoopBackedge { .. }));
        assert_eq!(plan.summary.metamethod_steps, 0);
    }

    #[test]
    fn lower_ir_with_upvalue_and_loop_prep_steps() {
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 2,
            insts: vec![
                TraceIrInst {
                    pc: 0,
                    opcode: OpCode::GetUpval,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::UpvalueAccess,
                    reads: vec![TraceIrOperand::Upvalue(1)],
                    writes: vec![TraceIrOperand::Register(0)],
                },
                TraceIrInst {
                    pc: 1,
                    opcode: OpCode::ForPrep,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::LoopPrep,
                    reads: vec![TraceIrOperand::RegisterRange { start: 0, count: 3 }],
                    writes: vec![TraceIrOperand::RegisterRange { start: 0, count: 3 }],
                },
                TraceIrInst {
                    pc: 2,
                    opcode: OpCode::ForLoop,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(0)],
                    writes: vec![],
                },
            ],
            guards: Vec::new(),
        };

        let plan = HelperPlan::lower(&ir);
        assert!(matches!(
            plan.steps[0],
            HelperPlanStep::UpvalueAccess { .. }
        ));
        assert!(matches!(plan.steps[1], HelperPlanStep::LoopPrep { .. }));
        assert_eq!(
            plan.dispatch(),
            HelperPlanDispatchSummary {
                steps_executed: 3,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 0,
            }
        );
    }

    #[test]
    fn lower_ir_with_tforcall_setupval_and_closure_steps() {
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 3,
            insts: vec![
                TraceIrInst {
                    pc: 0,
                    opcode: OpCode::TForCall,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::Call,
                    reads: vec![TraceIrOperand::Register(0)],
                    writes: vec![TraceIrOperand::RegisterRange { start: 3, count: 2 }],
                },
                TraceIrInst {
                    pc: 1,
                    opcode: OpCode::SetUpval,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::UpvalueMutation,
                    reads: vec![TraceIrOperand::Register(1), TraceIrOperand::Upvalue(2)],
                    writes: vec![TraceIrOperand::Upvalue(2)],
                },
                TraceIrInst {
                    pc: 2,
                    opcode: OpCode::Closure,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::ClosureCreation,
                    reads: vec![TraceIrOperand::UnsignedImmediate(5)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                TraceIrInst {
                    pc: 3,
                    opcode: OpCode::Jmp,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(0)],
                    writes: vec![],
                },
            ],
            guards: Vec::new(),
        };

        let plan = HelperPlan::lower(&ir);
        assert!(matches!(plan.steps[0], HelperPlanStep::Call { .. }));
        assert!(matches!(
            plan.steps[1],
            HelperPlanStep::UpvalueMutation { .. }
        ));
        assert!(matches!(
            plan.steps[2],
            HelperPlanStep::ClosureCreation { .. }
        ));
        assert_eq!(
            plan.dispatch(),
            HelperPlanDispatchSummary {
                steps_executed: 4,
                guards_observed: 0,
                call_steps: 1,
                metamethod_steps: 0,
            }
        );
    }

    #[test]
    fn lower_ir_with_tforloop_setlist_and_close_steps() {
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 3,
            insts: vec![
                TraceIrInst {
                    pc: 0,
                    opcode: OpCode::SetList,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::TableAccess,
                    reads: vec![TraceIrOperand::Register(1)],
                    writes: vec![],
                },
                TraceIrInst {
                    pc: 1,
                    opcode: OpCode::Close,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::Cleanup,
                    reads: vec![TraceIrOperand::Register(2)],
                    writes: vec![],
                },
                TraceIrInst {
                    pc: 2,
                    opcode: OpCode::TForCall,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::Call,
                    reads: vec![TraceIrOperand::Register(0)],
                    writes: vec![TraceIrOperand::RegisterRange { start: 3, count: 2 }],
                },
                TraceIrInst {
                    pc: 3,
                    opcode: OpCode::TForLoop,
                    raw_instruction: 0,
                    kind: TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::Register(3), TraceIrOperand::JumpTarget(0)],
                    writes: vec![],
                },
            ],
            guards: Vec::new(),
        };

        let plan = HelperPlan::lower(&ir);
        assert!(matches!(plan.steps[0], HelperPlanStep::TableAccess { .. }));
        assert!(matches!(plan.steps[1], HelperPlanStep::Cleanup { .. }));
        assert!(matches!(plan.steps[2], HelperPlanStep::Call { .. }));
        assert!(matches!(plan.steps[3], HelperPlanStep::LoopBackedge { .. }));
        assert_eq!(
            plan.dispatch(),
            HelperPlanDispatchSummary {
                steps_executed: 4,
                guards_observed: 0,
                call_steps: 1,
                metamethod_steps: 0,
            }
        );
    }
}
