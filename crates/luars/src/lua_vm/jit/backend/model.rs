#[cfg(test)]
use crate::Instruction;

use crate::lua_vm::jit::helper_plan::{HelperPlan, HelperPlanDispatchSummary, HelperPlanStep};
use crate::lua_vm::jit::ir::TraceIr;
use crate::lua_vm::jit::lowering::{DeoptRestoreSummary, LoweredTrace};
use crate::lua_vm::jit::trace_recorder::TraceArtifact;
#[cfg(test)]
use crate::lua_vm::jit::trace_recorder::{TraceExit, TraceExitKind, TraceOp, TraceSeed};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BackendCompileOutcome {
    NotYetSupported,
    Compiled(CompiledTrace),
}

pub(crate) trait TraceBackend {
    fn compile(
        &mut self,
        artifact: &TraceArtifact,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinearIntStep {
    Move { dst: u32, src: u32 },
    LoadI { dst: u32, imm: i32 },
    Add { dst: u32, lhs: u32, rhs: u32 },
    AddI { dst: u32, src: u32, imm: i32 },
    Sub { dst: u32, lhs: u32, rhs: u32 },
    Mul { dst: u32, lhs: u32, rhs: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    BAnd,
    BOr,
    BXor,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericOperand {
    Reg(u32),
    ImmI(i32),
    Const(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericStep {
    Move { dst: u32, src: u32 },
    #[allow(dead_code)]
    LoadBool { dst: u32, value: bool },
    LoadI { dst: u32, imm: i32 },
    LoadF { dst: u32, imm: i32 },
    GetTableInt { dst: u32, table: u32, index: u32 },
    Binary {
        dst: u32,
        lhs: NumericOperand,
        rhs: NumericOperand,
        op: NumericBinaryOp,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericIfElseCond {
    IntCompare {
        op: LinearIntGuardOp,
        reg: u32,
        imm: i32,
    },
    RegCompare {
        op: LinearIntGuardOp,
        lhs: u32,
        rhs: u32,
    },
    Truthy {
        reg: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericJmpLoopGuard {
    Head {
        cond: NumericIfElseCond,
        continue_when: bool,
        continue_preset: Option<NumericStep>,
        exit_preset: Option<NumericStep>,
        exit_pc: u32,
    },
    Tail {
        cond: NumericIfElseCond,
        continue_when: bool,
        continue_preset: Option<NumericStep>,
        exit_preset: Option<NumericStep>,
        exit_pc: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinearIntGuardOp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinearIntLoopGuard {
    HeadRegReg {
        op: LinearIntGuardOp,
        lhs: u32,
        rhs: u32,
        continue_when: bool,
        exit_pc: u32,
    },
    HeadRegImm {
        op: LinearIntGuardOp,
        reg: u32,
        imm: i32,
        continue_when: bool,
        exit_pc: u32,
    },
    TailRegReg {
        op: LinearIntGuardOp,
        lhs: u32,
        rhs: u32,
        continue_when: bool,
        exit_pc: u32,
    },
    TailRegImm {
        op: LinearIntGuardOp,
        reg: u32,
        imm: i32,
        continue_when: bool,
        exit_pc: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CompiledTraceExecutor {
    SummaryOnly,
    LinearIntForLoop {
        loop_reg: u32,
        steps: Vec<LinearIntStep>,
    },
    NumericForLoop {
        loop_reg: u32,
        steps: Vec<NumericStep>,
    },
    NumericIfElseForLoop {
        loop_reg: u32,
        pre_steps: Vec<NumericStep>,
        cond: NumericIfElseCond,
        then_preset: Option<NumericStep>,
        else_preset: Option<NumericStep>,
        then_steps: Vec<NumericStep>,
        else_steps: Vec<NumericStep>,
        then_on_true: bool,
    },
    NumericJmpLoop {
        pre_steps: Vec<NumericStep>,
        steps: Vec<NumericStep>,
        guard: NumericJmpLoopGuard,
    },
    NumericTableScanJmpLoop {
        table_reg: u32,
        index_reg: u32,
        limit_reg: u32,
        step_imm: i32,
        compare_op: LinearIntGuardOp,
        exit_pc: u32,
    },
    NumericTableShiftJmpLoop {
        table_reg: u32,
        index_reg: u32,
        left_bound_reg: u32,
        value_reg: u32,
        temp_reg: u32,
        exit_pc: u32,
    },
    LinearIntJmpLoop {
        steps: Vec<LinearIntStep>,
        guard: LinearIntLoopGuard,
    },
    GenericForBuiltinAdd {
        tfor_reg: u32,
        value_reg: u32,
        acc_reg: u32,
    },
    NextWhileBuiltinAdd {
        key_reg: u32,
        value_reg: u32,
        acc_reg: u32,
        table_reg: u32,
        env_upvalue: u32,
        key_const: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompiledTraceExit {
    pub exit_index: u16,
    pub exit_pc: u32,
    pub resume_pc: u32,
    pub is_loop_backedge: bool,
    pub restore_summary: DeoptRestoreSummary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompiledTrace {
    pub root_pc: u32,
    pub loop_tail_pc: u32,
    pub steps: Vec<CompiledTraceStepKind>,
    exits: Vec<CompiledTraceExit>,
    summary: HelperPlanDispatchSummary,
    executor: CompiledTraceExecutor,
}

impl CompiledTrace {
    #[cfg(test)]
    pub(crate) fn from_helper_plan(ir: &TraceIr, helper_plan: &HelperPlan) -> Option<Self> {
        let artifact = synthetic_artifact_for_ir(ir);
        let lowered_trace = LoweredTrace::lower(&artifact, ir, helper_plan);
        Self::from_artifact_helper_plan(&artifact, ir, &lowered_trace, helper_plan)
    }

    fn from_artifact_helper_plan(
        artifact: &TraceArtifact,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
        helper_plan: &HelperPlan,
    ) -> Option<Self> {
        let mut steps = Vec::with_capacity(helper_plan.steps.len());
        let mut has_helper_call = false;
        let mut summary = HelperPlanDispatchSummary::default();

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
            summary.steps_executed = summary.steps_executed.saturating_add(1);
            match kind {
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
            steps.push(kind);
        }

        let executor = super::compile::compile_executor(artifact, ir, lowered_trace);

        if !has_helper_call && matches!(executor, CompiledTraceExecutor::SummaryOnly) {
            return None;
        }

        let exits = lowered_trace
            .exits
            .iter()
            .map(|exit| CompiledTraceExit {
                exit_index: exit.exit_index,
                exit_pc: exit.exit_pc,
                resume_pc: exit.resume_pc,
                is_loop_backedge: exit.is_loop_backedge,
                restore_summary: exit.restore_summary,
            })
            .collect();

        Some(Self {
            root_pc: helper_plan.root_pc,
            loop_tail_pc: helper_plan.loop_tail_pc,
            steps,
            exits,
            summary,
            executor,
        })
    }

    #[cfg(test)]
    pub(crate) fn execute(&self) -> HelperPlanDispatchSummary {
        self.summary
    }

    pub(crate) fn summary(&self) -> HelperPlanDispatchSummary {
        self.summary
    }

    pub(crate) fn exits(&self) -> &[CompiledTraceExit] {
        &self.exits
    }

    pub(crate) fn executor_family(&self) -> &'static str {
        match self.executor {
            CompiledTraceExecutor::SummaryOnly => "SummaryOnly",
            CompiledTraceExecutor::LinearIntForLoop { .. } => "LinearIntForLoop",
            CompiledTraceExecutor::NumericForLoop { .. } => "NumericForLoop",
            CompiledTraceExecutor::NumericIfElseForLoop { .. } => "NumericIfElseForLoop",
            CompiledTraceExecutor::NumericJmpLoop { .. } => "NumericJmpLoop",
            CompiledTraceExecutor::NumericTableScanJmpLoop { .. } => "NumericTableScanJmpLoop",
            CompiledTraceExecutor::NumericTableShiftJmpLoop { .. } => "NumericTableShiftJmpLoop",
            CompiledTraceExecutor::LinearIntJmpLoop { .. } => "LinearIntJmpLoop",
            CompiledTraceExecutor::GenericForBuiltinAdd { .. } => "GenericForBuiltinAdd",
            CompiledTraceExecutor::NextWhileBuiltinAdd { .. } => "NextWhileBuiltinAdd",
        }
    }

    pub(crate) fn executor(&self) -> CompiledTraceExecutor {
        self.executor.clone()
    }
}

#[derive(Default)]
pub(crate) struct NullTraceBackend;

impl TraceBackend for NullTraceBackend {
    fn compile(
        &mut self,
        artifact: &TraceArtifact,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome {
        match CompiledTrace::from_artifact_helper_plan(artifact, ir, lowered_trace, helper_plan) {
            Some(compiled_trace) => BackendCompileOutcome::Compiled(compiled_trace),
            None => BackendCompileOutcome::NotYetSupported,
        }
    }
}

#[cfg(test)]
impl NullTraceBackend {
    pub(crate) fn compile_test(
        &mut self,
        ir: &TraceIr,
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome {
        let artifact = synthetic_artifact_for_ir(ir);
        let lowered_trace = LoweredTrace::lower(&artifact, ir, helper_plan);
        <Self as TraceBackend>::compile(self, &artifact, ir, &lowered_trace, helper_plan)
    }
}

#[cfg(test)]
fn synthetic_artifact_for_ir(ir: &TraceIr) -> TraceArtifact {
    TraceArtifact {
        seed: TraceSeed {
            start_pc: ir.root_pc,
            root_chunk_addr: 0,
            instruction_budget: ir.insts.len().min(u16::MAX as usize) as u16,
        },
        ops: ir
            .insts
            .iter()
            .map(|inst| TraceOp {
                pc: inst.pc,
                instruction: Instruction::from_u32(inst.raw_instruction),
                opcode: inst.opcode,
            })
            .collect(),
        exits: ir
            .guards
            .iter()
            .map(|guard| TraceExit {
                guard_pc: guard.guard_pc,
                branch_pc: guard.branch_pc,
                exit_pc: guard.exit_pc,
                taken_on_trace: guard.taken_on_trace,
                kind: TraceExitKind::GuardExit,
            })
            .collect(),
        loop_tail_pc: ir.loop_tail_pc,
    }
}