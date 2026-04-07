use super::helper_plan::HelperPlan;
use super::ir::{TraceIr, TraceIrGuardKind, TraceIrOperand};
use super::trace_recorder::TraceArtifact;
use crate::gc::UpvaluePtr;
use crate::lua_value::LuaValue;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotOperand {
    Register(u32),
    RegisterRange { start: u32, count: u32 },
    ConstantIndex(u32),
    Upvalue(u32),
    SignedImmediate(i32),
    UnsignedImmediate(u32),
    Bool(bool),
    JumpTarget(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeoptSnapshot {
    pub id: u16,
    pub resume_pc: u32,
    pub operands: Vec<SnapshotOperand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoweredExit {
    pub exit_index: u16,
    pub guard_pc: u32,
    pub branch_pc: u32,
    pub exit_pc: u32,
    pub resume_pc: u32,
    pub snapshot_id: u16,
    pub is_loop_backedge: bool,
    pub restore_summary: DeoptRestoreSummary,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeoptRestoreSummary {
    pub register_count: u16,
    pub register_range_count: u16,
    pub upvalue_count: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoweredTrace {
    pub root_pc: u32,
    pub loop_tail_pc: u32,
    pub snapshots: Vec<DeoptSnapshot>,
    pub exits: Vec<LoweredExit>,
    pub helper_plan_step_count: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeoptTarget {
    pub exit_index: u16,
    pub snapshot_id: u16,
    pub resume_pc: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MaterializedSnapshotOperand {
    Register { reg: u32, value: LuaValue },
    RegisterRange { start: u32, values: Vec<LuaValue> },
    Constant { index: u32, value: LuaValue },
    Upvalue { index: u32, value: LuaValue },
    SignedImmediate(i32),
    UnsignedImmediate(u32),
    Bool(bool),
    JumpTarget(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaterializedSnapshot {
    pub id: u16,
    pub resume_pc: u32,
    pub operands: Vec<MaterializedSnapshotOperand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeoptRecovery {
    pub target: DeoptTarget,
    pub snapshot: MaterializedSnapshot,
    pub register_restores: Vec<(u32, LuaValue)>,
    pub register_range_restores: Vec<(u32, Vec<LuaValue>)>,
    pub upvalue_restores: Vec<(u32, LuaValue)>,
}

impl LoweredTrace {
    pub(crate) fn lower(
        artifact: &TraceArtifact,
        ir: &TraceIr,
        helper_plan: &HelperPlan,
    ) -> Self {
        let snapshot_operands = collect_snapshot_operands(ir);
        let restore_summary = summarize_snapshot_operands(&snapshot_operands);
        let mut snapshots = Vec::with_capacity(ir.guards.len() + 1);
        snapshots.push(DeoptSnapshot {
            id: 0,
            resume_pc: ir.root_pc,
            operands: snapshot_operands.clone(),
        });

        let exits = ir
            .guards
            .iter()
            .enumerate()
            .map(|(index, guard)| {
                let snapshot_id = (index + 1) as u16;
                snapshots.push(DeoptSnapshot {
                    id: snapshot_id,
                    resume_pc: guard.exit_pc,
                    operands: snapshot_operands.clone(),
                });
                LoweredExit {
                    exit_index: index as u16,
                    guard_pc: guard.guard_pc,
                    branch_pc: guard.branch_pc,
                    exit_pc: guard.exit_pc,
                    resume_pc: guard.exit_pc,
                    snapshot_id,
                    is_loop_backedge: matches!(guard.kind, TraceIrGuardKind::LoopBackedgeGuard),
                    restore_summary,
                }
            })
            .collect();

        Self {
            root_pc: artifact.seed.start_pc,
            loop_tail_pc: artifact.loop_tail_pc,
            snapshots,
            exits,
            helper_plan_step_count: helper_plan.steps.len().min(u16::MAX as usize) as u16,
        }
    }

    pub(crate) fn deopt_target_for_exit_pc(&self, exit_pc: u32) -> Option<DeoptTarget> {
        let exit = self.exits.iter().find(|exit| exit.exit_pc == exit_pc)?;
        Some(DeoptTarget {
            exit_index: exit.exit_index,
            snapshot_id: exit.snapshot_id,
            resume_pc: exit.resume_pc,
        })
    }

    pub(crate) unsafe fn materialize_snapshot(
        &self,
        snapshot_id: u16,
        stack: *const LuaValue,
        base: usize,
        constants: &[LuaValue],
        upvalue_ptrs: *const UpvaluePtr,
    ) -> Option<MaterializedSnapshot> {
        let snapshot = self
            .snapshots
            .iter()
            .find(|snapshot| snapshot.id == snapshot_id)?;
        unsafe { materialize_snapshot(snapshot, stack, base, constants, upvalue_ptrs) }
    }

    pub(crate) unsafe fn recover_exit(
        &self,
        exit_pc: u32,
        stack: *const LuaValue,
        base: usize,
        constants: &[LuaValue],
        upvalue_ptrs: *const UpvaluePtr,
    ) -> Option<DeoptRecovery> {
        let target = self.deopt_target_for_exit_pc(exit_pc)?;
        let snapshot = unsafe {
            self.materialize_snapshot(target.snapshot_id, stack, base, constants, upvalue_ptrs)
        }?;
        Some(build_deopt_recovery(target, snapshot))
    }
}

fn build_deopt_recovery(target: DeoptTarget, snapshot: MaterializedSnapshot) -> DeoptRecovery {
    let mut register_restores = Vec::new();
    let mut register_range_restores = Vec::new();
    let mut upvalue_restores = Vec::new();

    for operand in &snapshot.operands {
        match operand {
            MaterializedSnapshotOperand::Register { reg, value } => {
                register_restores.push((*reg, *value));
            }
            MaterializedSnapshotOperand::RegisterRange { start, values } => {
                register_range_restores.push((*start, values.clone()));
            }
            MaterializedSnapshotOperand::Upvalue { index, value } => {
                upvalue_restores.push((*index, *value));
            }
            MaterializedSnapshotOperand::Constant { .. }
            | MaterializedSnapshotOperand::SignedImmediate(..)
            | MaterializedSnapshotOperand::UnsignedImmediate(..)
            | MaterializedSnapshotOperand::Bool(..)
            | MaterializedSnapshotOperand::JumpTarget(..) => {}
        }
    }

    DeoptRecovery {
        target,
        snapshot,
        register_restores,
        register_range_restores,
        upvalue_restores,
    }
}

unsafe fn materialize_snapshot(
    snapshot: &DeoptSnapshot,
    stack: *const LuaValue,
    base: usize,
    constants: &[LuaValue],
    upvalue_ptrs: *const UpvaluePtr,
) -> Option<MaterializedSnapshot> {
    let mut operands = Vec::with_capacity(snapshot.operands.len());

    for operand in &snapshot.operands {
        let materialized = match operand {
            SnapshotOperand::Register(reg) => MaterializedSnapshotOperand::Register {
                reg: *reg,
                value: unsafe { *stack.add(base + *reg as usize) },
            },
            SnapshotOperand::RegisterRange { start, count } => {
                let mut values = Vec::with_capacity(*count as usize);
                for offset in 0..*count as usize {
                    values.push(unsafe { *stack.add(base + *start as usize + offset) });
                }
                MaterializedSnapshotOperand::RegisterRange {
                    start: *start,
                    values,
                }
            }
            SnapshotOperand::ConstantIndex(index) => MaterializedSnapshotOperand::Constant {
                index: *index,
                value: *constants.get(*index as usize)?,
            },
            SnapshotOperand::Upvalue(index) => {
                if upvalue_ptrs.is_null() {
                    return None;
                }
                let upvalue_ptr = unsafe { *upvalue_ptrs.add(*index as usize) };
                let value = *upvalue_ptr.as_ref().data.get_value_ref();
                MaterializedSnapshotOperand::Upvalue {
                    index: *index,
                    value,
                }
            }
            SnapshotOperand::SignedImmediate(value) => {
                MaterializedSnapshotOperand::SignedImmediate(*value)
            }
            SnapshotOperand::UnsignedImmediate(value) => {
                MaterializedSnapshotOperand::UnsignedImmediate(*value)
            }
            SnapshotOperand::Bool(value) => MaterializedSnapshotOperand::Bool(*value),
            SnapshotOperand::JumpTarget(target) => MaterializedSnapshotOperand::JumpTarget(*target),
        };
        operands.push(materialized);
    }

    Some(MaterializedSnapshot {
        id: snapshot.id,
        resume_pc: snapshot.resume_pc,
        operands,
    })
}

fn collect_snapshot_operands(ir: &TraceIr) -> Vec<SnapshotOperand> {
    let mut operands = Vec::new();

    for inst in &ir.insts {
        for operand in &inst.reads {
            operands.push(map_operand(*operand));
        }
        for operand in &inst.writes {
            operands.push(map_operand(*operand));
        }
    }

    operands
}

fn summarize_snapshot_operands(operands: &[SnapshotOperand]) -> DeoptRestoreSummary {
    let mut summary = DeoptRestoreSummary::default();

    for operand in operands {
        match operand {
            SnapshotOperand::Register(_) => {
                summary.register_count = summary.register_count.saturating_add(1);
            }
            SnapshotOperand::RegisterRange { .. } => {
                summary.register_range_count = summary.register_range_count.saturating_add(1);
            }
            SnapshotOperand::Upvalue(_) => {
                summary.upvalue_count = summary.upvalue_count.saturating_add(1);
            }
            SnapshotOperand::ConstantIndex(_)
            | SnapshotOperand::SignedImmediate(_)
            | SnapshotOperand::UnsignedImmediate(_)
            | SnapshotOperand::Bool(_)
            | SnapshotOperand::JumpTarget(_) => {}
        }
    }

    summary
}

fn map_operand(operand: TraceIrOperand) -> SnapshotOperand {
    match operand {
        TraceIrOperand::Register(reg) => SnapshotOperand::Register(reg),
        TraceIrOperand::RegisterRange { start, count } => {
            SnapshotOperand::RegisterRange { start, count }
        }
        TraceIrOperand::ConstantIndex(index) => SnapshotOperand::ConstantIndex(index),
        TraceIrOperand::Upvalue(index) => SnapshotOperand::Upvalue(index),
        TraceIrOperand::SignedImmediate(value) => SnapshotOperand::SignedImmediate(value),
        TraceIrOperand::UnsignedImmediate(value) => SnapshotOperand::UnsignedImmediate(value),
        TraceIrOperand::Bool(value) => SnapshotOperand::Bool(value),
        TraceIrOperand::JumpTarget(target) => SnapshotOperand::JumpTarget(target),
    }
}

#[cfg(test)]
mod tests {
    use crate::Instruction;
    use crate::OpCode;
    use crate::lua_value::LuaValue;
    use crate::lua_vm::jit::helper_plan::HelperPlan;
    use crate::lua_vm::jit::ir::TraceIr;
    use crate::lua_vm::jit::lowering::{LoweredTrace, MaterializedSnapshotOperand};
    use crate::lua_vm::jit::trace_recorder::TraceRecorder;
    use crate::lua_value::LuaProto;

    #[test]
    fn lowering_creates_entry_and_exit_snapshots() {
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abck(OpCode::Test, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_abck(OpCode::Add, 0, 0, 1, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(lowered.root_pc, 0);
        assert_eq!(lowered.loop_tail_pc, artifact.loop_tail_pc);
        assert_eq!(lowered.exits.len(), 1);
        assert_eq!(lowered.snapshots.len(), 2);
        assert_eq!(lowered.snapshots[0].resume_pc, 0);
        assert_eq!(lowered.snapshots[1].resume_pc, lowered.exits[0].exit_pc);
        assert_eq!(lowered.exits[0].snapshot_id, 1);
        assert_eq!(lowered.exits[0].resume_pc, lowered.exits[0].exit_pc);
        assert!(lowered.exits[0].restore_summary.register_count > 0);
        assert_eq!(lowered.exits[0].restore_summary.register_range_count, 0);
        assert_eq!(lowered.exits[0].restore_summary.upvalue_count, 0);

        let deopt = lowered.deopt_target_for_exit_pc(lowered.exits[0].exit_pc).unwrap();
        assert_eq!(deopt.exit_index, 0);
        assert_eq!(deopt.snapshot_id, 1);
        assert_eq!(deopt.resume_pc, lowered.exits[0].exit_pc);

        let stack = [LuaValue::integer(11), LuaValue::integer(22), LuaValue::integer(33)];
        let materialized = unsafe {
            lowered.materialize_snapshot(
                deopt.snapshot_id,
                stack.as_ptr(),
                0,
                &chunk.constants,
                std::ptr::null(),
            )
        }
        .unwrap();
        assert_eq!(materialized.id, 1);
        assert_eq!(materialized.resume_pc, lowered.exits[0].exit_pc);
        assert!(materialized.operands.iter().any(|operand| matches!(
            operand,
            MaterializedSnapshotOperand::Register { reg: 0, value }
                if *value == LuaValue::integer(11)
        )));

        let recovery = unsafe {
            lowered.recover_exit(3, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.target.exit_index, 0);
        assert!(recovery
            .register_restores
            .iter()
            .any(|(reg, value)| *reg == 0 && *value == LuaValue::integer(11)));
    }
}