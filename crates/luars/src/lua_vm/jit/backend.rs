use crate::{Instruction, lua_value::LuaProto};

use super::helper_plan::{HelperPlan, HelperPlanDispatchSummary, HelperPlanStep};
use super::ir::{TraceIr, TraceIrGuardKind, TraceIrInst, TraceIrInstKind};
use super::trace_recorder::TraceArtifact;

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
    LoadI { dst: u32, imm: i32 },
    LoadF { dst: u32, imm: i32 },
    Binary {
        dst: u32,
        lhs: NumericOperand,
        rhs: NumericOperand,
        op: NumericBinaryOp,
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
        cond_reg: u32,
        cond_imm: i32,
        then_steps: Vec<NumericStep>,
        else_steps: Vec<NumericStep>,
        then_on_equal: bool,
    },
    LinearIntJmpLoop {
        steps: Vec<LinearIntStep>,
        guard: LinearIntLoopGuard,
    },
    NumericForGetTableAdd {
        loop_reg: u32,
        table_reg: u32,
        index_reg: u32,
        value_reg: u32,
        acc_reg: u32,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompiledTrace {
    pub root_pc: u32,
    pub loop_tail_pc: u32,
    pub steps: Vec<CompiledTraceStepKind>,
    summary: HelperPlanDispatchSummary,
    executor: CompiledTraceExecutor,
}

impl CompiledTrace {
    #[cfg(test)]
    pub(crate) fn from_helper_plan(ir: &TraceIr, helper_plan: &HelperPlan) -> Option<Self> {
        let artifact = synthetic_artifact_for_ir(ir);
        Self::from_artifact_helper_plan(&artifact, ir, helper_plan)
    }

    fn from_artifact_helper_plan(
        artifact: &TraceArtifact,
        ir: &TraceIr,
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

        let executor = compile_executor(artifact, ir);

        if !has_helper_call && matches!(executor, CompiledTraceExecutor::SummaryOnly) {
            return None;
        }

        Some(Self {
            root_pc: helper_plan.root_pc,
            loop_tail_pc: helper_plan.loop_tail_pc,
            steps,
            summary,
            executor,
        })
    }

    pub(crate) fn execute(&self) -> HelperPlanDispatchSummary {
        self.summary
    }

    pub(crate) fn summary(&self) -> HelperPlanDispatchSummary {
        self.summary
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
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome {
        match CompiledTrace::from_artifact_helper_plan(artifact, ir, helper_plan) {
            Some(compiled_trace) => BackendCompileOutcome::Compiled(compiled_trace),
            None => BackendCompileOutcome::NotYetSupported,
        }
    }
}

#[cfg(test)]
impl NullTraceBackend {
    fn compile(&mut self, ir: &TraceIr, helper_plan: &HelperPlan) -> BackendCompileOutcome {
        let artifact = synthetic_artifact_for_ir(ir);
        <Self as TraceBackend>::compile(self, &artifact, ir, helper_plan)
    }
}

#[cfg(test)]
fn synthetic_artifact_for_ir(ir: &TraceIr) -> TraceArtifact {
    TraceArtifact {
        seed: super::trace_recorder::TraceSeed {
            start_pc: ir.root_pc,
            root_chunk_addr: 0,
            instruction_budget: ir.insts.len().min(u16::MAX as usize) as u16,
        },
        ops: ir
            .insts
            .iter()
            .map(|inst| super::trace_recorder::TraceOp {
                pc: inst.pc,
                instruction: Instruction::from_u32(inst.raw_instruction),
                opcode: inst.opcode,
            })
            .collect(),
        exits: ir
            .guards
            .iter()
            .map(|guard| super::trace_recorder::TraceExit {
                guard_pc: guard.guard_pc,
                branch_pc: guard.branch_pc,
                exit_pc: guard.exit_pc,
                taken_on_trace: guard.taken_on_trace,
                kind: super::trace_recorder::TraceExitKind::GuardExit,
            })
            .collect(),
        loop_tail_pc: ir.loop_tail_pc,
    }
}

fn compile_executor(artifact: &TraceArtifact, ir: &TraceIr) -> CompiledTraceExecutor {
    if let Some(executor) = compile_numeric_for_gettable_add(ir) {
        return executor;
    }

    if let Some(executor) = compile_linear_int_forloop(ir) {
        return executor;
    }

    if let Some(executor) = compile_guarded_numeric_ifelse_forloop(artifact, ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_ifelse_forloop(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_forloop(ir) {
        return executor;
    }

    if let Some(executor) = compile_linear_int_jmp_loop(ir) {
        return executor;
    }

    if let Some(executor) = compile_generic_for_builtin_add(ir) {
        return executor;
    }

    if let Some(executor) = compile_next_while_builtin_add(ir) {
        return executor;
    }

    CompiledTraceExecutor::SummaryOnly
}

fn compile_linear_int_forloop(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() < 2 {
        return None;
    }

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let loop_reg = loop_inst.get_a();
    let steps = compile_linear_int_steps(&ir.insts[..ir.insts.len() - 1])?;

    Some(CompiledTraceExecutor::LinearIntForLoop { loop_reg, steps })
}

fn compile_numeric_forloop(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() < 2 {
        return None;
    }

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let loop_reg = loop_inst.get_a();
    let steps = compile_numeric_steps(&ir.insts[..ir.insts.len() - 1])?;

    Some(CompiledTraceExecutor::NumericForLoop { loop_reg, steps })
}

fn compile_numeric_ifelse_forloop(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() < 6 {
        return None;
    }

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let cond_index = ir.insts.iter().position(|inst| inst.opcode == crate::OpCode::EqI)?;
    if cond_index + 4 >= ir.insts.len() - 1 {
        return None;
    }

    let else_jump = &ir.insts[cond_index + 1];
    if else_jump.opcode != crate::OpCode::Jmp {
        return None;
    }
    let else_jump_inst = Instruction::from_u32(else_jump.raw_instruction);
    if else_jump_inst.get_sj() <= 0 {
        return None;
    }

    let merge_jump_index = cond_index + 1 + else_jump_inst.get_sj() as usize;
    if merge_jump_index <= cond_index + 1 || merge_jump_index >= ir.insts.len() - 1 {
        return None;
    }
    let merge_jump = &ir.insts[merge_jump_index];
    if merge_jump.opcode != crate::OpCode::Jmp {
        return None;
    }
    let merge_jump_inst = Instruction::from_u32(merge_jump.raw_instruction);
    if merge_jump_inst.get_sj() <= 0 {
        return None;
    }

    let pre_steps = compile_numeric_steps(&ir.insts[..cond_index])?;
    let then_steps = compile_numeric_steps(&ir.insts[cond_index + 2..merge_jump_index])?;
    let else_steps = compile_numeric_steps(&ir.insts[merge_jump_index + 1..ir.insts.len() - 1])?;
    if then_steps.is_empty() || else_steps.is_empty() {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let cond_inst = Instruction::from_u32(ir.insts[cond_index].raw_instruction);

    Some(CompiledTraceExecutor::NumericIfElseForLoop {
        loop_reg: loop_inst.get_a(),
        pre_steps,
        cond_reg: cond_inst.get_a(),
        cond_imm: cond_inst.get_sb(),
        then_steps,
        else_steps,
        then_on_equal: !cond_inst.get_k(),
    })
}

fn compile_guarded_numeric_ifelse_forloop(
    artifact: &TraceArtifact,
    ir: &TraceIr,
) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() < 5 || ir.guards.len() != 1 {
        return None;
    }

    let guard = ir.guards[0];
    if guard.kind != TraceIrGuardKind::SideExit || guard.taken_on_trace {
        return None;
    }

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let cond_index = ir.insts.iter().position(|inst| inst.opcode == crate::OpCode::EqI)?;
    if cond_index + 3 >= ir.insts.len() {
        return None;
    }

    let branch_inst = &ir.insts[cond_index + 1];
    if branch_inst.opcode != crate::OpCode::Jmp {
        return None;
    }

    let merge_jump_index = ir.insts.len() - 2;
    let merge_jump = &ir.insts[merge_jump_index];
    if merge_jump.opcode != crate::OpCode::Jmp {
        return None;
    }

    let pre_steps = compile_numeric_steps(&ir.insts[..cond_index])?;
    let then_steps = compile_numeric_steps(&ir.insts[cond_index + 2..merge_jump_index])?;
    if then_steps.is_empty() {
        return None;
    }

    let chunk = unsafe { (artifact.seed.root_chunk_addr as *const LuaProto).as_ref() }?;
    let merge_inst = Instruction::from_u32(merge_jump.raw_instruction);
    let merge_target_pc = ((merge_jump.pc + 1) as i64 + merge_inst.get_sj() as i64) as u32;
    if guard.exit_pc >= merge_target_pc {
        return None;
    }

    let else_steps = compile_numeric_steps_from_chunk(chunk, guard.exit_pc, merge_target_pc)?;
    if else_steps.is_empty() {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let cond_inst = Instruction::from_u32(ir.insts[cond_index].raw_instruction);
    Some(CompiledTraceExecutor::NumericIfElseForLoop {
        loop_reg: loop_inst.get_a(),
        pre_steps,
        cond_reg: cond_inst.get_a(),
        cond_imm: cond_inst.get_sb(),
        then_steps,
        else_steps,
        then_on_equal: !cond_inst.get_k(),
    })
}

fn compile_linear_int_jmp_loop(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() < 3 || ir.guards.len() != 1 {
        return None;
    }

    let backedge = ir.insts.last()?;
    if backedge.opcode != crate::OpCode::Jmp {
        return None;
    }
    let backedge_inst = Instruction::from_u32(backedge.raw_instruction);
    if backedge_inst.get_sj() >= 0 {
        return None;
    }

    let guard = ir.guards[0];

    if !guard.taken_on_trace {
        if ir.insts.len() < 4 || ir.insts[1].opcode != crate::OpCode::Jmp {
            return None;
        }
        let head = &ir.insts[0];
        let branch = &ir.insts[1];
        let loop_guard = compile_linear_int_guard(head, false, guard.exit_pc)?;
        let branch_inst = Instruction::from_u32(branch.raw_instruction);
        if branch_inst.get_sj() <= 0 {
            return None;
        }
        let steps = compile_linear_int_steps(&ir.insts[2..ir.insts.len() - 1])?;
        return Some(CompiledTraceExecutor::LinearIntJmpLoop {
            steps,
            guard: loop_guard,
        });
    }

    let head_len = ir.insts.len() - 2;
    let tail_guard_inst = &ir.insts[ir.insts.len() - 2];
    let steps = compile_linear_int_steps(&ir.insts[..head_len])?;
    let loop_guard = compile_linear_int_guard(tail_guard_inst, true, guard.exit_pc)?;
    Some(CompiledTraceExecutor::LinearIntJmpLoop {
        steps,
        guard: loop_guard,
    })
}

fn compile_linear_int_steps(insts: &[super::ir::TraceIrInst]) -> Option<Vec<LinearIntStep>> {
    let mut steps = Vec::with_capacity(insts.len());
    let mut index = 0usize;

    while index < insts.len() {
        let inst = &insts[index];
        let raw = Instruction::from_u32(inst.raw_instruction);
        let step = match inst.opcode {
            crate::OpCode::Move => LinearIntStep::Move {
                dst: raw.get_a(),
                src: raw.get_b(),
            },
            crate::OpCode::LoadI => LinearIntStep::LoadI {
                dst: raw.get_a(),
                imm: raw.get_sbx(),
            },
            crate::OpCode::Add if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                LinearIntStep::Add {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::AddI => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinI
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_sb() != raw.get_sc().unsigned_abs() as i32 {
                        return None;
                    }
                    index += 1;
                }
                LinearIntStep::AddI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: raw.get_sc(),
                }
            }
            crate::OpCode::Sub if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                LinearIntStep::Sub {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::Mul if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                LinearIntStep::Mul {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::MmBin | crate::OpCode::MmBinI => return None,
            _ => return None,
        };
        steps.push(step);
        index += 1;
    }

    Some(steps)
}

fn compile_numeric_steps(insts: &[super::ir::TraceIrInst]) -> Option<Vec<NumericStep>> {
    let mut steps = Vec::with_capacity(insts.len());
    let mut index = 0usize;

    while index < insts.len() {
        let inst = &insts[index];
        let raw = Instruction::from_u32(inst.raw_instruction);
        let step = match inst.opcode {
            crate::OpCode::Move => NumericStep::Move {
                dst: raw.get_a(),
                src: raw.get_b(),
            },
            crate::OpCode::LoadI => NumericStep::LoadI {
                dst: raw.get_a(),
                imm: raw.get_sbx(),
            },
            crate::OpCode::LoadF => NumericStep::LoadF {
                dst: raw.get_a(),
                imm: raw.get_sbx(),
            },
            crate::OpCode::Add if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::AddI => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinI
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_sb() != raw.get_sc().unsigned_abs() as i32 {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::ImmI(raw.get_sc()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::AddK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::Sub if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Sub,
                }
            }
            crate::OpCode::SubK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Sub,
                }
            }
            crate::OpCode::Mul if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Mul,
                }
            }
            crate::OpCode::MulK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Mul,
                }
            }
            crate::OpCode::Div if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Div,
                }
            }
            crate::OpCode::DivK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Div,
                }
            }
            crate::OpCode::IDiv if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::IDiv,
                }
            }
            crate::OpCode::IDivK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::IDiv,
                }
            }
            crate::OpCode::Mod if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Mod,
                }
            }
            crate::OpCode::ModK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Mod,
                }
            }
            crate::OpCode::Pow if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Pow,
                }
            }
            crate::OpCode::PowK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Pow,
                }
            }
            crate::OpCode::BAnd if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BAnd,
                }
            }
            crate::OpCode::BAndK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::BAnd,
                }
            }
            crate::OpCode::BOr if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BOr,
                }
            }
            crate::OpCode::BOrK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::BOr,
                }
            }
            crate::OpCode::BXor if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BXor,
                }
            }
            crate::OpCode::BXorK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::BXor,
                }
            }
            crate::OpCode::Shl if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Shl,
                }
            }
            crate::OpCode::Shr if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Shr,
                }
            }
            crate::OpCode::ShlI => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinI
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_sb() != raw.get_sc().unsigned_abs() as i32 {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::ImmI(raw.get_sc()),
                    rhs: NumericOperand::Reg(raw.get_b()),
                    op: NumericBinaryOp::Shl,
                }
            }
            crate::OpCode::ShrI => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinI
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_sb() != raw.get_sc().unsigned_abs() as i32 {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::ImmI(raw.get_sc()),
                    op: NumericBinaryOp::Shr,
                }
            }
            crate::OpCode::MmBin | crate::OpCode::MmBinI | crate::OpCode::MmBinK => return None,
            _ => return None,
        };
        steps.push(step);
        index += 1;
    }

    Some(steps)
}

fn compile_linear_int_guard(
    inst: &super::ir::TraceIrInst,
    tail: bool,
    exit_pc: u32,
) -> Option<LinearIntLoopGuard> {
    let raw = Instruction::from_u32(inst.raw_instruction);
    let continue_when = !raw.get_k();

    match inst.opcode {
        crate::OpCode::Lt | crate::OpCode::Le => {
            let op = match inst.opcode {
                crate::OpCode::Lt => LinearIntGuardOp::Lt,
                crate::OpCode::Le => LinearIntGuardOp::Le,
                _ => unreachable!(),
            };
            if tail {
                Some(LinearIntLoopGuard::TailRegReg {
                    op,
                    lhs: raw.get_a(),
                    rhs: raw.get_b(),
                    continue_when,
                    exit_pc,
                })
            } else {
                Some(LinearIntLoopGuard::HeadRegReg {
                    op,
                    lhs: raw.get_a(),
                    rhs: raw.get_b(),
                    continue_when,
                    exit_pc,
                })
            }
        }
        crate::OpCode::EqI
        | crate::OpCode::LtI
        | crate::OpCode::LeI
        | crate::OpCode::GtI
        | crate::OpCode::GeI => {
            if raw.get_c() != 0 {
                return None;
            }

            let op = match inst.opcode {
                crate::OpCode::EqI => LinearIntGuardOp::Eq,
                crate::OpCode::LtI => LinearIntGuardOp::Lt,
                crate::OpCode::LeI => LinearIntGuardOp::Le,
                crate::OpCode::GtI => LinearIntGuardOp::Gt,
                crate::OpCode::GeI => LinearIntGuardOp::Ge,
                _ => unreachable!(),
            };

            if tail {
                Some(LinearIntLoopGuard::TailRegImm {
                    op,
                    reg: raw.get_a(),
                    imm: raw.get_sb(),
                    continue_when,
                    exit_pc,
                })
            } else {
                Some(LinearIntLoopGuard::HeadRegImm {
                    op,
                    reg: raw.get_a(),
                    imm: raw.get_sb(),
                    continue_when,
                    exit_pc,
                })
            }
        }
        _ => None,
    }
}

fn compile_numeric_for_gettable_add(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 4 {
        return None;
    }

    let get_table = &ir.insts[0];
    let add = &ir.insts[1];
    let mm_bin = &ir.insts[2];
    let loop_backedge = &ir.insts[3];

    if get_table.opcode != crate::OpCode::GetTable
        || add.opcode != crate::OpCode::Add
        || mm_bin.opcode != crate::OpCode::MmBin
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_inst = Instruction::from_u32(get_table.raw_instruction);
    let add_inst = Instruction::from_u32(add.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let value_reg = get_inst.get_a();
    let table_reg = get_inst.get_b();
    let index_reg = get_inst.get_c();
    let acc_reg = add_inst.get_a();

    if add_inst.get_b() != acc_reg || add_inst.get_c() != value_reg || add_inst.get_k() {
        return None;
    }

    let loop_reg = loop_inst.get_a();
    if index_reg != loop_reg + 2 {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForGetTableAdd {
        loop_reg,
        table_reg,
        index_reg,
        value_reg,
        acc_reg,
    })
}

fn compile_generic_for_builtin_add(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() != 4 || ir.guards.len() != 1 {
        return None;
    }

    let add = &ir.insts[0];
    let mm_bin = &ir.insts[1];
    let tfor_call = &ir.insts[2];
    let tfor_loop = &ir.insts[3];
    let guard = ir.guards[0];

    if add.opcode != crate::OpCode::Add
        || mm_bin.opcode != crate::OpCode::MmBin
        || tfor_call.opcode != crate::OpCode::TForCall
        || tfor_loop.opcode != crate::OpCode::TForLoop
        || guard.kind != TraceIrGuardKind::LoopBackedgeGuard
        || guard.guard_pc != tfor_loop.pc
        || !guard.taken_on_trace
    {
        return None;
    }

    let add_inst = Instruction::from_u32(add.raw_instruction);
    let mm_bin_inst = Instruction::from_u32(mm_bin.raw_instruction);
    let tfor_call_inst = Instruction::from_u32(tfor_call.raw_instruction);
    let tfor_loop_inst = Instruction::from_u32(tfor_loop.raw_instruction);

    let acc_reg = add_inst.get_a();
    if add_inst.get_b() != acc_reg || add_inst.get_k() {
        return None;
    }

    let tfor_reg = tfor_call_inst.get_a();
    if tfor_call_inst.get_c() != 2 || tfor_loop_inst.get_a() != tfor_reg {
        return None;
    }

    let value_reg = tfor_reg + 4;
    if add_inst.get_c() != value_reg
        || mm_bin_inst.get_a() != acc_reg
        || mm_bin_inst.get_b() != value_reg
    {
        return None;
    }

    Some(CompiledTraceExecutor::GenericForBuiltinAdd {
        tfor_reg,
        value_reg,
        acc_reg,
    })
}

fn compile_next_while_builtin_add(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() != 11 || ir.guards.len() != 1 {
        return None;
    }

    let test = &ir.insts[0];
    let exit_jmp = &ir.insts[1];
    let add = &ir.insts[2];
    let mm_bin = &ir.insts[3];
    let get_tabup = &ir.insts[4];
    let move_state = &ir.insts[5];
    let move_key_arg = &ir.insts[6];
    let call = &ir.insts[7];
    let move_value = &ir.insts[8];
    let move_key = &ir.insts[9];
    let backedge_jmp = &ir.insts[10];
    let guard = ir.guards[0];

    if test.opcode != crate::OpCode::Test
        || exit_jmp.opcode != crate::OpCode::Jmp
        || add.opcode != crate::OpCode::Add
        || mm_bin.opcode != crate::OpCode::MmBin
        || get_tabup.opcode != crate::OpCode::GetTabUp
        || move_state.opcode != crate::OpCode::Move
        || move_key_arg.opcode != crate::OpCode::Move
        || call.opcode != crate::OpCode::Call
        || move_value.opcode != crate::OpCode::Move
        || move_key.opcode != crate::OpCode::Move
        || backedge_jmp.opcode != crate::OpCode::Jmp
        || guard.kind != TraceIrGuardKind::SideExit
        || guard.guard_pc != test.pc
        || guard.taken_on_trace
    {
        return None;
    }

    let test_inst = Instruction::from_u32(test.raw_instruction);
    let add_inst = Instruction::from_u32(add.raw_instruction);
    let mm_bin_inst = Instruction::from_u32(mm_bin.raw_instruction);
    let get_tabup_inst = Instruction::from_u32(get_tabup.raw_instruction);
    let move_state_inst = Instruction::from_u32(move_state.raw_instruction);
    let move_key_arg_inst = Instruction::from_u32(move_key_arg.raw_instruction);
    let call_inst = Instruction::from_u32(call.raw_instruction);
    let move_value_inst = Instruction::from_u32(move_value.raw_instruction);
    let move_key_inst = Instruction::from_u32(move_key.raw_instruction);

    let key_reg = test_inst.get_a();
    let acc_reg = add_inst.get_a();
    let value_reg = add_inst.get_c();
    let call_base = get_tabup_inst.get_a();

    if add_inst.get_b() != acc_reg
        || add_inst.get_k()
        || mm_bin_inst.get_a() != acc_reg
        || mm_bin_inst.get_b() != value_reg
        || move_state_inst.get_a() != call_base + 1
        || move_key_arg_inst.get_a() != call_base + 2
        || move_key_arg_inst.get_b() != key_reg
        || call_inst.get_a() != call_base
        || call_inst.get_b() != 3
        || call_inst.get_c() != 3
        || move_value_inst.get_a() != value_reg
        || move_value_inst.get_b() != call_base + 1
        || move_key_inst.get_a() != key_reg
        || move_key_inst.get_b() != call_base
    {
        return None;
    }

    Some(CompiledTraceExecutor::NextWhileBuiltinAdd {
        key_reg,
        value_reg,
        acc_reg,
        table_reg: move_state_inst.get_b(),
        env_upvalue: get_tabup_inst.get_b(),
        key_const: get_tabup_inst.get_c(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        BackendCompileOutcome, CompiledTrace, CompiledTraceExecutor, CompiledTraceStepKind,
        LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep, NullTraceBackend, NumericBinaryOp,
        NumericOperand, NumericStep, TraceBackend,
    };
    use crate::Instruction;
    use crate::lua_vm::jit::helper_plan::{
        HelperPlan, HelperPlanDispatchSummary, HelperPlanStep,
    };
    use crate::lua_vm::jit::ir::{TraceIr, TraceIrInst, TraceIrOperand};
    use crate::OpCode;

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
            summary: HelperPlanDispatchSummary::default(),
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
            summary: HelperPlanDispatchSummary {
                steps_executed: 3,
                guards_observed: 0,
                call_steps: 1,
                metamethod_steps: 1,
            },
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

    #[test]
    fn backend_marks_numeric_for_gettable_add_as_executable() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 10,
            loop_tail_pc: 13,
            insts: vec![
                TraceIrInst {
                    pc: 10,
                    opcode: OpCode::GetTable,
                    raw_instruction: Instruction::create_abc(OpCode::GetTable, 13, 1, 12).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                    reads: vec![TraceIrOperand::Register(1), TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::Register(13)],
                },
                TraceIrInst {
                    pc: 11,
                    opcode: OpCode::Add,
                    raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 13, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(13)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 12,
                    opcode: OpCode::MmBin,
                    raw_instruction: Instruction::create_abc(OpCode::MmBin, 5, 13, 6).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 13,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 10, 4).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(10)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 10,
            loop_tail_pc: 13,
            steps: vec![
                HelperPlanStep::TableAccess {
                    reads: vec![TraceIrOperand::Register(1), TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::Register(13)],
                },
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(13)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(5)],
                },
                HelperPlanStep::LoopBackedge {
                    reads: vec![TraceIrOperand::JumpTarget(10)],
                    writes: Vec::new(),
                },
            ],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 4,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 1,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NumericForGetTableAdd {
                        loop_reg: 10,
                        table_reg: 1,
                        index_reg: 12,
                        value_reg: 13,
                        acc_reg: 5,
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_pure_linear_int_forloop_without_helper_calls() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 20,
            loop_tail_pc: 22,
            insts: vec![
                TraceIrInst {
                    pc: 20,
                    opcode: OpCode::Move,
                    raw_instruction: Instruction::create_abc(OpCode::Move, 5, 7, 0).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                    reads: vec![TraceIrOperand::Register(7)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 21,
                    opcode: OpCode::Add,
                    raw_instruction: Instruction::create_abck(OpCode::Add, 6, 6, 12, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(6), TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::Register(6)],
                },
                TraceIrInst {
                    pc: 22,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 10, 3).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(20)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 20,
            loop_tail_pc: 22,
            steps: vec![
                HelperPlanStep::LoadMove {
                    reads: vec![TraceIrOperand::Register(7)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(6), TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::Register(6)],
                },
                HelperPlanStep::LoopBackedge {
                    reads: vec![TraceIrOperand::JumpTarget(20)],
                    writes: Vec::new(),
                },
            ],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 3,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 0,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::LinearIntForLoop {
                        loop_reg: 10,
                        steps: vec![
                            LinearIntStep::Move { dst: 5, src: 7 },
                            LinearIntStep::Add {
                                dst: 6,
                                lhs: 6,
                                rhs: 12,
                            },
                        ],
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_absorbs_mmbin_in_linear_int_forloop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 30,
            loop_tail_pc: 32,
            insts: vec![
                TraceIrInst {
                    pc: 30,
                    opcode: OpCode::Add,
                    raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 12, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 31,
                    opcode: OpCode::MmBin,
                    raw_instruction: Instruction::create_abc(OpCode::MmBin, 5, 12, 6).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(12)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 32,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 10, 3).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(30)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 30,
            loop_tail_pc: 32,
            steps: vec![
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(12)],
                },
                HelperPlanStep::LoopBackedge {
                    reads: vec![TraceIrOperand::JumpTarget(30)],
                    writes: Vec::new(),
                },
            ],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 3,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 1,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::LinearIntForLoop {
                        loop_reg: 10,
                        steps: vec![LinearIntStep::Add {
                            dst: 5,
                            lhs: 5,
                            rhs: 12,
                        }],
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_numeric_float_forloop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 46,
            loop_tail_pc: 49,
            insts: vec![
                TraceIrInst {
                    pc: 46,
                    opcode: OpCode::LoadF,
                    raw_instruction: Instruction::create_asbx(OpCode::LoadF, 4, 1).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                    reads: vec![TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                TraceIrInst {
                    pc: 47,
                    opcode: OpCode::MulK,
                    raw_instruction: Instruction::create_abc(OpCode::MulK, 4, 4, 10).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(10)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                TraceIrInst {
                    pc: 48,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 10, 8, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(10)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 49,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 3).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(46)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 46,
            loop_tail_pc: 49,
            steps: vec![
                HelperPlanStep::LoadMove {
                    reads: vec![TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(10)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(10)],
                },
                HelperPlanStep::LoopBackedge {
                    reads: vec![TraceIrOperand::JumpTarget(46)],
                    writes: Vec::new(),
                },
            ],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 4,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 1,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NumericForLoop {
                        loop_reg: 5,
                        steps: vec![
                            NumericStep::LoadF { dst: 4, imm: 1 },
                            NumericStep::Binary {
                                dst: 4,
                                lhs: NumericOperand::Reg(4),
                                rhs: NumericOperand::Const(10),
                                op: NumericBinaryOp::Mul,
                            },
                        ],
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_numeric_mixed_forloop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 77,
            loop_tail_pc: 84,
            insts: vec![
                TraceIrInst {
                    pc: 77,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 10, 132).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(10), TraceIrOperand::SignedImmediate(5)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 78,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 10, 132, 6, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(10), TraceIrOperand::SignedImmediate(5)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 79,
                    opcode: OpCode::MulK,
                    raw_instruction: Instruction::create_abc(OpCode::MulK, 6, 5, 12).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::ConstantIndex(12)],
                    writes: vec![TraceIrOperand::Register(6)],
                },
                TraceIrInst {
                    pc: 80,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 5, 12, 8, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::ConstantIndex(12)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 81,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 7, 6, 124).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(6), TraceIrOperand::SignedImmediate(-3)],
                    writes: vec![TraceIrOperand::Register(7)],
                },
                TraceIrInst {
                    pc: 82,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 6, 130, 7, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(6), TraceIrOperand::SignedImmediate(-3)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 84,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 8, 7).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(77)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 77,
            loop_tail_pc: 84,
            steps: vec![
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(10), TraceIrOperand::SignedImmediate(5)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(10), TraceIrOperand::SignedImmediate(5)],
                },
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::ConstantIndex(12)],
                    writes: vec![TraceIrOperand::Register(6)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::ConstantIndex(12)],
                },
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(6), TraceIrOperand::SignedImmediate(-3)],
                    writes: vec![TraceIrOperand::Register(7)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(6), TraceIrOperand::SignedImmediate(-3)],
                },
                HelperPlanStep::LoopBackedge {
                    reads: vec![TraceIrOperand::JumpTarget(77)],
                    writes: Vec::new(),
                },
            ],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 7,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 3,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NumericForLoop {
                        loop_reg: 8,
                        steps: vec![
                            NumericStep::Binary {
                                dst: 5,
                                lhs: NumericOperand::Reg(10),
                                rhs: NumericOperand::ImmI(5),
                                op: NumericBinaryOp::Add,
                            },
                            NumericStep::Binary {
                                dst: 6,
                                lhs: NumericOperand::Reg(5),
                                rhs: NumericOperand::Const(12),
                                op: NumericBinaryOp::Mul,
                            },
                            NumericStep::Binary {
                                dst: 7,
                                lhs: NumericOperand::Reg(6),
                                rhs: NumericOperand::ImmI(-3),
                                op: NumericBinaryOp::Add,
                            },
                        ],
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_numeric_div_forloop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 69,
            loop_tail_pc: 76,
            insts: vec![
                TraceIrInst {
                    pc: 70,
                    opcode: OpCode::MulK,
                    raw_instruction: Instruction::create_abc(OpCode::MulK, 18, 17, 23).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(17), TraceIrOperand::ConstantIndex(23)],
                    writes: vec![TraceIrOperand::Register(18)],
                },
                TraceIrInst {
                    pc: 71,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 17, 23, 8, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(17), TraceIrOperand::ConstantIndex(23)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 72,
                    opcode: OpCode::AddK,
                    raw_instruction: Instruction::create_abc(OpCode::AddK, 18, 18, 24).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::ConstantIndex(24)],
                    writes: vec![TraceIrOperand::Register(18)],
                },
                TraceIrInst {
                    pc: 73,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 18, 24, 6, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::ConstantIndex(24)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 74,
                    opcode: OpCode::DivK,
                    raw_instruction: Instruction::create_abc(OpCode::DivK, 14, 18, 25).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::ConstantIndex(25)],
                    writes: vec![TraceIrOperand::Register(14)],
                },
                TraceIrInst {
                    pc: 75,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 18, 25, 11, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::ConstantIndex(25)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 76,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 15, 7).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(70)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 69,
            loop_tail_pc: 76,
            steps: vec![],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 7,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 3,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NumericForLoop {
                        loop_reg: 15,
                        steps: vec![
                            NumericStep::Binary {
                                dst: 18,
                                lhs: NumericOperand::Reg(17),
                                rhs: NumericOperand::Const(23),
                                op: NumericBinaryOp::Mul,
                            },
                            NumericStep::Binary {
                                dst: 18,
                                lhs: NumericOperand::Reg(18),
                                rhs: NumericOperand::Const(24),
                                op: NumericBinaryOp::Add,
                            },
                            NumericStep::Binary {
                                dst: 14,
                                lhs: NumericOperand::Reg(18),
                                rhs: NumericOperand::Const(25),
                                op: NumericBinaryOp::Div,
                            },
                        ],
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_numeric_pow_forloop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 351,
            loop_tail_pc: 358,
            insts: vec![
                TraceIrInst {
                    pc: 352,
                    opcode: OpCode::ModK,
                    raw_instruction: Instruction::create_abc(OpCode::ModK, 18, 17, 43).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(17), TraceIrOperand::ConstantIndex(43)],
                    writes: vec![TraceIrOperand::Register(18)],
                },
                TraceIrInst {
                    pc: 353,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 17, 43, 9, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(17), TraceIrOperand::ConstantIndex(43)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 354,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 18, 18, 128).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(18)],
                },
                TraceIrInst {
                    pc: 355,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 18, 128, 6, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::SignedImmediate(1)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 356,
                    opcode: OpCode::PowK,
                    raw_instruction: Instruction::create_abc(OpCode::PowK, 14, 18, 44).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::ConstantIndex(44)],
                    writes: vec![TraceIrOperand::Register(14)],
                },
                TraceIrInst {
                    pc: 357,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 18, 44, 10, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::ConstantIndex(44)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 358,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 15, 7).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(352)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 351,
            loop_tail_pc: 358,
            steps: vec![],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 7,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 3,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NumericForLoop {
                        loop_reg: 15,
                        steps: vec![
                            NumericStep::Binary {
                                dst: 18,
                                lhs: NumericOperand::Reg(17),
                                rhs: NumericOperand::Const(43),
                                op: NumericBinaryOp::Mod,
                            },
                            NumericStep::Binary {
                                dst: 18,
                                lhs: NumericOperand::Reg(18),
                                rhs: NumericOperand::ImmI(1),
                                op: NumericBinaryOp::Add,
                            },
                            NumericStep::Binary {
                                dst: 14,
                                lhs: NumericOperand::Reg(18),
                                rhs: NumericOperand::Const(44),
                                op: NumericBinaryOp::Pow,
                            },
                        ],
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_numeric_bitwise_forloop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 293,
            loop_tail_pc: 300,
            insts: vec![
                TraceIrInst {
                    pc: 294,
                    opcode: OpCode::BAndK,
                    raw_instruction: Instruction::create_abc(OpCode::BAndK, 18, 17, 39).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(17), TraceIrOperand::ConstantIndex(39)],
                    writes: vec![TraceIrOperand::Register(18)],
                },
                TraceIrInst {
                    pc: 295,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 17, 39, 13, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(17), TraceIrOperand::ConstantIndex(39)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 296,
                    opcode: OpCode::ShrI,
                    raw_instruction: Instruction::create_abc(OpCode::ShrI, 19, 17, 131).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(17), TraceIrOperand::SignedImmediate(4)],
                    writes: vec![TraceIrOperand::Register(19)],
                },
                TraceIrInst {
                    pc: 297,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 17, 131, 17, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(17), TraceIrOperand::SignedImmediate(4)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 298,
                    opcode: OpCode::BOr,
                    raw_instruction: Instruction::create_abck(OpCode::BOr, 12, 18, 19, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::Register(19)],
                    writes: vec![TraceIrOperand::Register(12)],
                },
                TraceIrInst {
                    pc: 299,
                    opcode: OpCode::MmBin,
                    raw_instruction: Instruction::create_abc(OpCode::MmBin, 18, 19, 14).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(18), TraceIrOperand::Register(19)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 300,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 15, 7).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(294)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 293,
            loop_tail_pc: 300,
            steps: vec![],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 7,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 3,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NumericForLoop {
                        loop_reg: 15,
                        steps: vec![
                            NumericStep::Binary {
                                dst: 18,
                                lhs: NumericOperand::Reg(17),
                                rhs: NumericOperand::Const(39),
                                op: NumericBinaryOp::BAnd,
                            },
                            NumericStep::Binary {
                                dst: 19,
                                lhs: NumericOperand::Reg(17),
                                rhs: NumericOperand::ImmI(4),
                                op: NumericBinaryOp::Shr,
                            },
                            NumericStep::Binary {
                                dst: 12,
                                lhs: NumericOperand::Reg(18),
                                rhs: NumericOperand::Reg(19),
                                op: NumericBinaryOp::BOr,
                            },
                        ],
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_numeric_ifelse_forloop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 17,
            loop_tail_pc: 26,
            insts: vec![
                TraceIrInst {
                    pc: 18,
                    opcode: OpCode::ModK,
                    raw_instruction: Instruction::create_abc(OpCode::ModK, 7, 4, 0).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                    writes: vec![TraceIrOperand::Register(7)],
                },
                TraceIrInst {
                    pc: 19,
                    opcode: OpCode::MmBinK,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 9, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 20,
                    opcode: OpCode::EqI,
                    raw_instruction: Instruction::create_abck(OpCode::EqI, 7, 127, 0, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                    reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(0)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 21,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                    reads: vec![TraceIrOperand::JumpTarget(25)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 22,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 23,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 24,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, 2).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                    reads: vec![TraceIrOperand::JumpTarget(27)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 25,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 126).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(-1)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 26,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 7, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 27,
                    opcode: OpCode::ForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::ForLoop, 1, 9).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(18)],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan {
            root_pc: 17,
            loop_tail_pc: 26,
            steps: vec![],
            guard_count: 0,
            summary: HelperPlanDispatchSummary {
                steps_executed: 10,
                guards_observed: 0,
                call_steps: 0,
                metamethod_steps: 3,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NumericIfElseForLoop {
                        loop_reg: 1,
                        pre_steps: vec![NumericStep::Binary {
                            dst: 7,
                            lhs: NumericOperand::Reg(4),
                            rhs: NumericOperand::Const(0),
                            op: NumericBinaryOp::Mod,
                        }],
                        cond_reg: 7,
                        cond_imm: 0,
                        then_steps: vec![NumericStep::Binary {
                            dst: 5,
                            lhs: NumericOperand::Reg(5),
                            rhs: NumericOperand::ImmI(1),
                            op: NumericBinaryOp::Add,
                        }],
                        else_steps: vec![NumericStep::Binary {
                            dst: 5,
                            lhs: NumericOperand::Reg(5),
                            rhs: NumericOperand::ImmI(-1),
                            op: NumericBinaryOp::Add,
                        }],
                        then_on_equal: true,
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_guarded_numeric_ifelse_forloop_from_artifact() {
        let mut backend = NullTraceBackend;
        let mut chunk = crate::lua_value::LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::ModK, 7, 4, 0));
        chunk.code.push(Instruction::create_abck(OpCode::MmBinK, 4, 0, 9, false));
        chunk.code.push(Instruction::create_abck(OpCode::EqI, 7, 127, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 3));
        chunk.code.push(Instruction::create_abc(OpCode::AddI, 5, 5, 128));
        chunk.code.push(Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 2));
        chunk.code.push(Instruction::create_abc(OpCode::AddI, 5, 5, 126));
        chunk.code.push(Instruction::create_abck(OpCode::MmBinI, 5, 128, 7, false));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 1, 9));

        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 9,
            insts: vec![
                TraceIrInst {
                    pc: 0,
                    opcode: OpCode::ModK,
                    raw_instruction: chunk.code[0].as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                    writes: vec![TraceIrOperand::Register(7)],
                },
                TraceIrInst {
                    pc: 1,
                    opcode: OpCode::MmBinK,
                    raw_instruction: chunk.code[1].as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 2,
                    opcode: OpCode::EqI,
                    raw_instruction: chunk.code[2].as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                    reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(0)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 3,
                    opcode: OpCode::Jmp,
                    raw_instruction: chunk.code[3].as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                    reads: vec![TraceIrOperand::JumpTarget(7)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 4,
                    opcode: OpCode::AddI,
                    raw_instruction: chunk.code[4].as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 5,
                    opcode: OpCode::MmBinI,
                    raw_instruction: chunk.code[5].as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 6,
                    opcode: OpCode::Jmp,
                    raw_instruction: chunk.code[6].as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                    reads: vec![TraceIrOperand::JumpTarget(9)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 9,
                    opcode: OpCode::ForLoop,
                    raw_instruction: chunk.code[9].as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(0)],
                    writes: Vec::new(),
                },
            ],
            guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 2,
                branch_pc: 3,
                exit_pc: 7,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            }],
        };
        let artifact = super::TraceArtifact {
            seed: super::super::trace_recorder::TraceSeed {
                start_pc: 0,
                root_chunk_addr: &chunk as *const _ as usize,
                instruction_budget: 10,
            },
            ops: ir
                .insts
                .iter()
                .map(|inst| super::super::trace_recorder::TraceOp {
                    pc: inst.pc,
                    instruction: Instruction::from_u32(inst.raw_instruction),
                    opcode: inst.opcode,
                })
                .collect(),
            exits: vec![super::super::trace_recorder::TraceExit {
                guard_pc: 2,
                branch_pc: 3,
                exit_pc: 7,
                taken_on_trace: false,
                kind: super::super::trace_recorder::TraceExitKind::GuardExit,
            }],
            loop_tail_pc: 9,
        };
        let helper_plan = HelperPlan {
            root_pc: 0,
            loop_tail_pc: 9,
            steps: vec![],
            guard_count: 1,
            summary: HelperPlanDispatchSummary {
                steps_executed: 8,
                guards_observed: 1,
                call_steps: 0,
                metamethod_steps: 2,
            },
        };

        match <NullTraceBackend as TraceBackend>::compile(&mut backend, &artifact, &ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NumericIfElseForLoop {
                        loop_reg: 1,
                        pre_steps: vec![NumericStep::Binary {
                            dst: 7,
                            lhs: NumericOperand::Reg(4),
                            rhs: NumericOperand::Const(0),
                            op: NumericBinaryOp::Mod,
                        }],
                        cond_reg: 7,
                        cond_imm: 0,
                        then_steps: vec![NumericStep::Binary {
                            dst: 5,
                            lhs: NumericOperand::Reg(5),
                            rhs: NumericOperand::ImmI(1),
                            op: NumericBinaryOp::Add,
                        }],
                        else_steps: vec![NumericStep::Binary {
                            dst: 5,
                            lhs: NumericOperand::Reg(5),
                            rhs: NumericOperand::ImmI(-1),
                            op: NumericBinaryOp::Add,
                        }],
                        then_on_equal: true,
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_head_guard_linear_int_jmp_loop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 49,
            loop_tail_pc: 55,
            insts: vec![
                TraceIrInst {
                    pc: 49,
                    opcode: OpCode::Lt,
                    raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 0, 0, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(0)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 50,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, 5).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                    reads: vec![TraceIrOperand::JumpTarget(56)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 51,
                    opcode: OpCode::Add,
                    raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 4, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(4)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 52,
                    opcode: OpCode::MmBin,
                    raw_instruction: Instruction::create_abc(OpCode::MmBin, 5, 4, 6).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(4)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 53,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 4, 4, 128).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                TraceIrInst {
                    pc: 54,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 6, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 55,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, -7).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(49)],
                    writes: Vec::new(),
                },
            ],
            guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 49,
                branch_pc: 50,
                exit_pc: 56,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            }],
        };
        let helper_plan = HelperPlan {
            root_pc: 49,
            loop_tail_pc: 55,
            steps: vec![],
            guard_count: 1,
            summary: HelperPlanDispatchSummary {
                steps_executed: 7,
                guards_observed: 1,
                call_steps: 0,
                metamethod_steps: 2,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::LinearIntJmpLoop {
                        steps: vec![
                            LinearIntStep::Add {
                                dst: 5,
                                lhs: 5,
                                rhs: 4,
                            },
                            LinearIntStep::AddI {
                                dst: 4,
                                src: 4,
                                imm: 1,
                            },
                        ],
                        guard: LinearIntLoopGuard::HeadRegReg {
                            op: LinearIntGuardOp::Lt,
                            lhs: 4,
                            rhs: 0,
                            continue_when: true,
                            exit_pc: 56,
                        },
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_tail_guard_linear_int_jmp_loop() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 78,
            loop_tail_pc: 83,
            insts: vec![
                TraceIrInst {
                    pc: 78,
                    opcode: OpCode::Add,
                    raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 4, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(4)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 79,
                    opcode: OpCode::MmBin,
                    raw_instruction: Instruction::create_abc(OpCode::MmBin, 5, 4, 6).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(4)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 80,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 4, 4, 128).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                TraceIrInst {
                    pc: 81,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 6, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 82,
                    opcode: OpCode::Le,
                    raw_instruction: Instruction::create_abck(OpCode::Le, 0, 4, 0, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                    reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(4)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 83,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, -6).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(78)],
                    writes: Vec::new(),
                },
            ],
            guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 82,
                branch_pc: 83,
                exit_pc: 84,
                taken_on_trace: true,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard,
            }],
        };
        let helper_plan = HelperPlan {
            root_pc: 78,
            loop_tail_pc: 83,
            steps: vec![],
            guard_count: 1,
            summary: HelperPlanDispatchSummary {
                steps_executed: 6,
                guards_observed: 1,
                call_steps: 0,
                metamethod_steps: 2,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::LinearIntJmpLoop {
                        steps: vec![
                            LinearIntStep::Add {
                                dst: 5,
                                lhs: 5,
                                rhs: 4,
                            },
                            LinearIntStep::AddI {
                                dst: 4,
                                src: 4,
                                imm: 1,
                            },
                        ],
                        guard: LinearIntLoopGuard::TailRegReg {
                            op: LinearIntGuardOp::Le,
                            lhs: 0,
                            rhs: 4,
                            continue_when: true,
                            exit_pc: 84,
                        },
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_head_guard_linear_int_jmp_loop_with_immediate_guard() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 49,
            loop_tail_pc: 54,
            insts: vec![
                TraceIrInst {
                    pc: 49,
                    opcode: OpCode::LtI,
                    raw_instruction: Instruction::create_abck(OpCode::LtI, 4, 137, 0, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(10)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 50,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, 4).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                    reads: vec![TraceIrOperand::JumpTarget(55)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 51,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 52,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 53,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 4, 4, 128).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                TraceIrInst {
                    pc: 54,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, -6).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(49)],
                    writes: Vec::new(),
                },
            ],
            guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 49,
                branch_pc: 50,
                exit_pc: 55,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            }],
        };
        let helper_plan = HelperPlan {
            root_pc: 49,
            loop_tail_pc: 54,
            steps: vec![],
            guard_count: 1,
            summary: HelperPlanDispatchSummary {
                steps_executed: 6,
                guards_observed: 1,
                call_steps: 0,
                metamethod_steps: 1,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::LinearIntJmpLoop {
                        steps: vec![
                            LinearIntStep::AddI {
                                dst: 5,
                                src: 5,
                                imm: 1,
                            },
                            LinearIntStep::AddI {
                                dst: 4,
                                src: 4,
                                imm: 1,
                            },
                        ],
                        guard: LinearIntLoopGuard::HeadRegImm {
                            op: LinearIntGuardOp::Lt,
                            reg: 4,
                            imm: 10,
                            continue_when: true,
                            exit_pc: 55,
                        },
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_compiles_tail_guard_linear_int_jmp_loop_with_immediate_guard() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 78,
            loop_tail_pc: 82,
            insts: vec![
                TraceIrInst {
                    pc: 78,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 79,
                    opcode: OpCode::AddI,
                    raw_instruction: Instruction::create_abc(OpCode::AddI, 4, 4, 128).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                    writes: vec![TraceIrOperand::Register(4)],
                },
                TraceIrInst {
                    pc: 80,
                    opcode: OpCode::MmBinI,
                    raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 6, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 81,
                    opcode: OpCode::GeI,
                    raw_instruction: Instruction::create_abck(OpCode::GeI, 4, 137, 0, false)
                        .as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                    reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(10)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 82,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, -5).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(78)],
                    writes: Vec::new(),
                },
            ],
            guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 81,
                branch_pc: 82,
                exit_pc: 83,
                taken_on_trace: true,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard,
            }],
        };
        let helper_plan = HelperPlan {
            root_pc: 78,
            loop_tail_pc: 82,
            steps: vec![],
            guard_count: 1,
            summary: HelperPlanDispatchSummary {
                steps_executed: 5,
                guards_observed: 1,
                call_steps: 0,
                metamethod_steps: 1,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::LinearIntJmpLoop {
                        steps: vec![
                            LinearIntStep::AddI {
                                dst: 5,
                                src: 5,
                                imm: 1,
                            },
                            LinearIntStep::AddI {
                                dst: 4,
                                src: 4,
                                imm: 1,
                            },
                        ],
                        guard: LinearIntLoopGuard::TailRegImm {
                            op: LinearIntGuardOp::Ge,
                            reg: 4,
                            imm: 10,
                            continue_when: true,
                            exit_pc: 83,
                        },
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_marks_generic_for_builtin_add_as_executable() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 57,
            loop_tail_pc: 60,
            insts: vec![
                TraceIrInst {
                    pc: 57,
                    opcode: OpCode::Add,
                    raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 13, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(13)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 58,
                    opcode: OpCode::MmBin,
                    raw_instruction: Instruction::create_abc(OpCode::MmBin, 5, 13, 6).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(13)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 59,
                    opcode: OpCode::TForCall,
                    raw_instruction: Instruction::create_abc(OpCode::TForCall, 9, 0, 2).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                    reads: vec![
                        TraceIrOperand::Register(9),
                        TraceIrOperand::Register(10),
                        TraceIrOperand::Register(12),
                    ],
                    writes: vec![TraceIrOperand::RegisterRange { start: 12, count: 2 }],
                },
                TraceIrInst {
                    pc: 60,
                    opcode: OpCode::TForLoop,
                    raw_instruction: Instruction::create_abx(OpCode::TForLoop, 9, 4).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::Register(12), TraceIrOperand::JumpTarget(57)],
                    writes: Vec::new(),
                },
            ],
            guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 60,
                branch_pc: 60,
                exit_pc: 61,
                taken_on_trace: true,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard,
            }],
        };
        let helper_plan = HelperPlan {
            root_pc: 57,
            loop_tail_pc: 60,
            steps: vec![
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(13)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(13)],
                },
                HelperPlanStep::Call {
                    reads: vec![
                        TraceIrOperand::Register(9),
                        TraceIrOperand::Register(10),
                        TraceIrOperand::Register(12),
                    ],
                    writes: vec![TraceIrOperand::RegisterRange { start: 12, count: 2 }],
                },
                HelperPlanStep::LoopBackedge {
                    reads: vec![TraceIrOperand::Register(12), TraceIrOperand::JumpTarget(57)],
                    writes: Vec::new(),
                },
            ],
            guard_count: 1,
            summary: HelperPlanDispatchSummary {
                steps_executed: 4,
                guards_observed: 0,
                call_steps: 1,
                metamethod_steps: 1,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::GenericForBuiltinAdd {
                        tfor_reg: 9,
                        value_reg: 13,
                        acc_reg: 5,
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }

    #[test]
    fn backend_marks_next_while_builtin_add_as_executable() {
        let mut backend = NullTraceBackend;
        let ir = TraceIr {
            root_pc: 196,
            loop_tail_pc: 206,
            insts: vec![
                TraceIrInst {
                    pc: 196,
                    opcode: OpCode::Test,
                    raw_instruction: Instruction::create_abck(OpCode::Test, 10, 0, 0, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                    reads: vec![TraceIrOperand::Register(10), TraceIrOperand::Bool(false)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 197,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, 9).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                    reads: vec![TraceIrOperand::JumpTarget(207)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 198,
                    opcode: OpCode::Add,
                    raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 11, false).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(11)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                TraceIrInst {
                    pc: 199,
                    opcode: OpCode::MmBin,
                    raw_instruction: Instruction::create_abc(OpCode::MmBin, 5, 11, 6).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(11)],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 200,
                    opcode: OpCode::GetTabUp,
                    raw_instruction: Instruction::create_abc(OpCode::GetTabUp, 12, 0, 15).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                    reads: vec![TraceIrOperand::Upvalue(0), TraceIrOperand::ConstantIndex(15)],
                    writes: vec![TraceIrOperand::Register(12)],
                },
                TraceIrInst {
                    pc: 201,
                    opcode: OpCode::Move,
                    raw_instruction: Instruction::create_abc(OpCode::Move, 13, 1, 0).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                    reads: vec![TraceIrOperand::Register(1)],
                    writes: vec![TraceIrOperand::Register(13)],
                },
                TraceIrInst {
                    pc: 202,
                    opcode: OpCode::Move,
                    raw_instruction: Instruction::create_abc(OpCode::Move, 14, 10, 0).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                    reads: vec![TraceIrOperand::Register(10)],
                    writes: vec![TraceIrOperand::Register(14)],
                },
                TraceIrInst {
                    pc: 203,
                    opcode: OpCode::Call,
                    raw_instruction: Instruction::create_abc(OpCode::Call, 12, 3, 3).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                    reads: vec![TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::RegisterRange { start: 12, count: 2 }],
                },
                TraceIrInst {
                    pc: 204,
                    opcode: OpCode::Move,
                    raw_instruction: Instruction::create_abc(OpCode::Move, 11, 13, 0).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                    reads: vec![TraceIrOperand::Register(13)],
                    writes: vec![TraceIrOperand::Register(11)],
                },
                TraceIrInst {
                    pc: 205,
                    opcode: OpCode::Move,
                    raw_instruction: Instruction::create_abc(OpCode::Move, 10, 12, 0).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                    reads: vec![TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::Register(10)],
                },
                TraceIrInst {
                    pc: 206,
                    opcode: OpCode::Jmp,
                    raw_instruction: Instruction::create_sj(OpCode::Jmp, -11).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                    reads: vec![TraceIrOperand::JumpTarget(196)],
                    writes: Vec::new(),
                },
            ],
            guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 196,
                branch_pc: 197,
                exit_pc: 207,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            }],
        };
        let helper_plan = HelperPlan {
            root_pc: 196,
            loop_tail_pc: 206,
            steps: vec![
                HelperPlanStep::Guard {
                    reads: vec![TraceIrOperand::Register(10), TraceIrOperand::Bool(false)],
                },
                HelperPlanStep::Branch {
                    reads: vec![TraceIrOperand::JumpTarget(207)],
                },
                HelperPlanStep::Arithmetic {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(11)],
                    writes: vec![TraceIrOperand::Register(5)],
                },
                HelperPlanStep::MetamethodFallback {
                    reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(11)],
                },
                HelperPlanStep::TableAccess {
                    reads: vec![TraceIrOperand::Upvalue(0), TraceIrOperand::ConstantIndex(15)],
                    writes: vec![TraceIrOperand::Register(12)],
                },
                HelperPlanStep::LoadMove {
                    reads: vec![TraceIrOperand::Register(1)],
                    writes: vec![TraceIrOperand::Register(13)],
                },
                HelperPlanStep::LoadMove {
                    reads: vec![TraceIrOperand::Register(10)],
                    writes: vec![TraceIrOperand::Register(14)],
                },
                HelperPlanStep::Call {
                    reads: vec![TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::RegisterRange { start: 12, count: 2 }],
                },
                HelperPlanStep::LoadMove {
                    reads: vec![TraceIrOperand::Register(13)],
                    writes: vec![TraceIrOperand::Register(11)],
                },
                HelperPlanStep::LoadMove {
                    reads: vec![TraceIrOperand::Register(12)],
                    writes: vec![TraceIrOperand::Register(10)],
                },
                HelperPlanStep::LoopBackedge {
                    reads: vec![TraceIrOperand::JumpTarget(196)],
                    writes: Vec::new(),
                },
            ],
            guard_count: 1,
            summary: HelperPlanDispatchSummary {
                steps_executed: 11,
                guards_observed: 1,
                call_steps: 1,
                metamethod_steps: 1,
            },
        };

        match backend.compile(&ir, &helper_plan) {
            BackendCompileOutcome::Compiled(compiled) => {
                assert_eq!(
                    compiled.executor(),
                    CompiledTraceExecutor::NextWhileBuiltinAdd {
                        key_reg: 10,
                        value_reg: 11,
                        acc_reg: 5,
                        table_reg: 1,
                        env_upvalue: 0,
                        key_const: 15,
                    }
                );
            }
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        }
    }
}

fn compile_numeric_steps_from_chunk(
    chunk: &LuaProto,
    start_pc: u32,
    end_pc: u32,
) -> Option<Vec<NumericStep>> {
    if start_pc >= end_pc {
        return Some(Vec::new());
    }

    let insts = (start_pc..end_pc)
        .map(|pc| {
            let raw_instruction = chunk.code.get(pc as usize)?.as_u32();
            let opcode = Instruction::from_u32(raw_instruction).get_opcode();
            let kind = match opcode {
                crate::OpCode::MmBin | crate::OpCode::MmBinI | crate::OpCode::MmBinK => {
                    TraceIrInstKind::MetamethodFallback
                }
                crate::OpCode::Jmp => TraceIrInstKind::Branch,
                crate::OpCode::ForLoop | crate::OpCode::TForLoop => TraceIrInstKind::LoopBackedge,
                _ => TraceIrInstKind::Arithmetic,
            };
            Some(TraceIrInst {
                pc,
                opcode,
                raw_instruction,
                kind,
                reads: Vec::new(),
                writes: Vec::new(),
            })
        })
        .collect::<Option<Vec<_>>>()?;

    compile_numeric_steps(&insts)
}