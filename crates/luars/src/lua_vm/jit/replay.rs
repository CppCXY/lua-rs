use crate::lua_vm::execute::helper::{
    float_for_loop, lua_fmod, lua_idiv, lua_imod, lua_shiftl, lua_shiftr, luai_numpow, setbfvalue,
    setbtvalue, setfltvalue, setivalue, setnilvalue,
};
use crate::{Chunk, Instruction, LuaState, LuaValue, OpCode};

use super::{
    CompiledTraceArtifact, CompiledTraceExit, JitPolicy, TraceAbortReason, TraceArtifact,
    TraceDispatchResult, TraceExitAction, TraceExitKind, TraceExitSite, TraceGuard,
    TraceGuardKind, TraceGuardMode, TraceGuardOperands, TraceId, TracePlan, TraceRunOutcome,
    TraceRunResult,
};
#[cfg(feature = "jit-cranelift")]
use super::artifact::CraneliftTraceArtifact;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompiledTraceIterationOutcome {
    LoopContinue,
    Return(TraceRunResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceTreeStep {
    Return(TraceRunResult),
    JumpToChild(TraceId),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MaterializedReg {
    reg: u8,
    value: LuaValue,
}

#[derive(Debug, Clone, PartialEq)]
struct MaterializedTraceExit {
    target_pc: usize,
    resume_pc: usize,
    base: usize,
    frame_depth: usize,
    regs: Vec<MaterializedReg>,
}

pub fn execute_trace(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    plan: &TracePlan,
    base: usize,
    policy: JitPolicy,
) -> Result<TraceRunResult, TraceAbortReason> {
    match execute_trace_inner(lua_state, chunk, plan, base, policy, false)? {
        TraceTreeStep::Return(result) => Ok(result),
        TraceTreeStep::JumpToChild(_) => Err(TraceAbortReason::UnsupportedControlFlow),
    }
}

pub fn execute_trace_tree(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    plan: &TracePlan,
    base: usize,
    policy: JitPolicy,
) -> Result<TraceDispatchResult, TraceAbortReason> {
    execute_tree_artifact(
        lua_state,
        chunk,
        TraceArtifact::Replay(plan.clone()),
        base,
        policy,
    )
}

fn execute_trace_inner(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    plan: &TracePlan,
    base: usize,
    policy: JitPolicy,
    follow_linked_side_traces: bool,
) -> Result<TraceTreeStep, TraceAbortReason> {
    let replay_budget = policy.max_trace_replays.max(1) as usize;

    for _ in 0..replay_budget {
        let mut loop_completed = false;

        for trace_instr in &plan.instructions {
            let mut has_control_guard = false;
            for guard in plan
                .guards
                .iter()
                .filter(|guard| guard.pc == trace_instr.pc)
            {
                has_control_guard |= guard.mode == TraceGuardMode::Control;
                if !evaluate_guard(lua_state, chunk, base, guard)? {
                    let exit = plan
                        .exits
                        .iter()
                        .find(|exit| exit.snapshot_index == guard.exit_snapshot_index)
                        .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
                    if follow_linked_side_traces {
                        if let Some(side_trace_id) = exit.side_trace {
                            apply_exit_actions_direct(lua_state, base, &exit.actions)?;
                            return Ok(TraceTreeStep::JumpToChild(side_trace_id));
                        }
                    }
                    let target_pc = if exit.side_trace.is_some() {
                        apply_exit_actions_direct(lua_state, base, &exit.actions)?;
                        exit.target_pc
                    } else {
                        let materialized_exit =
                            materialize_exit_state(lua_state, plan, exit.snapshot_index, exit, base)?;
                        commit_materialized_exit(lua_state, base, &materialized_exit);
                        materialized_exit.target_pc
                    };
                    return Ok(TraceTreeStep::Return(TraceRunResult {
                        next_pc: target_pc,
                        outcome: TraceRunOutcome::SideExit,
                        exit: Some(TraceExitSite {
                            kind: exit.kind,
                            source_pc: exit.source_pc,
                            target_pc: exit.target_pc,
                            snapshot_index: exit.snapshot_index,
                        }),
                    }));
                }
            }
            if has_control_guard {
                continue;
            }

            let instr = *chunk
                .code
                .get(trace_instr.pc)
                .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
            match instr.get_opcode() {
                OpCode::Move => execute_move(lua_state, base, instr),
                OpCode::LoadI => execute_loadi(lua_state, base, instr),
                OpCode::LoadF => execute_loadf(lua_state, base, instr),
                OpCode::LoadK => execute_loadk(lua_state, chunk, base, instr)?,
                OpCode::LoadFalse => execute_loadfalse(lua_state, base, instr),
                OpCode::LoadTrue => execute_loadtrue(lua_state, base, instr),
                OpCode::LoadNil => execute_loadnil(lua_state, base, instr),
                OpCode::AddI => execute_addi(lua_state, base, instr)?,
                OpCode::AddK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Add)?,
                OpCode::SubK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Sub)?,
                OpCode::MulK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Mul)?,
                OpCode::ModK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Mod)?,
                OpCode::PowK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Pow)?,
                OpCode::DivK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Div)?,
                OpCode::IDivK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::IDiv)?,
                OpCode::BAndK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::BAnd)?,
                OpCode::BOrK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::BOr)?,
                OpCode::BXorK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::BXor)?,
                OpCode::ShlI => execute_shli(lua_state, base, instr)?,
                OpCode::ShrI => execute_shri(lua_state, base, instr)?,
                OpCode::Add => execute_binary_rr(lua_state, base, instr, NumericOp::Add)?,
                OpCode::Sub => execute_binary_rr(lua_state, base, instr, NumericOp::Sub)?,
                OpCode::Mul => execute_binary_rr(lua_state, base, instr, NumericOp::Mul)?,
                OpCode::Mod => execute_binary_rr(lua_state, base, instr, NumericOp::Mod)?,
                OpCode::Pow => execute_binary_rr(lua_state, base, instr, NumericOp::Pow)?,
                OpCode::Div => execute_binary_rr(lua_state, base, instr, NumericOp::Div)?,
                OpCode::IDiv => execute_binary_rr(lua_state, base, instr, NumericOp::IDiv)?,
                OpCode::BAnd => execute_binary_rr(lua_state, base, instr, NumericOp::BAnd)?,
                OpCode::BOr => execute_binary_rr(lua_state, base, instr, NumericOp::BOr)?,
                OpCode::BXor => execute_binary_rr(lua_state, base, instr, NumericOp::BXor)?,
                OpCode::Shl => execute_binary_rr(lua_state, base, instr, NumericOp::Shl)?,
                OpCode::Shr => execute_binary_rr(lua_state, base, instr, NumericOp::Shr)?,
                OpCode::Unm => execute_unm(lua_state, base, instr)?,
                OpCode::BNot => execute_bnot(lua_state, base, instr)?,
                OpCode::Not => execute_not(lua_state, base, instr),
                OpCode::Jmp => {
                    let target = jump_target(trace_instr.pc, instr)?;
                    if target != plan.anchor_pc {
                        if target > trace_instr.pc {
                            continue;
                        }
                        return Err(TraceAbortReason::UnsupportedControlFlow);
                    }
                    loop_completed = true;
                    break;
                }
                OpCode::ForLoop => {
                    let target = for_loop_target(trace_instr.pc, instr)?;
                    if target != plan.anchor_pc {
                        return Err(TraceAbortReason::UnsupportedControlFlow);
                    }
                    if execute_for_loop(lua_state, base, instr) {
                        loop_completed = true;
                        break;
                    }

                    let exit = plan
                        .exits
                        .iter()
                        .find(|exit| {
                            exit.source_pc == trace_instr.pc && exit.target_pc != trace_instr.pc
                        })
                        .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
                    let materialized_exit =
                        materialize_exit_state(lua_state, plan, exit.snapshot_index, exit, base)?;
                    commit_materialized_exit(lua_state, base, &materialized_exit);
                    return Ok(TraceTreeStep::Return(TraceRunResult {
                        next_pc: materialized_exit.target_pc,
                        outcome: TraceRunOutcome::SideExit,
                        exit: Some(TraceExitSite {
                            kind: exit.kind,
                            source_pc: exit.source_pc,
                            target_pc: exit.target_pc,
                            snapshot_index: exit.snapshot_index,
                        }),
                    }));
                }
                _ => return Err(TraceAbortReason::UnsupportedOpcode),
            }
        }

        if !loop_completed {
            return Ok(TraceTreeStep::Return(TraceRunResult {
                next_pc: plan.anchor_pc,
                outcome: TraceRunOutcome::Anchored,
                exit: None,
            }));
        }
    }

    Ok(TraceTreeStep::Return(TraceRunResult {
        next_pc: plan.anchor_pc,
        outcome: TraceRunOutcome::Anchored,
        exit: None,
    }))
}

pub fn execute_compiled_trace(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    compiled: &CompiledTraceArtifact,
    base: usize,
    policy: JitPolicy,
) -> Result<TraceRunResult, TraceAbortReason> {
    match execute_compiled_trace_inner(lua_state, chunk, compiled, base, policy, false)? {
        TraceTreeStep::Return(result) => Ok(result),
        TraceTreeStep::JumpToChild(_) => Err(TraceAbortReason::UnsupportedControlFlow),
    }
}

pub fn execute_compiled_trace_tree(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    compiled: &CompiledTraceArtifact,
    base: usize,
    policy: JitPolicy,
) -> Result<TraceDispatchResult, TraceAbortReason> {
    execute_tree_artifact(
        lua_state,
        chunk,
        TraceArtifact::Compiled(compiled.clone()),
        base,
        policy,
    )
}

#[cfg(feature = "jit-cranelift")]
pub fn execute_cranelift_trace_tree(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    artifact: &CraneliftTraceArtifact,
    base: usize,
    policy: JitPolicy,
) -> Result<TraceDispatchResult, TraceAbortReason> {
    execute_tree_artifact(
        lua_state,
        chunk,
        TraceArtifact::Cranelift(artifact.clone()),
        base,
        policy,
    )
}

#[cfg(feature = "jit-cranelift")]
fn execute_cranelift_trace_inner(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    artifact: &CraneliftTraceArtifact,
    base: usize,
    policy: JitPolicy,
    follow_linked_side_traces: bool,
) -> Result<TraceTreeStep, TraceAbortReason> {
    let result = super::cranelift::execute_cranelift_trace(lua_state, chunk, artifact, base, policy)?;
    if follow_linked_side_traces {
        if let Some(exit) = result.exit {
            if exit.kind == TraceExitKind::GuardExit {
                if let Some(side_trace_id) = artifact
                    .compiled
                    .unit
                    .plan
                    .exits
                    .iter()
                    .find(|candidate| candidate.snapshot_index == exit.snapshot_index)
                    .and_then(|candidate| candidate.side_trace)
                {
                    return Ok(TraceTreeStep::JumpToChild(side_trace_id));
                }
            }
        }
    }

    Ok(TraceTreeStep::Return(result))
}

fn execute_compiled_trace_inner(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    compiled: &CompiledTraceArtifact,
    base: usize,
    policy: JitPolicy,
    follow_linked_side_traces: bool,
) -> Result<TraceTreeStep, TraceAbortReason> {
    let replay_budget = policy.max_trace_replays.max(1) as usize;

    for _ in 0..replay_budget {
        match execute_compiled_trace_iteration(lua_state, chunk, compiled, base)? {
            CompiledTraceIterationOutcome::LoopContinue => {}
            CompiledTraceIterationOutcome::Return(result) => {
                if follow_linked_side_traces {
                    if let Some(exit) = result.exit {
                        if exit.kind == TraceExitKind::GuardExit {
                            if let Some(side_trace_id) = compiled
                                .unit
                                .plan
                                .exits
                                .iter()
                                .find(|candidate| candidate.snapshot_index == exit.snapshot_index)
                                .and_then(|candidate| candidate.side_trace)
                            {
                                return Ok(TraceTreeStep::JumpToChild(side_trace_id));
                            }
                        }
                    }
                }
                return Ok(TraceTreeStep::Return(result));
            }
        }
    }

    Ok(TraceTreeStep::Return(TraceRunResult {
        next_pc: compiled.unit.plan.anchor_pc,
        outcome: TraceRunOutcome::Anchored,
        exit: None,
    }))
}

fn execute_tree_artifact(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    root: TraceArtifact,
    base: usize,
    policy: JitPolicy,
) -> Result<TraceDispatchResult, TraceAbortReason> {
    const MAX_TRACE_TREE_DEPTH: usize = 32;

    let mut current = root;
    for _ in 0..MAX_TRACE_TREE_DEPTH {
        let trace_id = current.id();
        let step = match &current {
            TraceArtifact::Replay(plan) => {
                execute_trace_inner(lua_state, chunk, plan, base, policy, true)?
            }
            TraceArtifact::Compiled(compiled) => {
                execute_compiled_trace_inner(lua_state, chunk, compiled, base, policy, true)?
            }
            #[cfg(feature = "jit-cranelift")]
            TraceArtifact::Cranelift(artifact) => {
                execute_cranelift_trace_inner(lua_state, chunk, artifact, base, policy, true)?
            }
            TraceArtifact::NativePlaceholder(_) => return Err(TraceAbortReason::NotImplemented),
        };

        match step {
            TraceTreeStep::Return(run_result) => {
                return Ok(TraceDispatchResult {
                    trace_id,
                    run_result,
                });
            }
            TraceTreeStep::JumpToChild(next_trace_id) => {
                let Some(next_artifact) = lua_state
                    .vm_mut()
                    .jit_runtime()
                    .trace_artifact(next_trace_id)
                    .cloned()
                else {
                    return Err(TraceAbortReason::UnsupportedControlFlow);
                };

                current = next_artifact;
            }
        }
    }

    let trace_id = current.id();
    let run_result = current.execute(lua_state, chunk, base, policy)?;
    Ok(TraceDispatchResult {
        trace_id,
        run_result,
    })
}

pub(crate) fn execute_compiled_trace_iteration(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    compiled: &CompiledTraceArtifact,
    base: usize,
) -> Result<CompiledTraceIterationOutcome, TraceAbortReason> {
    execute_compiled_trace_iteration_from_step(lua_state, chunk, compiled, base, 0)
}

pub(crate) fn execute_compiled_trace_iteration_from_step(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    compiled: &CompiledTraceArtifact,
    base: usize,
    start_step: usize,
) -> Result<CompiledTraceIterationOutcome, TraceAbortReason> {
    for step in compiled.steps.iter().skip(start_step) {
        let mut has_control_guard = false;
        for compiled_guard in &step.guards {
            has_control_guard |= compiled_guard.guard.mode == TraceGuardMode::Control;
            if !evaluate_guard(lua_state, chunk, base, &compiled_guard.guard)? {
                let target_pc = if compiled_guard.exit.side_trace.is_some() {
                    apply_exit_actions_direct(lua_state, base, &compiled_guard.exit.actions)?;
                    compiled_guard.exit.target_pc
                } else {
                    let materialized_exit = materialize_compiled_exit_state(
                        lua_state,
                        &compiled.unit.plan,
                        &compiled_guard.exit,
                        base,
                    )?;
                    commit_materialized_exit(lua_state, base, &materialized_exit);
                    materialized_exit.target_pc
                };
                return Ok(CompiledTraceIterationOutcome::Return(TraceRunResult {
                    next_pc: target_pc,
                    outcome: TraceRunOutcome::SideExit,
                    exit: Some(TraceExitSite {
                        kind: TraceExitKind::GuardExit,
                        source_pc: compiled_guard.guard.pc,
                        target_pc: compiled_guard.exit.target_pc,
                        snapshot_index: compiled_guard.exit.snapshot_index,
                    }),
                }));
            }
        }
        if has_control_guard {
            continue;
        }

        let instr = *chunk
            .code
            .get(step.instruction.pc)
            .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
        match step.instruction.opcode {
            OpCode::Move => execute_move(lua_state, base, instr),
            OpCode::LoadI => execute_loadi(lua_state, base, instr),
            OpCode::LoadF => execute_loadf(lua_state, base, instr),
            OpCode::LoadK => execute_loadk(lua_state, chunk, base, instr)?,
            OpCode::LoadFalse => execute_loadfalse(lua_state, base, instr),
            OpCode::LoadTrue => execute_loadtrue(lua_state, base, instr),
            OpCode::LoadNil => execute_loadnil(lua_state, base, instr),
            OpCode::AddI => execute_addi(lua_state, base, instr)?,
            OpCode::AddK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Add)?,
            OpCode::SubK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Sub)?,
            OpCode::MulK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Mul)?,
            OpCode::ModK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Mod)?,
            OpCode::PowK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Pow)?,
            OpCode::DivK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::Div)?,
            OpCode::IDivK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::IDiv)?,
            OpCode::BAndK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::BAnd)?,
            OpCode::BOrK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::BOr)?,
            OpCode::BXorK => execute_binary_k(lua_state, chunk, base, instr, NumericOp::BXor)?,
            OpCode::ShlI => execute_shli(lua_state, base, instr)?,
            OpCode::ShrI => execute_shri(lua_state, base, instr)?,
            OpCode::Add => execute_binary_rr(lua_state, base, instr, NumericOp::Add)?,
            OpCode::Sub => execute_binary_rr(lua_state, base, instr, NumericOp::Sub)?,
            OpCode::Mul => execute_binary_rr(lua_state, base, instr, NumericOp::Mul)?,
            OpCode::Mod => execute_binary_rr(lua_state, base, instr, NumericOp::Mod)?,
            OpCode::Pow => execute_binary_rr(lua_state, base, instr, NumericOp::Pow)?,
            OpCode::Div => execute_binary_rr(lua_state, base, instr, NumericOp::Div)?,
            OpCode::IDiv => execute_binary_rr(lua_state, base, instr, NumericOp::IDiv)?,
            OpCode::BAnd => execute_binary_rr(lua_state, base, instr, NumericOp::BAnd)?,
            OpCode::BOr => execute_binary_rr(lua_state, base, instr, NumericOp::BOr)?,
            OpCode::BXor => execute_binary_rr(lua_state, base, instr, NumericOp::BXor)?,
            OpCode::Shl => execute_binary_rr(lua_state, base, instr, NumericOp::Shl)?,
            OpCode::Shr => execute_binary_rr(lua_state, base, instr, NumericOp::Shr)?,
            OpCode::Unm => execute_unm(lua_state, base, instr)?,
            OpCode::BNot => execute_bnot(lua_state, base, instr)?,
            OpCode::Not => execute_not(lua_state, base, instr),
            OpCode::Jmp => {
                let target = jump_target(step.instruction.pc, instr)?;
                if target != compiled.unit.plan.anchor_pc {
                    if target > step.instruction.pc {
                        continue;
                    }
                    return Err(TraceAbortReason::UnsupportedControlFlow);
                }
                return Ok(CompiledTraceIterationOutcome::LoopContinue);
            }
            OpCode::ForLoop => {
                let target = for_loop_target(step.instruction.pc, instr)?;
                if target != compiled.unit.plan.anchor_pc {
                    return Err(TraceAbortReason::UnsupportedControlFlow);
                }
                if execute_for_loop(lua_state, base, instr) {
                    return Ok(CompiledTraceIterationOutcome::LoopContinue);
                }

                let exit = step
                    .loop_exit
                    .as_ref()
                    .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
                let materialized_exit =
                    materialize_compiled_exit_state(lua_state, &compiled.unit.plan, exit, base)?;
                commit_materialized_exit(lua_state, base, &materialized_exit);
                return Ok(CompiledTraceIterationOutcome::Return(TraceRunResult {
                    next_pc: materialized_exit.target_pc,
                    outcome: TraceRunOutcome::SideExit,
                    exit: Some(TraceExitSite {
                        kind: TraceExitKind::LoopExit,
                        source_pc: step.instruction.pc,
                        target_pc: exit.target_pc,
                        snapshot_index: exit.snapshot_index,
                    }),
                }));
            }
            _ => return Err(TraceAbortReason::UnsupportedOpcode),
        }
    }

    Ok(CompiledTraceIterationOutcome::Return(TraceRunResult {
        next_pc: compiled.unit.plan.anchor_pc,
        outcome: TraceRunOutcome::Anchored,
        exit: None,
    }))
}

#[derive(Clone, Copy)]
enum NumericOp {
    Add,
    Sub,
    Mul,
    Mod,
    Pow,
    Div,
    IDiv,
    BAnd,
    BOr,
    BXor,
    Shl,
    Shr,
}

fn evaluate_guard(
    lua_state: &LuaState,
    chunk: &Chunk,
    base: usize,
    guard: &TraceGuard,
) -> Result<bool, TraceAbortReason> {
    let cond = match (guard.kind, guard.operands) {
        (TraceGuardKind::Eq, TraceGuardOperands::Registers { lhs, rhs }) => {
            stack_value(lua_state, base, lhs) == stack_value(lua_state, base, rhs)
        }
        (TraceGuardKind::Eq, TraceGuardOperands::RegisterImmediate { reg, imm }) => {
            compare_eq_immediate(stack_value(lua_state, base, reg), imm)
        }
        (
            TraceGuardKind::Eq,
            TraceGuardOperands::RegisterConstant {
                reg,
                constant_index,
            },
        ) => {
            let constant = chunk
                .constants
                .get(constant_index)
                .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
            stack_value(lua_state, base, reg) == constant
        }
        (TraceGuardKind::Lt, TraceGuardOperands::Registers { lhs, rhs }) => compare_lt(
            stack_value(lua_state, base, lhs),
            stack_value(lua_state, base, rhs),
        )?,
        (TraceGuardKind::Lt, TraceGuardOperands::RegisterImmediate { reg, imm }) => {
            compare_lt_immediate_rhs(stack_value(lua_state, base, reg), imm)?
        }
        (TraceGuardKind::Lt, TraceGuardOperands::ImmediateRegister { imm, reg }) => {
            compare_lt_immediate_lhs(imm, stack_value(lua_state, base, reg))?
        }
        (TraceGuardKind::Le, TraceGuardOperands::Registers { lhs, rhs }) => compare_le(
            stack_value(lua_state, base, lhs),
            stack_value(lua_state, base, rhs),
        )?,
        (TraceGuardKind::Le, TraceGuardOperands::RegisterImmediate { reg, imm }) => {
            compare_le_immediate_rhs(stack_value(lua_state, base, reg), imm)?
        }
        (TraceGuardKind::Le, TraceGuardOperands::ImmediateRegister { imm, reg }) => {
            compare_le_immediate_lhs(imm, stack_value(lua_state, base, reg))?
        }
        (TraceGuardKind::Truthy, TraceGuardOperands::Register { reg }) => {
            let value = stack_value(lua_state, base, reg);
            !value.is_nil() && !value.ttisfalse()
        }
        (TraceGuardKind::Falsey, TraceGuardOperands::Register { reg }) => {
            let value = stack_value(lua_state, base, reg);
            value.is_nil() || value.ttisfalse()
        }
        (TraceGuardKind::IsNumber, TraceGuardOperands::Register { reg }) => {
            stack_value(lua_state, base, reg).is_number()
        }
        (TraceGuardKind::IsIntegerLike, TraceGuardOperands::Register { reg }) => {
            stack_value(lua_state, base, reg).as_integer().is_some()
        }
        (TraceGuardKind::IsComparableLtLe, TraceGuardOperands::Registers { lhs, rhs }) => {
            let lhs = stack_value(lua_state, base, lhs);
            let rhs = stack_value(lua_state, base, rhs);
            (lhs.as_float().is_some() && rhs.as_float().is_some())
                || (lhs.as_bytes().is_some() && rhs.as_bytes().is_some())
        }
        (TraceGuardKind::IsEqSafeComparable, TraceGuardOperands::Registers { lhs, rhs }) => {
            let lhs = stack_value(lua_state, base, lhs);
            let rhs = stack_value(lua_state, base, rhs);
            is_eq_safe(lhs) && is_eq_safe(rhs)
        }
        _ => return Err(TraceAbortReason::UnsupportedOpcode),
    };

    Ok(cond == guard.continue_when)
}

fn is_eq_safe(value: &LuaValue) -> bool {
    value.is_nil() || value.is_boolean() || value.is_number() || value.is_string()
}

fn compare_eq_immediate(value: &LuaValue, imm: i64) -> bool {
    if let Some(integer) = value.as_integer_strict() {
        integer == imm
    } else if let Some(number) = value.as_float() {
        number == imm as f64
    } else {
        false
    }
}

fn compare_lt(lhs: &LuaValue, rhs: &LuaValue) -> Result<bool, TraceAbortReason> {
    if let (Some(li), Some(ri)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
        Ok(li < ri)
    } else if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        Ok(ln < rn)
    } else if let (Some(lb), Some(rb)) = (lhs.as_bytes(), rhs.as_bytes()) {
        Ok(lb < rb)
    } else {
        Err(TraceAbortReason::UnsupportedOpcode)
    }
}

fn compare_lt_immediate_rhs(lhs: &LuaValue, imm: i64) -> Result<bool, TraceAbortReason> {
    if let Some(integer) = lhs.as_integer_strict() {
        Ok(integer < imm)
    } else if let Some(number) = lhs.as_float() {
        Ok(number < imm as f64)
    } else {
        Err(TraceAbortReason::UnsupportedOpcode)
    }
}

fn compare_lt_immediate_lhs(imm: i64, rhs: &LuaValue) -> Result<bool, TraceAbortReason> {
    if let Some(integer) = rhs.as_integer_strict() {
        Ok(imm < integer)
    } else if let Some(number) = rhs.as_float() {
        Ok((imm as f64) < number)
    } else {
        Err(TraceAbortReason::UnsupportedOpcode)
    }
}

fn compare_le(lhs: &LuaValue, rhs: &LuaValue) -> Result<bool, TraceAbortReason> {
    if let (Some(li), Some(ri)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
        Ok(li <= ri)
    } else if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        Ok(ln <= rn)
    } else if let (Some(lb), Some(rb)) = (lhs.as_bytes(), rhs.as_bytes()) {
        Ok(lb <= rb)
    } else {
        Err(TraceAbortReason::UnsupportedOpcode)
    }
}

fn compare_le_immediate_rhs(lhs: &LuaValue, imm: i64) -> Result<bool, TraceAbortReason> {
    if let Some(integer) = lhs.as_integer_strict() {
        Ok(integer <= imm)
    } else if let Some(number) = lhs.as_float() {
        Ok(number <= imm as f64)
    } else {
        Err(TraceAbortReason::UnsupportedOpcode)
    }
}

fn compare_le_immediate_lhs(imm: i64, rhs: &LuaValue) -> Result<bool, TraceAbortReason> {
    if let Some(integer) = rhs.as_integer_strict() {
        Ok(imm <= integer)
    } else if let Some(number) = rhs.as_float() {
        Ok((imm as f64) <= number)
    } else {
        Err(TraceAbortReason::UnsupportedOpcode)
    }
}

fn materialize_exit_state(
    lua_state: &LuaState,
    plan: &TracePlan,
    snapshot_index: usize,
    exit: &crate::lua_vm::jit::TraceExit,
    base: usize,
) -> Result<MaterializedTraceExit, TraceAbortReason> {
    let snapshot = plan
        .snapshots
        .get(snapshot_index)
        .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
    let mut materialized = MaterializedTraceExit {
        target_pc: exit.target_pc,
        resume_pc: snapshot.resume_pc,
        base: snapshot.base,
        frame_depth: snapshot.frame_depth,
        regs: snapshot
            .live_regs
            .iter()
            .map(|&reg| MaterializedReg {
                reg,
                value: *stack_value(lua_state, base, reg),
            })
            .collect(),
    };
    apply_exit_actions(&mut materialized, lua_state, base, &exit.actions)?;
    Ok(materialized)
}

fn materialize_compiled_exit_state(
    lua_state: &LuaState,
    plan: &TracePlan,
    exit: &CompiledTraceExit,
    base: usize,
) -> Result<MaterializedTraceExit, TraceAbortReason> {
    let snapshot = plan
        .snapshots
        .get(exit.snapshot_index)
        .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
    let mut materialized = MaterializedTraceExit {
        target_pc: exit.target_pc,
        resume_pc: snapshot.resume_pc,
        base: snapshot.base,
        frame_depth: snapshot.frame_depth,
        regs: snapshot
            .live_regs
            .iter()
            .map(|&reg| MaterializedReg {
                reg,
                value: *stack_value(lua_state, base, reg),
            })
            .collect(),
    };
    apply_exit_actions(&mut materialized, lua_state, base, &exit.actions)?;
    Ok(materialized)
}

fn apply_exit_actions(
    materialized: &mut MaterializedTraceExit,
    lua_state: &LuaState,
    base: usize,
    actions: &[TraceExitAction],
) -> Result<(), TraceAbortReason> {
    for action in actions {
        match *action {
            TraceExitAction::CopyReg { dst, src } => {
                let value = materialized_reg_value(materialized, src)
                    .unwrap_or_else(|| *stack_value(lua_state, base, src));
                set_materialized_reg(materialized, dst, value);
            }
        }
    }
    Ok(())
}

fn apply_exit_actions_direct(
    lua_state: &mut LuaState,
    base: usize,
    actions: &[TraceExitAction],
) -> Result<(), TraceAbortReason> {
    for action in actions {
        match *action {
            TraceExitAction::CopyReg { dst, src } => {
                let value = *stack_value(lua_state, base, src);
                *stack_value_mut(lua_state, base, dst) = value;
            }
        }
    }
    Ok(())
}

fn materialized_reg_value(materialized: &MaterializedTraceExit, reg: u8) -> Option<LuaValue> {
    materialized
        .regs
        .iter()
        .find(|entry| entry.reg == reg)
        .map(|entry| entry.value)
}

fn set_materialized_reg(materialized: &mut MaterializedTraceExit, reg: u8, value: LuaValue) {
    if let Some(entry) = materialized.regs.iter_mut().find(|entry| entry.reg == reg) {
        entry.value = value;
    } else {
        materialized.regs.push(MaterializedReg { reg, value });
    }
}

fn commit_materialized_exit(
    lua_state: &mut LuaState,
    base: usize,
    materialized: &MaterializedTraceExit,
) {
    for reg in &materialized.regs {
        *stack_value_mut(lua_state, base, reg.reg) = reg.value;
    }
}

fn execute_move(lua_state: &mut LuaState, base: usize, instr: Instruction) {
    let value = *stack_value(lua_state, base, instr.get_b() as u8);
    *stack_value_mut(lua_state, base, instr.get_a() as u8) = value;
}

fn execute_loadi(lua_state: &mut LuaState, base: usize, instr: Instruction) {
    setivalue(
        stack_value_mut(lua_state, base, instr.get_a() as u8),
        instr.get_sbx() as i64,
    );
}

fn execute_loadf(lua_state: &mut LuaState, base: usize, instr: Instruction) {
    setfltvalue(
        stack_value_mut(lua_state, base, instr.get_a() as u8),
        instr.get_sbx() as f64,
    );
}

fn execute_loadk(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    base: usize,
    instr: Instruction,
) -> Result<(), TraceAbortReason> {
    let constant = *chunk
        .constants
        .get(instr.get_bx() as usize)
        .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
    *stack_value_mut(lua_state, base, instr.get_a() as u8) = constant;
    Ok(())
}

fn execute_loadfalse(lua_state: &mut LuaState, base: usize, instr: Instruction) {
    setbfvalue(stack_value_mut(lua_state, base, instr.get_a() as u8));
}

fn execute_loadtrue(lua_state: &mut LuaState, base: usize, instr: Instruction) {
    setbtvalue(stack_value_mut(lua_state, base, instr.get_a() as u8));
}

fn execute_loadnil(lua_state: &mut LuaState, base: usize, instr: Instruction) {
    for reg in instr.get_a()..=instr.get_a() + instr.get_b() {
        setnilvalue(stack_value_mut(lua_state, base, reg as u8));
    }
}

fn execute_addi(
    lua_state: &mut LuaState,
    base: usize,
    instr: Instruction,
) -> Result<(), TraceAbortReason> {
    let src = *stack_value(lua_state, base, instr.get_b() as u8);
    let dst = stack_value_mut(lua_state, base, instr.get_a() as u8);
    let sc = instr.get_sc() as i64;
    if let Some(integer) = src.as_integer_strict() {
        setivalue(dst, integer.wrapping_add(sc));
    } else if let Some(number) = src.as_float() {
        setfltvalue(dst, number + sc as f64);
    } else {
        return Err(TraceAbortReason::UnsupportedOpcode);
    }
    Ok(())
}

fn execute_binary_rr(
    lua_state: &mut LuaState,
    base: usize,
    instr: Instruction,
    op: NumericOp,
) -> Result<(), TraceAbortReason> {
    let lhs = *stack_value(lua_state, base, instr.get_b() as u8);
    let rhs = *stack_value(lua_state, base, instr.get_c() as u8);
    execute_binary_into(
        stack_value_mut(lua_state, base, instr.get_a() as u8),
        lhs,
        rhs,
        op,
    )
}

fn execute_binary_k(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    base: usize,
    instr: Instruction,
    op: NumericOp,
) -> Result<(), TraceAbortReason> {
    let lhs = *stack_value(lua_state, base, instr.get_b() as u8);
    let rhs = *chunk
        .constants
        .get(instr.get_c() as usize)
        .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
    execute_binary_into(
        stack_value_mut(lua_state, base, instr.get_a() as u8),
        lhs,
        rhs,
        op,
    )
}

fn execute_binary_into(
    dst: &mut LuaValue,
    lhs: LuaValue,
    rhs: LuaValue,
    op: NumericOp,
) -> Result<(), TraceAbortReason> {
    match op {
        NumericOp::Add => numeric_add(dst, &lhs, &rhs),
        NumericOp::Sub => numeric_sub(dst, &lhs, &rhs),
        NumericOp::Mul => numeric_mul(dst, &lhs, &rhs),
        NumericOp::Mod => numeric_mod(dst, &lhs, &rhs),
        NumericOp::Pow => numeric_pow(dst, &lhs, &rhs),
        NumericOp::Div => numeric_div(dst, &lhs, &rhs),
        NumericOp::IDiv => numeric_idiv(dst, &lhs, &rhs),
        NumericOp::BAnd => numeric_band(dst, &lhs, &rhs),
        NumericOp::BOr => numeric_bor(dst, &lhs, &rhs),
        NumericOp::BXor => numeric_bxor(dst, &lhs, &rhs),
        NumericOp::Shl => numeric_shl(dst, &lhs, &rhs),
        NumericOp::Shr => numeric_shr(dst, &lhs, &rhs),
    }
}

fn numeric_add(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    if let (Some(li), Some(ri)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
        setivalue(dst, li.wrapping_add(ri));
    } else if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        setfltvalue(dst, ln + rn);
    } else {
        return Err(TraceAbortReason::UnsupportedOpcode);
    }
    Ok(())
}

fn numeric_sub(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    if let (Some(li), Some(ri)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
        setivalue(dst, li.wrapping_sub(ri));
    } else if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        setfltvalue(dst, ln - rn);
    } else {
        return Err(TraceAbortReason::UnsupportedOpcode);
    }
    Ok(())
}

fn numeric_mul(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    if let (Some(li), Some(ri)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
        setivalue(dst, li.wrapping_mul(ri));
    } else if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        setfltvalue(dst, ln * rn);
    } else {
        return Err(TraceAbortReason::UnsupportedOpcode);
    }
    Ok(())
}

fn numeric_div(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        setfltvalue(dst, ln / rn);
        Ok(())
    } else {
        Err(TraceAbortReason::UnsupportedOpcode)
    }
}

fn numeric_idiv(
    dst: &mut LuaValue,
    lhs: &LuaValue,
    rhs: &LuaValue,
) -> Result<(), TraceAbortReason> {
    if let (Some(li), Some(ri)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
        if ri == 0 {
            return Err(TraceAbortReason::UnsupportedOpcode);
        }
        setivalue(dst, lua_idiv(li, ri));
    } else if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        setfltvalue(dst, (ln / rn).floor());
    } else {
        return Err(TraceAbortReason::UnsupportedOpcode);
    }
    Ok(())
}

fn numeric_mod(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    if let (Some(li), Some(ri)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
        if ri == 0 {
            return Err(TraceAbortReason::UnsupportedOpcode);
        }
        setivalue(dst, lua_imod(li, ri));
    } else if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        setfltvalue(dst, lua_fmod(ln, rn));
    } else {
        return Err(TraceAbortReason::UnsupportedOpcode);
    }
    Ok(())
}

fn numeric_pow(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    if let (Some(ln), Some(rn)) = (lhs.as_float(), rhs.as_float()) {
        setfltvalue(dst, luai_numpow(ln, rn));
        Ok(())
    } else {
        Err(TraceAbortReason::UnsupportedOpcode)
    }
}

fn numeric_band(
    dst: &mut LuaValue,
    lhs: &LuaValue,
    rhs: &LuaValue,
) -> Result<(), TraceAbortReason> {
    let (li, ri) = integer_pair(lhs, rhs)?;
    setivalue(dst, li & ri);
    Ok(())
}

fn numeric_bor(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    let (li, ri) = integer_pair(lhs, rhs)?;
    setivalue(dst, li | ri);
    Ok(())
}

fn numeric_bxor(
    dst: &mut LuaValue,
    lhs: &LuaValue,
    rhs: &LuaValue,
) -> Result<(), TraceAbortReason> {
    let (li, ri) = integer_pair(lhs, rhs)?;
    setivalue(dst, li ^ ri);
    Ok(())
}

fn numeric_shl(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    let (li, ri) = integer_pair(lhs, rhs)?;
    setivalue(dst, lua_shiftl(li, ri));
    Ok(())
}

fn numeric_shr(dst: &mut LuaValue, lhs: &LuaValue, rhs: &LuaValue) -> Result<(), TraceAbortReason> {
    let (li, ri) = integer_pair(lhs, rhs)?;
    setivalue(dst, lua_shiftr(li, ri));
    Ok(())
}

fn execute_shli(
    lua_state: &mut LuaState,
    base: usize,
    instr: Instruction,
) -> Result<(), TraceAbortReason> {
    let rhs = *stack_value(lua_state, base, instr.get_b() as u8);
    let shift = instr.get_sc() as i64;
    let value = rhs
        .as_integer()
        .ok_or(TraceAbortReason::UnsupportedOpcode)?;
    setivalue(
        stack_value_mut(lua_state, base, instr.get_a() as u8),
        lua_shiftl(shift, value),
    );
    Ok(())
}

fn execute_shri(
    lua_state: &mut LuaState,
    base: usize,
    instr: Instruction,
) -> Result<(), TraceAbortReason> {
    let lhs = *stack_value(lua_state, base, instr.get_b() as u8);
    let value = lhs
        .as_integer()
        .ok_or(TraceAbortReason::UnsupportedOpcode)?;
    setivalue(
        stack_value_mut(lua_state, base, instr.get_a() as u8),
        lua_shiftr(value, instr.get_sc() as i64),
    );
    Ok(())
}

fn execute_unm(
    lua_state: &mut LuaState,
    base: usize,
    instr: Instruction,
) -> Result<(), TraceAbortReason> {
    let src = *stack_value(lua_state, base, instr.get_b() as u8);
    let dst = stack_value_mut(lua_state, base, instr.get_a() as u8);
    if let Some(integer) = src.as_integer_strict() {
        setivalue(dst, integer.wrapping_neg());
    } else if let Some(number) = src.as_float() {
        setfltvalue(dst, -number);
    } else {
        return Err(TraceAbortReason::UnsupportedOpcode);
    }
    Ok(())
}

fn execute_bnot(
    lua_state: &mut LuaState,
    base: usize,
    instr: Instruction,
) -> Result<(), TraceAbortReason> {
    let src = *stack_value(lua_state, base, instr.get_b() as u8);
    let value = src
        .as_integer()
        .ok_or(TraceAbortReason::UnsupportedOpcode)?;
    setivalue(
        stack_value_mut(lua_state, base, instr.get_a() as u8),
        !value,
    );
    Ok(())
}

fn execute_not(lua_state: &mut LuaState, base: usize, instr: Instruction) {
    let src = *stack_value(lua_state, base, instr.get_b() as u8);
    let result = src.is_nil() || src.ttisfalse();
    *stack_value_mut(lua_state, base, instr.get_a() as u8) = LuaValue::boolean(result);
}

fn execute_for_loop(lua_state: &mut LuaState, base: usize, instr: Instruction) -> bool {
    let a = instr.get_a() as usize;
    let ra_pos = base + a;

    {
        let stack = lua_state.stack_mut();
        let count = stack[ra_pos].as_integer_strict();
        let step = stack[ra_pos + 1].as_integer_strict();
        let idx = stack[ra_pos + 2].as_integer_strict();

        if let (Some(count), Some(step), Some(idx)) = (count, step, idx) {
            if count > 0 {
                stack[ra_pos] = LuaValue::integer(count - 1);
                stack[ra_pos + 2] = LuaValue::integer(idx.wrapping_add(step));
                return true;
            }
            return false;
        }
    }

    float_for_loop(lua_state, ra_pos)
}

fn jump_target(pc: usize, instr: Instruction) -> Result<usize, TraceAbortReason> {
    let next_pc = pc
        .checked_add(1)
        .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
    let target = next_pc as isize + instr.get_sj() as isize;
    if target < 0 {
        return Err(TraceAbortReason::UnsupportedControlFlow);
    }
    Ok(target as usize)
}

fn for_loop_target(pc: usize, instr: Instruction) -> Result<usize, TraceAbortReason> {
    let next_pc = pc
        .checked_add(1)
        .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
    next_pc
        .checked_sub(instr.get_bx() as usize)
        .ok_or(TraceAbortReason::UnsupportedControlFlow)
}

fn stack_value(lua_state: &LuaState, base: usize, reg: u8) -> &LuaValue {
    &lua_state.stack()[base + reg as usize]
}

fn stack_value_mut(lua_state: &mut LuaState, base: usize, reg: u8) -> &mut LuaValue {
    &mut lua_state.stack_mut()[base + reg as usize]
}

fn integer_pair(lhs: &LuaValue, rhs: &LuaValue) -> Result<(i64, i64), TraceAbortReason> {
    Ok((
        lhs.as_integer()
            .ok_or(TraceAbortReason::UnsupportedOpcode)?,
        rhs.as_integer()
            .ok_or(TraceAbortReason::UnsupportedOpcode)?,
    ))
}

#[cfg(test)]
mod tests {
    use crate::{Chunk, LuaLanguageLevel, LuaVM, OpCode, SafeOption};

    use super::*;
    use crate::lua_vm::Instruction;
    use crate::lua_vm::jit::{
        LoweredTraceBackend, TraceAnchorKind, TraceBackend, TraceCompilationUnit, TraceExit,
        TraceExitAction, TraceExitKind, TraceFallback, TraceGuard, TraceGuardKind,
        TraceGuardMode, TraceGuardOperands, TraceId, TraceInstruction, TracePlan,
        TraceSnapshot, TraceSnapshotKind,
    };

    fn setup_state(stack_size: usize) -> Box<LuaVM> {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.set_language_level(LuaLanguageLevel::LuaJIT);
        let state = vm.main_state();
        state.grow_stack(stack_size).expect("grow stack");
        state.set_top(stack_size).expect("set top");
        vm
    }

    #[test]
    fn replay_runs_multiple_iterations_until_budget() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = LuaValue::integer(1);
        state.stack_mut()[1] = LuaValue::integer(2);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let plan = TracePlan {
            id: TraceId(1),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::Add,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 1,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![TraceSnapshot {
                kind: TraceSnapshotKind::Entry,
                pc: 0,
                resume_pc: 0,
                base: 0,
                frame_depth: 0,
                live_regs: vec![0, 1],
            }, TraceSnapshot {
                kind: TraceSnapshotKind::SideExit,
                pc: 0,
                resume_pc: 0,
                base: 0,
                frame_depth: 0,
                live_regs: vec![0, 1],
            }],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 0 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
            ],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 0,
                snapshot_index: 1,
                side_trace: None,
                actions: Vec::new(),
            }],
        };

        let next_pc = execute_trace(
            state,
            &chunk,
            &plan,
            0,
            JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 3,
            },
        )
        .expect("replay should succeed");

        assert_eq!(next_pc.next_pc, 0);
        assert_eq!(next_pc.outcome, TraceRunOutcome::Anchored);
        assert_eq!(state.stack()[0].as_integer_strict(), Some(7));
    }

    #[test]
    fn compiled_trace_runs_multiple_iterations_until_budget() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = LuaValue::integer(1);
        state.stack_mut()[1] = LuaValue::integer(2);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let plan = TracePlan {
            id: TraceId(11),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::Add,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 1,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 0 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
            ],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 0,
                snapshot_index: 1,
                side_trace: None,
                actions: Vec::new(),
            }],
        };

        let backend = LoweredTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let compiled = match artifact {
            crate::lua_vm::jit::TraceArtifact::Compiled(compiled) => compiled,
            _ => panic!("expected compiled trace artifact"),
        };

        let next_pc = execute_compiled_trace(
            state,
            &chunk,
            &compiled,
            0,
            JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 3,
            },
        )
        .expect("compiled trace should succeed");

        assert_eq!(next_pc.next_pc, 0);
        assert_eq!(next_pc.outcome, TraceRunOutcome::Anchored);
        assert_eq!(state.stack()[0].as_integer_strict(), Some(7));
    }

    #[test]
    fn replay_takes_guard_exit_and_applies_actions() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = LuaValue::integer(10);
        state.stack_mut()[1] = LuaValue::boolean(false);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::TestSet, 0, 1, 0, false),
            Instruction::create_sj(OpCode::Jmp, 2),
            Instruction::create_abc(OpCode::Add, 2, 2, 2),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let plan = TracePlan {
            id: TraceId(2),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 3,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::TestSet,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 2,
                    opcode: OpCode::Add,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 3,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 4,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![TraceGuard {
                pc: 0,
                mode: TraceGuardMode::Control,
                kind: TraceGuardKind::Falsey,
                operands: TraceGuardOperands::Register { reg: 1 },
                continue_when: false,
                exit_snapshot_index: 1,
            }],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 4,
                snapshot_index: 1,
                side_trace: None,
                actions: vec![TraceExitAction::CopyReg { dst: 0, src: 1 }],
            }],
        };

        let next_pc = execute_trace(state, &chunk, &plan, 0, JitPolicy::default())
            .expect("replay should exit");

        assert_eq!(next_pc.next_pc, 4);
        assert_eq!(next_pc.outcome, TraceRunOutcome::SideExit);
        assert_eq!(state.stack()[0].as_boolean(), Some(false));
    }

    #[test]
    fn replay_precondition_guard_exits_before_unsupported_numeric_path() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = LuaValue::nil();
        state.stack_mut()[1] = LuaValue::integer(2);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let plan = TracePlan {
            id: TraceId(3),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::Add,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 1,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![TraceGuard {
                pc: 0,
                mode: TraceGuardMode::Precondition,
                kind: TraceGuardKind::IsNumber,
                operands: TraceGuardOperands::Register { reg: 0 },
                continue_when: true,
                exit_snapshot_index: 1,
            }],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 0,
                snapshot_index: 1,
                side_trace: None,
                actions: Vec::new(),
            }],
        };

        let next_pc = execute_trace(state, &chunk, &plan, 0, JitPolicy::default())
            .expect("precondition exit should succeed");

        assert_eq!(next_pc.next_pc, 0);
        assert_eq!(next_pc.outcome, TraceRunOutcome::SideExit);
        assert!(state.stack()[0].is_nil());
    }

    #[test]
    fn replay_supports_immediate_and_constant_guards() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = LuaValue::integer(7);
        state.stack_mut()[1] = LuaValue::integer(9);

        let mut chunk = Chunk::new();
        chunk.constants = vec![LuaValue::integer(7)];
        chunk.code = vec![
            Instruction::create_abck(OpCode::EqK, 0, 0, 0, false),
            Instruction::create_sj(OpCode::Jmp, 3),
            Instruction::create_abck(OpCode::GtI, 1, 127 + 5, 0, false),
            Instruction::create_sj(OpCode::Jmp, 1),
            Instruction::create_abc(OpCode::Add, 1, 1, 0),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let plan = TracePlan {
            id: TraceId(4),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 5,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::EqK,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 2,
                    opcode: OpCode::GtI,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 4,
                    opcode: OpCode::Add,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 5,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 5,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 2,
                    resume_pc: 4,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Control,
                    kind: TraceGuardKind::Eq,
                    operands: TraceGuardOperands::RegisterConstant {
                        reg: 0,
                        constant_index: 0,
                    },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 2,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 2,
                },
                TraceGuard {
                    pc: 2,
                    mode: TraceGuardMode::Control,
                    kind: TraceGuardKind::Lt,
                    operands: TraceGuardOperands::ImmediateRegister { imm: 5, reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 2,
                },
            ],
            exits: vec![
                crate::lua_vm::jit::TraceExit {
                    kind: crate::lua_vm::jit::TraceExitKind::GuardExit,
                    source_pc: 0,
                    target_pc: 5,
                    snapshot_index: 1,
                    side_trace: None,
                    actions: Vec::new(),
                },
                crate::lua_vm::jit::TraceExit {
                    kind: crate::lua_vm::jit::TraceExitKind::GuardExit,
                    source_pc: 2,
                    target_pc: 4,
                    snapshot_index: 2,
                    side_trace: None,
                    actions: Vec::new(),
                },
            ],
        };

        let next_pc = execute_trace(
            state,
            &chunk,
            &plan,
            0,
            JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 1,
            },
        )
        .expect("replay should handle constant and immediate guards");

        assert_eq!(next_pc.next_pc, 0);
        assert_eq!(next_pc.outcome, TraceRunOutcome::Anchored);
        assert_eq!(state.stack()[1].as_integer_strict(), Some(16));
    }

    #[test]
    fn replay_and_compiled_support_forward_merge_jump() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = LuaValue::integer(0);
        state.stack_mut()[1] = LuaValue::integer(0);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::EqI, 1, 127, 0, false),
            Instruction::create_sj(OpCode::Jmp, 3),
            Instruction::create_abc(OpCode::AddI, 0, 0, 127 + 1),
            Instruction::create_abck(
                OpCode::MmBinI,
                0,
                127 + 1,
                crate::lua_vm::TmKind::Add as u32,
                false,
            ),
            Instruction::create_sj(OpCode::Jmp, 2),
            Instruction::create_abc(OpCode::AddI, 0, 0, 127 - 1),
            Instruction::create_abck(
                OpCode::MmBinI,
                0,
                127 + 1,
                crate::lua_vm::TmKind::Sub as u32,
                false,
            ),
            Instruction::create_sj(OpCode::Jmp, -8),
        ];

        let plan = TracePlan {
            id: TraceId(31),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 7,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::EqI,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 2,
                    opcode: OpCode::AddI,
                    line: None,
                    fallback: Some(TraceFallback::MmBinI {
                        imm: 1,
                        tm: crate::lua_vm::TmKind::Add,
                        flip: false,
                    }),
                },
                TraceInstruction {
                    pc: 4,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 7,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 5,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 2,
                    resume_pc: 2,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Control,
                    kind: TraceGuardKind::Eq,
                    operands: TraceGuardOperands::RegisterImmediate { reg: 1, imm: 0 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 2,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 0 },
                    continue_when: true,
                    exit_snapshot_index: 2,
                },
            ],
            exits: vec![
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 0,
                    target_pc: 5,
                    snapshot_index: 1,
                    side_trace: None,
                    actions: Vec::new(),
                },
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 2,
                    target_pc: 2,
                    snapshot_index: 2,
                    side_trace: None,
                    actions: Vec::new(),
                },
            ],
        };

        let replay_result = execute_trace(
            state,
            &chunk,
            &plan,
            0,
            JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 3,
            },
        )
        .expect("replay should handle forward merge jump");

        assert_eq!(replay_result.outcome, TraceRunOutcome::Anchored);
        assert_eq!(state.stack()[0].as_integer_strict(), Some(3));

        state.stack_mut()[0] = LuaValue::integer(0);
        let compiled = LoweredTraceBackend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("lowered compile should succeed")
            .expect("compiled artifact should exist");
        let TraceArtifact::Compiled(compiled) = compiled else {
            panic!("expected lowered compiled artifact");
        };

        let compiled_result = execute_compiled_trace(
            state,
            &chunk,
            &compiled,
            0,
            JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 3,
            },
        )
        .expect("compiled execution should handle forward merge jump");

        assert_eq!(compiled_result.outcome, TraceRunOutcome::Anchored);
        assert_eq!(state.stack()[0].as_integer_strict(), Some(3));
    }
}
