#[cfg(test)]
use crate::Instruction;
use crate::LuaValue;

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
    BNot { dst: u32, src: u32 },
    Add { dst: u32, lhs: u32, rhs: u32 },
    AddI { dst: u32, src: u32, imm: i32 },
    Sub { dst: u32, lhs: u32, rhs: u32 },
    SubI { dst: u32, src: u32, imm: i32 },
    Mul { dst: u32, lhs: u32, rhs: u32 },
    MulI { dst: u32, src: u32, imm: i32 },
    IDiv { dst: u32, lhs: u32, rhs: u32 },
    IDivI { dst: u32, src: u32, imm: i32 },
    Mod { dst: u32, lhs: u32, rhs: u32 },
    ModI { dst: u32, src: u32, imm: i32 },
    BAnd { dst: u32, lhs: u32, rhs: u32 },
    BAndI { dst: u32, src: u32, imm: i32 },
    BOr { dst: u32, lhs: u32, rhs: u32 },
    BOrI { dst: u32, src: u32, imm: i32 },
    BXor { dst: u32, lhs: u32, rhs: u32 },
    BXorI { dst: u32, src: u32, imm: i32 },
    Shl { dst: u32, lhs: u32, rhs: u32 },
    ShlI { dst: u32, imm: i32, src: u32 },
    Shr { dst: u32, lhs: u32, rhs: u32 },
    ShrI { dst: u32, src: u32, imm: i32 },
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
    GetUpval { dst: u32, upvalue: u32 },
    SetUpval { src: u32, upvalue: u32 },
    GetTableInt { dst: u32, table: u32, index: u32 },
    SetTableInt { table: u32, index: u32, value: u32 },
    Binary {
        dst: u32,
        lhs: NumericOperand,
        rhs: NumericOperand,
        op: NumericBinaryOp,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericIfElseCond {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NumericJmpLoopGuardBlock {
    pub pre_steps: Vec<NumericStep>,
    pub guard: NumericJmpLoopGuard,
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
pub(crate) enum CompiledTraceExecution {
    LoweredOnly,
    Native(NativeCompiledTrace),
}

impl CompiledTraceExecution {
    pub(crate) fn is_enterable(&self) -> bool {
        !matches!(self, Self::LoweredOnly)
    }

    pub(crate) fn family(&self) -> &'static str {
        match self {
            Self::LoweredOnly => "SummaryOnly",
            Self::Native(native) => match native {
                NativeCompiledTrace::Return { .. } => "NativeReturn",
                NativeCompiledTrace::Return0 { .. } => "NativeReturn0",
                NativeCompiledTrace::Return1 { .. } => "NativeReturn1",
                NativeCompiledTrace::LinearIntForLoop { .. } => "NativeLinearIntForLoop",
                NativeCompiledTrace::LinearIntJmpLoop { .. } => "NativeLinearIntJmpLoop",
                NativeCompiledTrace::NumericForLoop { .. } => "NativeNumericForLoop",
                NativeCompiledTrace::GuardedNumericForLoop { .. } => "NativeGuardedNumericForLoop",
                NativeCompiledTrace::NumericJmpLoop { .. } => "NativeNumericJmpLoop",
            },
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeTraceStatus {
    Fallback = 0,
    LoopExit = 1,
    SideExit = 2,
    Returned = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeTraceResult {
    pub status: NativeTraceStatus,
    pub hits: u32,
    pub exit_pc: u32,
    pub start_reg: u32,
    pub result_count: u32,
    pub exit_index: u32,
}

impl Default for NativeTraceResult {
    fn default() -> Self {
        Self::fallback(0)
    }
}

impl NativeTraceResult {
    pub(crate) const fn fallback(hits: u32) -> Self {
        Self {
            status: NativeTraceStatus::Fallback,
            hits,
            exit_pc: 0,
            start_reg: 0,
            result_count: 0,
            exit_index: 0,
        }
    }
}

pub(crate) type NativeTraceEntry = unsafe extern "C" fn(
    *mut LuaValue,
    usize,
    *const LuaValue,
    usize,
    *mut crate::LuaState,
    *const crate::gc::UpvaluePtr,
    *mut NativeTraceResult,
);

#[derive(Clone, Copy, Debug)]
pub(crate) enum NativeCompiledTrace {
    Return { entry: NativeTraceEntry },
    Return0 { entry: NativeTraceEntry },
    Return1 { entry: NativeTraceEntry },
    LinearIntForLoop { entry: NativeTraceEntry },
    LinearIntJmpLoop { entry: NativeTraceEntry },
    NumericForLoop { entry: NativeTraceEntry },
    GuardedNumericForLoop { entry: NativeTraceEntry },
    NumericJmpLoop { entry: NativeTraceEntry },
}

impl PartialEq for NativeCompiledTrace {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Return { entry: lhs }, Self::Return { entry: rhs }) => {
                std::ptr::fn_addr_eq(*lhs, *rhs)
            }
            (Self::Return0 { entry: lhs }, Self::Return0 { entry: rhs }) => {
                std::ptr::fn_addr_eq(*lhs, *rhs)
            }
            (Self::Return1 { entry: lhs }, Self::Return1 { entry: rhs }) => {
                std::ptr::fn_addr_eq(*lhs, *rhs)
            }
            (
                Self::LinearIntForLoop { entry: lhs },
                Self::LinearIntForLoop { entry: rhs },
            ) => std::ptr::fn_addr_eq(*lhs, *rhs),
            (
                Self::LinearIntJmpLoop {
                    entry: lhs,
                },
                Self::LinearIntJmpLoop {
                    entry: rhs,
                },
            ) => std::ptr::fn_addr_eq(*lhs, *rhs),
            (Self::NumericForLoop { entry: lhs }, Self::NumericForLoop { entry: rhs }) => {
                std::ptr::fn_addr_eq(*lhs, *rhs)
            }
            (
                Self::GuardedNumericForLoop { entry: lhs },
                Self::GuardedNumericForLoop { entry: rhs },
            ) => std::ptr::fn_addr_eq(*lhs, *rhs),
            (Self::NumericJmpLoop { entry: lhs }, Self::NumericJmpLoop { entry: rhs }) => {
                std::ptr::fn_addr_eq(*lhs, *rhs)
            }
            _ => false,
        }
    }
}

impl Eq for NativeCompiledTrace {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompiledTraceExit {
    pub exit_index: u16,
    pub exit_pc: u32,
    pub resume_pc: u32,
    pub is_loop_backedge: bool,
    pub restore_summary: DeoptRestoreSummary,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NativeLoweringProfile {
    pub guard_steps: u32,
    pub linear_guard_steps: u32,
    pub numeric_int_compare_guard_steps: u32,
    pub numeric_reg_compare_guard_steps: u32,
    pub truthy_guard_steps: u32,
    pub arithmetic_helper_steps: u32,
    pub table_helper_steps: u32,
    pub upvalue_helper_steps: u32,
    pub shift_helper_steps: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompiledTrace {
    pub root_pc: u32,
    pub loop_tail_pc: u32,
    pub steps: Vec<CompiledTraceStepKind>,
    exits: Vec<CompiledTraceExit>,
    summary: HelperPlanDispatchSummary,
    execution: CompiledTraceExecution,
    native_profile: Option<NativeLoweringProfile>,
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
        let execution = CompiledTraceExecution::LoweredOnly;
        Self::from_artifact_helper_plan_with_execution(
            artifact,
            ir,
            lowered_trace,
            helper_plan,
            execution,
            None,
        )
    }

    pub(super) fn from_artifact_helper_plan_with_execution(
        _artifact: &TraceArtifact,
        _ir: &TraceIr,
        lowered_trace: &LoweredTrace,
        helper_plan: &HelperPlan,
        execution: CompiledTraceExecution,
        native_profile: Option<NativeLoweringProfile>,
    ) -> Option<Self> {
        let mut steps = Vec::with_capacity(helper_plan.steps.len());
        let mut has_helper_call = false;
        let mut helper_plan_summary = HelperPlanDispatchSummary::default();

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
            helper_plan_summary.steps_executed = helper_plan_summary.steps_executed.saturating_add(1);
            match kind {
                CompiledTraceStepKind::Call => {
                    helper_plan_summary.call_steps = helper_plan_summary.call_steps.saturating_add(1);
                }
                CompiledTraceStepKind::MetamethodFallback => {
                    helper_plan_summary.metamethod_steps = helper_plan_summary
                        .metamethod_steps
                        .saturating_add(1);
                }
                CompiledTraceStepKind::Guard => {
                    helper_plan_summary.guards_observed = helper_plan_summary
                        .guards_observed
                        .saturating_add(1);
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

        let recognized_lowered = matches!(execution, CompiledTraceExecution::LoweredOnly)
            && native_profile.is_some();
        if !has_helper_call && !execution.is_enterable() && !recognized_lowered {
            return None;
        }

        let summary = execution_summary_for_dispatch(execution.clone(), native_profile, helper_plan_summary);

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
            execution,
            native_profile,
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

    pub(crate) fn is_enterable(&self) -> bool {
        self.execution.is_enterable()
    }

    pub(crate) fn executor_family(&self) -> &'static str {
        self.execution.family()
    }

    pub(crate) fn execution(&self) -> CompiledTraceExecution {
        self.execution.clone()
    }

    pub(crate) fn native_profile(&self) -> Option<NativeLoweringProfile> {
        self.native_profile
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
pub(super) fn synthetic_artifact_for_ir(ir: &TraceIr) -> TraceArtifact {
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

impl LinearIntLoopGuard {
    pub(crate) fn is_head(self) -> bool {
        matches!(self, Self::HeadRegReg { .. } | Self::HeadRegImm { .. })
    }

    pub(crate) fn is_tail(self) -> bool {
        matches!(self, Self::TailRegReg { .. } | Self::TailRegImm { .. })
    }

    pub(crate) fn exit_pc(self) -> u32 {
        match self {
            Self::HeadRegReg { exit_pc, .. }
            | Self::HeadRegImm { exit_pc, .. }
            | Self::TailRegReg { exit_pc, .. }
            | Self::TailRegImm { exit_pc, .. } => exit_pc,
        }
    }
}

fn execution_summary_for_dispatch(
    execution: CompiledTraceExecution,
    native_profile: Option<NativeLoweringProfile>,
    helper_plan_summary: HelperPlanDispatchSummary,
) -> HelperPlanDispatchSummary {
    match execution {
        CompiledTraceExecution::LoweredOnly => helper_plan_summary,
        CompiledTraceExecution::Native(_) => native_dispatch_summary(native_profile),
    }
}

fn native_dispatch_summary(
    native_profile: Option<NativeLoweringProfile>,
) -> HelperPlanDispatchSummary {
    let Some(profile) = native_profile else {
        return HelperPlanDispatchSummary::default();
    };

    let steps_executed = profile
        .arithmetic_helper_steps
        .saturating_add(profile.table_helper_steps)
        .saturating_add(profile.upvalue_helper_steps)
        .saturating_add(profile.shift_helper_steps);

    HelperPlanDispatchSummary {
        steps_executed,
        guards_observed: 0,
        call_steps: 0,
        metamethod_steps: 0,
    }
}