use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::helper_plan::HelperPlan;
use super::ir::{
    TraceIr, TraceIrGuardKind, TraceIrInst, TraceIrInstKind, TraceIrOperand, instruction_reads,
    instruction_writes, is_fused_arithmetic_metamethod_fallback,
};
use super::trace_recorder::TraceArtifact;
use crate::Instruction;
use crate::OpCode;
use crate::gc::UpvaluePtr;
use crate::lua_value::{LuaProto, LuaValue};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotOperand {
    Register(u32),
    RegisterRange {
        start: u32,
        count: u32,
    },
    RestoreRegisterFromRegister {
        reg: u32,
        source: u32,
    },
    RestoreRegisterFromConstantIndex {
        reg: u32,
        index: u32,
    },
    RestoreRegisterFromIntegerImmediate {
        reg: u32,
        value: i32,
    },
    RestoreRegisterFromIntegerAddImm {
        reg: u32,
        source: u32,
        offset: i32,
    },
    RestoreRegisterFromIntegerBitwiseK {
        reg: u32,
        source: u32,
        index: u32,
        op: IntegerBitwiseKOp,
    },
    RestoreRegisterFromIntegerShiftImm {
        reg: u32,
        source: u32,
        imm: i32,
        op: IntegerShiftImmOp,
    },
    RestoreRegisterFromFloatImmediate {
        reg: u32,
        value: i32,
    },
    RestoreRegisterFromBool {
        reg: u32,
        value: bool,
    },
    RestoreRegisterFromNil {
        reg: u32,
    },
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
    pub restore_operands: Vec<SnapshotOperand>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceValueKind {
    Unknown,
    Integer,
    Float,
    Numeric,
    Boolean,
    Table,
    Closure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegisterValueHint {
    pub reg: u32,
    pub kind: TraceValueKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ValueHintSummary {
    pub integer_count: u16,
    pub float_count: u16,
    pub numeric_count: u16,
    pub boolean_count: u16,
    pub table_count: u16,
    pub closure_count: u16,
    pub unknown_count: u16,
}

pub(crate) type SsaValueId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SsaValueOrigin {
    EntryRegister(u32),
    InstructionOutput { pc: u32, kind: TraceIrInstKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SsaValue {
    pub id: SsaValueId,
    pub kind: TraceValueKind,
    pub origin: SsaValueOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SsaOperand {
    Value(SsaValueId),
    ValueRange {
        start_reg: u32,
        values: Vec<SsaValueId>,
    },
    ConstantIndex(u32),
    Upvalue(u32),
    SignedImmediate(i32),
    UnsignedImmediate(u32),
    Bool(bool),
    JumpTarget(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SsaInstruction {
    pub pc: u32,
    pub kind: TraceIrInstKind,
    pub opcode: OpCode,
    pub read_operands: Vec<TraceIrOperand>,
    pub inputs: Vec<SsaOperand>,
    pub outputs: Vec<SsaValueId>,
    pub memory_effects: Vec<SsaMemoryEffect>,
    pub table_int_rewrite: Option<SsaTableIntRewrite>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SsaTableIntRewrite {
    ForwardFromRegister { reg: u32, value: SsaValueId },
    DeadStore,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SsaTableKey {
    Value(SsaValueId),
    AffineValue { base: SsaValueId, offset: i32 },
    ConstantIndex(u32),
    UnsignedImmediate(u32),
    SignedImmediate(i32),
    Bool(bool),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SsaTableIntRegion {
    pub table: SsaValueId,
    pub version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SsaMemoryEffect {
    TableRead {
        table: Option<SsaValueId>,
        key: SsaTableKey,
    },
    TableWrite {
        table: Option<SsaValueId>,
        key: SsaTableKey,
        value: Option<SsaValueId>,
    },
    TableIntRead {
        region: Option<SsaTableIntRegion>,
        key: SsaTableKey,
    },
    TableIntWrite {
        region: Option<SsaTableIntRegion>,
        key: SsaTableKey,
        value: Option<SsaValueId>,
    },
    UpvalueRead {
        index: u32,
    },
    UpvalueWrite {
        index: u32,
        value: Option<SsaValueId>,
    },
    Call,
    MetamethodFallback,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LoweredSsaTrace {
    pub values: Vec<SsaValue>,
    pub instructions: Vec<SsaInstruction>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SsaValueSummary {
    pub entry_count: u16,
    pub derived_count: u16,
    pub integer_count: u16,
    pub float_count: u16,
    pub numeric_count: u16,
    pub boolean_count: u16,
    pub table_count: u16,
    pub closure_count: u16,
    pub unknown_count: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SsaMemoryEffectSummary {
    pub table_read_count: u16,
    pub table_write_count: u16,
    pub table_int_read_count: u16,
    pub table_int_write_count: u16,
    pub upvalue_read_count: u16,
    pub upvalue_write_count: u16,
    pub call_count: u16,
    pub metamethod_count: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SsaTableIntOptimizationSummary {
    pub forwardable_read_count: u16,
    pub dead_store_count: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SsaTableIntVersionState {
    default_version: u32,
    next_version: u32,
    table_versions: BTreeMap<SsaValueId, u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LiveVmState {
    registers: BTreeSet<u32>,
    upvalues: BTreeSet<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntegerBitwiseKOp {
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntegerShiftImmOp {
    ShlI,
    ShrI,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegisterRestoreSource {
    Register(u32),
    ConstantIndex(u32),
    IntegerImmediate(i32),
    IntegerAddImm {
        source: u32,
        offset: i32,
    },
    IntegerBitwiseK {
        source: u32,
        index: u32,
        op: IntegerBitwiseKOp,
    },
    IntegerShiftImm {
        source: u32,
        imm: i32,
        op: IntegerShiftImmOp,
    },
    FloatImmediate(i32),
    Bool(bool),
    Nil,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoweredTrace {
    pub root_pc: u32,
    pub loop_tail_pc: u32,
    pub snapshots: Vec<DeoptSnapshot>,
    pub exits: Vec<LoweredExit>,
    pub helper_plan_step_count: u16,
    pub constants: Vec<LuaValue>,
    pub root_register_hints: Vec<RegisterValueHint>,
    pub entry_stable_register_hints: Vec<RegisterValueHint>,
    pub ssa_trace: LoweredSsaTrace,
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
    pub restore_operands: Vec<MaterializedSnapshotOperand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeoptRecovery {
    pub target: DeoptTarget,
    pub snapshot: MaterializedSnapshot,
    pub register_restores: Vec<(u32, LuaValue)>,
    pub register_range_restores: Vec<(u32, Vec<LuaValue>)>,
    pub upvalue_restores: Vec<(u32, LuaValue)>,
}

impl DeoptRecovery {
    #[cfg(test)]
    pub(crate) fn is_noop(&self) -> bool {
        self.register_restores.is_empty()
            && self.register_range_restores.is_empty()
            && self.upvalue_restores.is_empty()
    }

    #[allow(unsafe_op_in_unsafe_fn)]
    pub(crate) unsafe fn is_redundant_for_state(
        &self,
        stack: *const LuaValue,
        base: usize,
        upvalue_ptrs: *const UpvaluePtr,
    ) -> bool {
        for (reg, value) in &self.register_restores {
            if unsafe { *stack.add(base + *reg as usize) } != *value {
                return false;
            }
        }

        for (start, values) in &self.register_range_restores {
            for (offset, value) in values.iter().enumerate() {
                if unsafe { *stack.add(base + *start as usize + offset) } != *value {
                    return false;
                }
            }
        }

        if self.upvalue_restores.is_empty() {
            return true;
        }

        if upvalue_ptrs.is_null() {
            return false;
        }

        for (index, value) in &self.upvalue_restores {
            let upvalue_ptr = unsafe { *upvalue_ptrs.add(*index as usize) };
            if *upvalue_ptr.as_ref().data.get_value_ref() != *value {
                return false;
            }
        }

        true
    }
}

impl LoweredTrace {
    pub(crate) fn lower(artifact: &TraceArtifact, ir: &TraceIr, helper_plan: &HelperPlan) -> Self {
        let ssa_trace = build_minimal_ssa_trace(ir);
        let snapshot_operands = collect_snapshot_operands(ir);
        let chunk = unsafe { (artifact.seed.root_chunk_addr as *const LuaProto).as_ref() };
        let root_restore_operands =
            collect_restore_operands_for_resume_pc(ir, chunk, ir.root_pc, Some(&ssa_trace));
        let mut snapshots = Vec::with_capacity(ir.guards.len() + 1);
        snapshots.push(DeoptSnapshot {
            id: 0,
            resume_pc: ir.root_pc,
            operands: snapshot_operands.clone(),
            restore_operands: root_restore_operands,
        });

        let exits = ir
            .guards
            .iter()
            .enumerate()
            .map(|(index, guard)| {
                let snapshot_id = (index + 1) as u16;
                let restore_operands =
                    collect_restore_operands_for_exit(ir, chunk, guard, Some(&ssa_trace));
                let restore_summary = summarize_snapshot_operands(&restore_operands);
                snapshots.push(DeoptSnapshot {
                    id: snapshot_id,
                    resume_pc: guard.exit_pc,
                    operands: snapshot_operands.clone(),
                    restore_operands: restore_operands.clone(),
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
            constants: chunk
                .map(|chunk| chunk.constants.clone())
                .unwrap_or_default(),
            root_register_hints: collect_root_register_hints(ir),
            entry_stable_register_hints: collect_entry_stable_register_hints(ir),
            ssa_trace,
        }
    }

    pub(crate) fn integer_constant(&self, index: u32) -> Option<i32> {
        let value = self.constants.get(index as usize)?;
        let integer = value.as_integer_strict()?;
        i32::try_from(integer).ok()
    }

    pub(crate) fn float_constant(&self, index: u32) -> Option<f64> {
        self.constants.get(index as usize)?.as_float()
    }

    #[cfg(test)]
    pub(crate) fn register_value_kind(&self, reg: u32) -> Option<TraceValueKind> {
        self.root_register_hints
            .iter()
            .find(|hint| hint.reg == reg)
            .map(|hint| hint.kind)
    }

    pub(crate) fn root_value_hint_summary(&self) -> ValueHintSummary {
        let mut summary = ValueHintSummary::default();
        for hint in &self.root_register_hints {
            match hint.kind {
                TraceValueKind::Unknown => {
                    summary.unknown_count = summary.unknown_count.saturating_add(1);
                }
                TraceValueKind::Integer => {
                    summary.integer_count = summary.integer_count.saturating_add(1);
                }
                TraceValueKind::Float => {
                    summary.float_count = summary.float_count.saturating_add(1);
                }
                TraceValueKind::Numeric => {
                    summary.numeric_count = summary.numeric_count.saturating_add(1);
                }
                TraceValueKind::Boolean => {
                    summary.boolean_count = summary.boolean_count.saturating_add(1);
                }
                TraceValueKind::Table => {
                    summary.table_count = summary.table_count.saturating_add(1);
                }
                TraceValueKind::Closure => {
                    summary.closure_count = summary.closure_count.saturating_add(1);
                }
            }
        }
        summary
    }

    pub(crate) fn entry_stable_register_value_kind(&self, reg: u32) -> Option<TraceValueKind> {
        self.entry_stable_register_hints
            .iter()
            .find(|hint| hint.reg == reg)
            .map(|hint| hint.kind)
    }

    pub(crate) fn entry_register_value_kind(&self, reg: u32) -> Option<TraceValueKind> {
        self.ssa_trace
            .values
            .iter()
            .find_map(|value| match value.origin {
                SsaValueOrigin::EntryRegister(entry_reg) if entry_reg == reg => Some(value.kind),
                _ => None,
            })
    }

    pub(crate) fn entry_ssa_register_hints(&self) -> Vec<RegisterValueHint> {
        self.ssa_trace
            .values
            .iter()
            .filter_map(|value| match value.origin {
                SsaValueOrigin::EntryRegister(reg) => Some(RegisterValueHint {
                    reg,
                    kind: value.kind,
                }),
                SsaValueOrigin::InstructionOutput { .. } => None,
            })
            .collect()
    }

    pub(crate) fn ssa_value_summary(&self) -> SsaValueSummary {
        summarize_ssa_values(&self.ssa_trace.values)
    }

    pub(crate) fn ssa_memory_effect_summary(&self) -> SsaMemoryEffectSummary {
        summarize_ssa_memory_effects(&self.ssa_trace.instructions)
    }

    pub(crate) fn ssa_table_int_optimization_summary(&self) -> SsaTableIntOptimizationSummary {
        summarize_ssa_table_int_optimizations(&self.ssa_trace.instructions)
    }

    pub(crate) fn deopt_target_for_exit_pc(&self, exit_pc: u32) -> Option<DeoptTarget> {
        let exit = self.exits.iter().find(|exit| exit.exit_pc == exit_pc)?;
        Some(DeoptTarget {
            exit_index: exit.exit_index,
            snapshot_id: exit.snapshot_id,
            resume_pc: exit.resume_pc,
        })
    }

    pub(crate) fn deopt_target_for_exit_index(&self, exit_index: u16) -> Option<DeoptTarget> {
        let exit = self
            .exits
            .iter()
            .find(|exit| exit.exit_index == exit_index)?;
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

    pub(crate) unsafe fn recover_exit_by_index(
        &self,
        exit_index: u16,
        stack: *const LuaValue,
        base: usize,
        constants: &[LuaValue],
        upvalue_ptrs: *const UpvaluePtr,
    ) -> Option<DeoptRecovery> {
        let target = self.deopt_target_for_exit_index(exit_index)?;
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

    for operand in &snapshot.restore_operands {
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
    let mut restore_operands = Vec::with_capacity(snapshot.restore_operands.len());

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
            SnapshotOperand::RestoreRegisterFromRegister { reg, source } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: unsafe { *stack.add(base + *source as usize) },
                }
            }
            SnapshotOperand::RestoreRegisterFromConstantIndex { reg, index } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: *constants.get(*index as usize)?,
                }
            }
            SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg, value } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: LuaValue::integer(*value as i64),
                }
            }
            SnapshotOperand::RestoreRegisterFromIntegerAddImm {
                reg,
                source,
                offset,
            } => {
                let source_value = unsafe { *stack.add(base + *source as usize) };
                let value = if source_value.is_integer() {
                    LuaValue::integer(source_value.ivalue().saturating_add(*offset as i64))
                } else {
                    source_value
                };
                MaterializedSnapshotOperand::Register { reg: *reg, value }
            }
            SnapshotOperand::RestoreRegisterFromIntegerBitwiseK {
                reg,
                source,
                index,
                op,
            } => {
                let source_value = unsafe { *stack.add(base + *source as usize) };
                let constant_value = *constants.get(*index as usize)?;
                let value = materialize_integer_bitwise_k(source_value, constant_value, *op);
                MaterializedSnapshotOperand::Register { reg: *reg, value }
            }
            SnapshotOperand::RestoreRegisterFromIntegerShiftImm {
                reg,
                source,
                imm,
                op,
            } => {
                let source_value = unsafe { *stack.add(base + *source as usize) };
                let value = materialize_integer_shift_imm(source_value, *imm, *op);
                MaterializedSnapshotOperand::Register { reg: *reg, value }
            }
            SnapshotOperand::RestoreRegisterFromFloatImmediate { reg, value } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: LuaValue::float(*value as f64),
                }
            }
            SnapshotOperand::RestoreRegisterFromBool { reg, value } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: LuaValue::boolean(*value),
                }
            }
            SnapshotOperand::RestoreRegisterFromNil { reg } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: LuaValue::nil(),
                }
            }
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

    for operand in &snapshot.restore_operands {
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
            SnapshotOperand::RestoreRegisterFromRegister { reg, source } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: unsafe { *stack.add(base + *source as usize) },
                }
            }
            SnapshotOperand::RestoreRegisterFromConstantIndex { reg, index } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: *constants.get(*index as usize)?,
                }
            }
            SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg, value } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: LuaValue::integer(*value as i64),
                }
            }
            SnapshotOperand::RestoreRegisterFromIntegerAddImm {
                reg,
                source,
                offset,
            } => {
                let source_value = unsafe { *stack.add(base + *source as usize) };
                let value = if source_value.is_integer() {
                    LuaValue::integer(source_value.ivalue().saturating_add(*offset as i64))
                } else {
                    source_value
                };
                MaterializedSnapshotOperand::Register { reg: *reg, value }
            }
            SnapshotOperand::RestoreRegisterFromIntegerBitwiseK {
                reg,
                source,
                index,
                op,
            } => {
                let source_value = unsafe { *stack.add(base + *source as usize) };
                let constant_value = *constants.get(*index as usize)?;
                let value = materialize_integer_bitwise_k(source_value, constant_value, *op);
                MaterializedSnapshotOperand::Register { reg: *reg, value }
            }
            SnapshotOperand::RestoreRegisterFromIntegerShiftImm {
                reg,
                source,
                imm,
                op,
            } => {
                let source_value = unsafe { *stack.add(base + *source as usize) };
                let value = materialize_integer_shift_imm(source_value, *imm, *op);
                MaterializedSnapshotOperand::Register { reg: *reg, value }
            }
            SnapshotOperand::RestoreRegisterFromFloatImmediate { reg, value } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: LuaValue::float(*value as f64),
                }
            }
            SnapshotOperand::RestoreRegisterFromBool { reg, value } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: LuaValue::boolean(*value),
                }
            }
            SnapshotOperand::RestoreRegisterFromNil { reg } => {
                MaterializedSnapshotOperand::Register {
                    reg: *reg,
                    value: LuaValue::nil(),
                }
            }
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
        restore_operands.push(materialized);
    }

    Some(MaterializedSnapshot {
        id: snapshot.id,
        resume_pc: snapshot.resume_pc,
        operands,
        restore_operands,
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

#[cfg(test)]
fn collect_restore_operands(ir: &TraceIr) -> Vec<SnapshotOperand> {
    compact_restore_operands(collect_written_restore_operands(ir))
}

fn collect_written_restore_operands(ir: &TraceIr) -> Vec<SnapshotOperand> {
    collect_written_restore_operands_from_insts(ir.insts.iter(), None, None)
}

fn collect_restore_operands_for_resume_pc(
    ir: &TraceIr,
    chunk: Option<&LuaProto>,
    resume_pc: u32,
    ssa_trace: Option<&LoweredSsaTrace>,
) -> Vec<SnapshotOperand> {
    collect_restore_operands_for_operands(
        collect_written_restore_operands_from_insts(
            ir.insts.iter(),
            chunk.map(|chunk| chunk.constants.as_slice()),
            ssa_trace,
        ),
        chunk,
        resume_pc,
    )
}

fn collect_restore_operands_for_exit(
    ir: &TraceIr,
    chunk: Option<&LuaProto>,
    guard: &super::ir::TraceIrGuard,
    ssa_trace: Option<&LoweredSsaTrace>,
) -> Vec<SnapshotOperand> {
    let write_operands = if matches!(guard.kind, TraceIrGuardKind::SideExit) {
        collect_written_restore_operands_through_guard(
            ir,
            guard,
            chunk.map(|chunk| chunk.constants.as_slice()),
            ssa_trace,
        )
    } else {
        collect_written_restore_operands_from_insts(
            ir.insts.iter(),
            chunk.map(|chunk| chunk.constants.as_slice()),
            ssa_trace,
        )
    };

    collect_restore_operands_for_operands(write_operands, chunk, guard.exit_pc)
}

fn collect_restore_operands_for_operands(
    write_operands: Vec<SnapshotOperand>,
    chunk: Option<&LuaProto>,
    resume_pc: u32,
) -> Vec<SnapshotOperand> {
    let Some(chunk) = chunk else {
        return compact_restore_operands(write_operands);
    };

    let live = compute_live_vm_state_at_pc(chunk, resume_pc as usize);
    let mut filtered = Vec::new();

    for operand in write_operands {
        match operand {
            SnapshotOperand::Register(reg) => {
                if live.registers.contains(&reg) {
                    filtered.push(SnapshotOperand::Register(reg));
                }
            }
            SnapshotOperand::RestoreRegisterFromRegister { reg, source } => {
                if live.registers.contains(&reg) {
                    filtered.push(SnapshotOperand::RestoreRegisterFromRegister { reg, source });
                }
            }
            SnapshotOperand::RestoreRegisterFromConstantIndex { reg, index } => {
                if live.registers.contains(&reg) {
                    filtered.push(SnapshotOperand::RestoreRegisterFromConstantIndex { reg, index });
                }
            }
            SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg, value } => {
                if live.registers.contains(&reg) {
                    filtered
                        .push(SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg, value });
                }
            }
            SnapshotOperand::RestoreRegisterFromIntegerAddImm {
                reg,
                source,
                offset,
            } => {
                if live.registers.contains(&reg) {
                    filtered.push(SnapshotOperand::RestoreRegisterFromIntegerAddImm {
                        reg,
                        source,
                        offset,
                    });
                }
            }
            SnapshotOperand::RestoreRegisterFromFloatImmediate { reg, value } => {
                if live.registers.contains(&reg) {
                    filtered
                        .push(SnapshotOperand::RestoreRegisterFromFloatImmediate { reg, value });
                }
            }
            SnapshotOperand::RestoreRegisterFromBool { reg, value } => {
                if live.registers.contains(&reg) {
                    filtered.push(SnapshotOperand::RestoreRegisterFromBool { reg, value });
                }
            }
            SnapshotOperand::RestoreRegisterFromNil { reg } => {
                if live.registers.contains(&reg) {
                    filtered.push(SnapshotOperand::RestoreRegisterFromNil { reg });
                }
            }
            SnapshotOperand::RegisterRange { start, count } => {
                for reg in start..start.saturating_add(count) {
                    if live.registers.contains(&reg) {
                        filtered.push(SnapshotOperand::Register(reg));
                    }
                }
            }
            SnapshotOperand::Upvalue(index) => {
                if live.upvalues.contains(&index) {
                    filtered.push(SnapshotOperand::Upvalue(index));
                }
            }
            other => filtered.push(other),
        }
    }

    compact_restore_operands(filtered)
}

fn collect_written_restore_operands_through_guard(
    ir: &TraceIr,
    guard: &super::ir::TraceIrGuard,
    constants: Option<&[LuaValue]>,
    ssa_trace: Option<&LoweredSsaTrace>,
) -> Vec<SnapshotOperand> {
    let mut register_sources = BTreeMap::<u32, RegisterRestoreSource>::new();
    let mut current_hints = BTreeMap::<u32, TraceValueKind>::new();
    let mut upvalues = BTreeSet::<u32>::new();

    for inst in &ir.insts {
        if inst.pc > guard.guard_pc {
            break;
        }

        if inst.pc == guard.guard_pc {
            if conditional_writes_on_exit(inst, ir, guard).unwrap_or(true) {
                apply_written_restore_sources(
                    inst,
                    &current_hints,
                    constants,
                    ssa_trace,
                    &mut register_sources,
                    &mut upvalues,
                );
                apply_written_register_hints(inst, &mut current_hints);
            }
            if has_conditional_writes(inst.opcode) {
                break;
            }
        }

        apply_written_restore_sources(
            inst,
            &current_hints,
            constants,
            ssa_trace,
            &mut register_sources,
            &mut upvalues,
        );
        apply_written_register_hints(inst, &mut current_hints);
    }

    let mut operands = register_sources
        .into_iter()
        .map(|(reg, source)| restore_snapshot_operand_for_source(reg, source))
        .collect::<Vec<_>>();

    for index in upvalues {
        operands.push(SnapshotOperand::Upvalue(index));
    }

    operands
}

fn collect_written_restore_operands_from_insts<'a>(
    insts: impl IntoIterator<Item = &'a TraceIrInst>,
    constants: Option<&[LuaValue]>,
    ssa_trace: Option<&LoweredSsaTrace>,
) -> Vec<SnapshotOperand> {
    let mut register_sources = BTreeMap::<u32, RegisterRestoreSource>::new();
    let mut current_hints = BTreeMap::<u32, TraceValueKind>::new();
    let mut upvalues = BTreeSet::<u32>::new();

    for inst in insts {
        apply_written_restore_sources(
            inst,
            &current_hints,
            constants,
            ssa_trace,
            &mut register_sources,
            &mut upvalues,
        );
        apply_written_register_hints(inst, &mut current_hints);
    }

    let mut operands = register_sources
        .into_iter()
        .map(|(reg, source)| restore_snapshot_operand_for_source(reg, source))
        .collect::<Vec<_>>();

    for index in upvalues {
        operands.push(SnapshotOperand::Upvalue(index));
    }

    operands
}

fn apply_written_restore_sources(
    inst: &TraceIrInst,
    current_hints: &BTreeMap<u32, TraceValueKind>,
    constants: Option<&[LuaValue]>,
    ssa_trace: Option<&LoweredSsaTrace>,
    register_sources: &mut BTreeMap<u32, RegisterRestoreSource>,
    upvalues: &mut BTreeSet<u32>,
) {
    match inst.opcode {
        OpCode::SetUpval => {
            if let Some(TraceIrOperand::Upvalue(index)) = inst.writes.first() {
                upvalues.insert(*index);
            }
        }
        OpCode::LoadNil => {
            if let Some((start, count)) = written_register_range(inst) {
                for reg in start..start.saturating_add(count) {
                    register_sources.insert(reg, RegisterRestoreSource::Nil);
                }
            }
        }
        OpCode::ForLoop => {
            for operand in &inst.writes {
                if let TraceIrOperand::Register(reg) = operand {
                    register_sources.insert(*reg, RegisterRestoreSource::Register(*reg));
                }
            }
        }
        _ => {
            if let Some(reg) = single_written_register(inst) {
                register_sources.insert(
                    reg,
                    restore_source_for_single_write(
                        inst,
                        current_hints,
                        constants,
                        ssa_trace,
                        register_sources,
                    ),
                );
            }

            if let Some((start, count)) = written_register_range(inst) {
                for reg in start..start.saturating_add(count) {
                    register_sources.insert(reg, RegisterRestoreSource::Register(reg));
                }
            }
        }
    }
}

fn restore_source_for_single_write(
    inst: &TraceIrInst,
    current_hints: &BTreeMap<u32, TraceValueKind>,
    constants: Option<&[LuaValue]>,
    ssa_trace: Option<&LoweredSsaTrace>,
    register_sources: &BTreeMap<u32, RegisterRestoreSource>,
) -> RegisterRestoreSource {
    match inst.opcode {
        OpCode::Move => inst
            .reads
            .first()
            .and_then(|operand| match operand {
                TraceIrOperand::Register(reg) => Some(
                    register_sources
                        .get(reg)
                        .copied()
                        .unwrap_or(RegisterRestoreSource::Register(*reg)),
                ),
                _ => None,
            })
            .unwrap_or_else(|| {
                single_written_register(inst)
                    .map(RegisterRestoreSource::Register)
                    .unwrap_or(RegisterRestoreSource::Register(0))
            }),
        OpCode::LoadI => RegisterRestoreSource::IntegerImmediate(
            Instruction::from_u32(inst.raw_instruction).get_sbx(),
        ),
        OpCode::LoadF => RegisterRestoreSource::FloatImmediate(
            Instruction::from_u32(inst.raw_instruction).get_sbx(),
        ),
        OpCode::LoadK => RegisterRestoreSource::ConstantIndex(
            Instruction::from_u32(inst.raw_instruction).get_bx(),
        ),
        OpCode::LoadFalse => RegisterRestoreSource::Bool(false),
        OpCode::LoadTrue => RegisterRestoreSource::Bool(true),
        OpCode::AddI => {
            let raw = Instruction::from_u32(inst.raw_instruction);
            let source_reg = raw.get_b();
            let offset = raw.get_sc();
            if can_restore_integer_source(
                source_reg,
                inst,
                current_hints,
                register_sources,
                constants,
                ssa_trace,
            ) {
                register_sources
                    .get(&source_reg)
                    .copied()
                    .or(Some(RegisterRestoreSource::Register(source_reg)))
                    .and_then(|source| {
                        integer_restore_source_with_offset(source, offset, constants)
                    })
                    .unwrap_or_else(|| {
                        single_written_register(inst)
                            .map(RegisterRestoreSource::Register)
                            .unwrap_or(RegisterRestoreSource::Register(0))
                    })
            } else {
                single_written_register(inst)
                    .map(RegisterRestoreSource::Register)
                    .unwrap_or(RegisterRestoreSource::Register(0))
            }
        }
        OpCode::AddK | OpCode::SubK => {
            let raw = Instruction::from_u32(inst.raw_instruction);
            let source_reg = raw.get_b();
            let constant_offset = constants
                .and_then(|constants| integer_constant_at(constants, raw.get_c()))
                .map(|value| {
                    if inst.opcode == OpCode::SubK {
                        value.saturating_neg()
                    } else {
                        value
                    }
                });
            if can_restore_integer_source(
                source_reg,
                inst,
                current_hints,
                register_sources,
                constants,
                ssa_trace,
            ) {
                constant_offset
                    .and_then(|offset| {
                        register_sources
                            .get(&source_reg)
                            .copied()
                            .or(Some(RegisterRestoreSource::Register(source_reg)))
                            .and_then(|source| {
                                integer_restore_source_with_offset(source, offset, constants)
                            })
                    })
                    .unwrap_or_else(|| {
                        single_written_register(inst)
                            .map(RegisterRestoreSource::Register)
                            .unwrap_or(RegisterRestoreSource::Register(0))
                    })
            } else {
                single_written_register(inst)
                    .map(RegisterRestoreSource::Register)
                    .unwrap_or(RegisterRestoreSource::Register(0))
            }
        }
        OpCode::BAndK | OpCode::BOrK | OpCode::BXorK => {
            let raw = Instruction::from_u32(inst.raw_instruction);
            let source_reg = raw.get_b();
            let index = raw.get_c();
            if can_restore_integer_source(
                source_reg,
                inst,
                current_hints,
                register_sources,
                constants,
                ssa_trace,
            ) && constants
                .and_then(|constants| integer_constant_at(constants, index))
                .is_some()
            {
                RegisterRestoreSource::IntegerBitwiseK {
                    source: source_reg,
                    index,
                    op: match inst.opcode {
                        OpCode::BAndK => IntegerBitwiseKOp::And,
                        OpCode::BOrK => IntegerBitwiseKOp::Or,
                        OpCode::BXorK => IntegerBitwiseKOp::Xor,
                        _ => unreachable!(),
                    },
                }
            } else {
                single_written_register(inst)
                    .map(RegisterRestoreSource::Register)
                    .unwrap_or(RegisterRestoreSource::Register(0))
            }
        }
        OpCode::ShlI | OpCode::ShrI => {
            let raw = Instruction::from_u32(inst.raw_instruction);
            let source_reg = raw.get_b();
            let imm = raw.get_sc();
            if can_restore_integer_source(
                source_reg,
                inst,
                current_hints,
                register_sources,
                constants,
                ssa_trace,
            ) {
                RegisterRestoreSource::IntegerShiftImm {
                    source: source_reg,
                    imm,
                    op: match inst.opcode {
                        OpCode::ShlI => IntegerShiftImmOp::ShlI,
                        OpCode::ShrI => IntegerShiftImmOp::ShrI,
                        _ => unreachable!(),
                    },
                }
            } else {
                single_written_register(inst)
                    .map(RegisterRestoreSource::Register)
                    .unwrap_or(RegisterRestoreSource::Register(0))
            }
        }
        OpCode::TestSet => inst
            .reads
            .first()
            .and_then(|operand| match operand {
                TraceIrOperand::Register(reg) => Some(
                    register_sources
                        .get(reg)
                        .copied()
                        .unwrap_or(RegisterRestoreSource::Register(*reg)),
                ),
                _ => None,
            })
            .unwrap_or_else(|| {
                single_written_register(inst)
                    .map(RegisterRestoreSource::Register)
                    .unwrap_or(RegisterRestoreSource::Register(0))
            }),
        _ => single_written_register(inst)
            .map(RegisterRestoreSource::Register)
            .unwrap_or(RegisterRestoreSource::Register(0)),
    }
}

fn can_restore_integer_source(
    source_reg: u32,
    inst: &TraceIrInst,
    current_hints: &BTreeMap<u32, TraceValueKind>,
    register_sources: &BTreeMap<u32, RegisterRestoreSource>,
    constants: Option<&[LuaValue]>,
    ssa_trace: Option<&LoweredSsaTrace>,
) -> bool {
    if current_hints.get(&source_reg).copied() == Some(TraceValueKind::Integer) {
        return true;
    }

    match register_sources.get(&source_reg).copied() {
        Some(RegisterRestoreSource::IntegerImmediate(_))
        | Some(RegisterRestoreSource::IntegerAddImm { .. })
        | Some(RegisterRestoreSource::IntegerBitwiseK { .. })
        | Some(RegisterRestoreSource::IntegerShiftImm { .. }) => true,
        Some(RegisterRestoreSource::ConstantIndex(index)) => constants
            .and_then(|constants| integer_constant_at(constants, index))
            .is_some(),
        Some(RegisterRestoreSource::Register(_))
        | Some(RegisterRestoreSource::FloatImmediate(_))
        | Some(RegisterRestoreSource::Bool(_))
        | Some(RegisterRestoreSource::Nil)
        | None => ssa_trace.is_some_and(|ssa_trace| {
            ssa_integer_kind_for_register_read(ssa_trace, inst, source_reg)
        }),
    }
}

fn ssa_integer_kind_for_register_read(
    ssa_trace: &LoweredSsaTrace,
    inst: &TraceIrInst,
    source_reg: u32,
) -> bool {
    let Some(ssa_inst) = ssa_trace
        .instructions
        .iter()
        .find(|ssa_inst| ssa_inst.pc == inst.pc && ssa_inst.opcode == inst.opcode)
    else {
        return false;
    };

    ssa_inst
        .read_operands
        .iter()
        .zip(ssa_inst.inputs.iter())
        .find_map(|(operand, input)| match (operand, input) {
            (TraceIrOperand::Register(reg), SsaOperand::Value(value_id)) if *reg == source_reg => {
                ssa_trace
                    .values
                    .iter()
                    .find(|value| value.id == *value_id)
                    .map(|value| value.kind == TraceValueKind::Integer)
            }
            _ => None,
        })
        .unwrap_or(false)
}

fn integer_restore_source_with_offset(
    source: RegisterRestoreSource,
    offset: i32,
    constants: Option<&[LuaValue]>,
) -> Option<RegisterRestoreSource> {
    match source {
        RegisterRestoreSource::IntegerImmediate(value) => Some(
            RegisterRestoreSource::IntegerImmediate(value.saturating_add(offset)),
        ),
        RegisterRestoreSource::ConstantIndex(index) => integer_constant_at(constants?, index)
            .map(|value| RegisterRestoreSource::IntegerImmediate(value.saturating_add(offset))),
        RegisterRestoreSource::Register(source) => {
            Some(RegisterRestoreSource::IntegerAddImm { source, offset })
        }
        RegisterRestoreSource::IntegerAddImm {
            source,
            offset: base_offset,
        } => Some(RegisterRestoreSource::IntegerAddImm {
            source,
            offset: base_offset.saturating_add(offset),
        }),
        RegisterRestoreSource::IntegerBitwiseK { .. }
        | RegisterRestoreSource::IntegerShiftImm { .. }
        | RegisterRestoreSource::FloatImmediate(_)
        | RegisterRestoreSource::Bool(_)
        | RegisterRestoreSource::Nil => None,
    }
}

fn integer_constant_at(constants: &[LuaValue], index: u32) -> Option<i32> {
    let value = constants.get(index as usize)?;
    if value.is_integer() {
        i32::try_from(value.ivalue()).ok()
    } else {
        None
    }
}

fn has_conditional_writes(opcode: OpCode) -> bool {
    matches!(opcode, OpCode::TestSet | OpCode::ForLoop)
}

fn conditional_writes_on_exit(
    inst: &TraceIrInst,
    ir: &TraceIr,
    guard: &super::ir::TraceIrGuard,
) -> Option<bool> {
    match inst.opcode {
        OpCode::TestSet => Some(
            guard_branch_target_pc(ir, guard).is_some_and(|target_pc| target_pc == guard.exit_pc),
        ),
        OpCode::ForLoop => Some(guard.taken_on_trace),
        _ => None,
    }
}

fn guard_branch_target_pc(ir: &TraceIr, guard: &super::ir::TraceIrGuard) -> Option<u32> {
    let branch_inst = ir.insts.iter().find(|inst| inst.pc == guard.branch_pc)?;
    match branch_inst.opcode {
        OpCode::Jmp => {
            let instruction = Instruction::from_u32(branch_inst.raw_instruction);
            Some(((guard.branch_pc + 1) as i32 + instruction.get_sj()) as u32)
        }
        OpCode::ForLoop => Some(
            (guard.branch_pc + 1)
                .checked_sub(Instruction::from_u32(branch_inst.raw_instruction).get_bx())?,
        ),
        _ => None,
    }
}

fn restore_snapshot_operand_for_source(reg: u32, source: RegisterRestoreSource) -> SnapshotOperand {
    match source {
        RegisterRestoreSource::Register(source) if source == reg => SnapshotOperand::Register(reg),
        RegisterRestoreSource::Register(source) => {
            SnapshotOperand::RestoreRegisterFromRegister { reg, source }
        }
        RegisterRestoreSource::ConstantIndex(index) => {
            SnapshotOperand::RestoreRegisterFromConstantIndex { reg, index }
        }
        RegisterRestoreSource::IntegerImmediate(value) => {
            SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg, value }
        }
        RegisterRestoreSource::IntegerAddImm { source, offset } => {
            SnapshotOperand::RestoreRegisterFromIntegerAddImm {
                reg,
                source,
                offset,
            }
        }
        RegisterRestoreSource::IntegerBitwiseK { source, index, op } => {
            SnapshotOperand::RestoreRegisterFromIntegerBitwiseK {
                reg,
                source,
                index,
                op,
            }
        }
        RegisterRestoreSource::IntegerShiftImm { source, imm, op } => {
            SnapshotOperand::RestoreRegisterFromIntegerShiftImm {
                reg,
                source,
                imm,
                op,
            }
        }
        RegisterRestoreSource::FloatImmediate(value) => {
            SnapshotOperand::RestoreRegisterFromFloatImmediate { reg, value }
        }
        RegisterRestoreSource::Bool(value) => {
            SnapshotOperand::RestoreRegisterFromBool { reg, value }
        }
        RegisterRestoreSource::Nil => SnapshotOperand::RestoreRegisterFromNil { reg },
    }
}

fn materialize_integer_bitwise_k(
    source_value: LuaValue,
    constant_value: LuaValue,
    op: IntegerBitwiseKOp,
) -> LuaValue {
    if source_value.is_integer() && constant_value.is_integer() {
        let lhs = source_value.ivalue();
        let rhs = constant_value.ivalue();
        let result = match op {
            IntegerBitwiseKOp::And => lhs & rhs,
            IntegerBitwiseKOp::Or => lhs | rhs,
            IntegerBitwiseKOp::Xor => lhs ^ rhs,
        };
        LuaValue::integer(result)
    } else {
        source_value
    }
}

fn materialize_integer_shift_imm(
    source_value: LuaValue,
    imm: i32,
    op: IntegerShiftImmOp,
) -> LuaValue {
    if source_value.is_integer() {
        let source = source_value.ivalue();
        let result = match op {
            IntegerShiftImmOp::ShlI => {
                crate::lua_vm::execute::helper::lua_shiftl(imm as i64, source)
            }
            IntegerShiftImmOp::ShrI => {
                crate::lua_vm::execute::helper::lua_shiftr(source, imm as i64)
            }
        };
        LuaValue::integer(result)
    } else {
        source_value
    }
}

fn compact_restore_operands(operands: Vec<SnapshotOperand>) -> Vec<SnapshotOperand> {
    let mut register_ranges = operands
        .iter()
        .filter_map(|operand| match operand {
            SnapshotOperand::RegisterRange { start, count } => Some((*start, *count)),
            _ => None,
        })
        .collect::<Vec<_>>();
    register_ranges.sort_by_key(|(start, _)| *start);

    let mut merged_ranges = Vec::<(u32, u32)>::new();
    for (start, count) in register_ranges {
        let end = start.saturating_add(count);
        if let Some((existing_start, existing_count)) = merged_ranges.last_mut() {
            let existing_end = existing_start.saturating_add(*existing_count);
            if start <= existing_end {
                *existing_count = end.max(existing_end).saturating_sub(*existing_start);
                continue;
            }
        }
        merged_ranges.push((start, count));
    }

    let mut compacted = Vec::new();
    let mut emitted_ranges = BTreeSet::new();
    for operand in operands {
        match operand {
            SnapshotOperand::Register(reg)
                if merged_ranges
                    .iter()
                    .any(|(start, count)| reg >= *start && reg < start.saturating_add(*count)) =>
            {
                continue;
            }
            SnapshotOperand::RegisterRange { start, count } => {
                if emitted_ranges.insert((start, count)) {
                    compacted.push(SnapshotOperand::RegisterRange { start, count });
                }
            }
            other if !compacted.contains(&other) => compacted.push(other),
            _ => {}
        }
    }

    if emitted_ranges.len() != merged_ranges.len() {
        compacted.retain(|operand| !matches!(operand, SnapshotOperand::RegisterRange { .. }));
        for (start, count) in merged_ranges {
            compacted.push(SnapshotOperand::RegisterRange { start, count });
        }
    }

    compacted
}

fn summarize_snapshot_operands(operands: &[SnapshotOperand]) -> DeoptRestoreSummary {
    let mut summary = DeoptRestoreSummary::default();

    for operand in operands {
        match operand {
            SnapshotOperand::Register(_)
            | SnapshotOperand::RestoreRegisterFromRegister { .. }
            | SnapshotOperand::RestoreRegisterFromConstantIndex { .. }
            | SnapshotOperand::RestoreRegisterFromIntegerImmediate { .. }
            | SnapshotOperand::RestoreRegisterFromIntegerAddImm { .. }
            | SnapshotOperand::RestoreRegisterFromIntegerBitwiseK { .. }
            | SnapshotOperand::RestoreRegisterFromIntegerShiftImm { .. }
            | SnapshotOperand::RestoreRegisterFromFloatImmediate { .. }
            | SnapshotOperand::RestoreRegisterFromBool { .. }
            | SnapshotOperand::RestoreRegisterFromNil { .. } => {
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

fn compute_live_vm_state_at_pc(chunk: &LuaProto, resume_pc: usize) -> LiveVmState {
    let live = compute_live_vm_state(chunk);
    live.get(resume_pc).cloned().unwrap_or_default()
}

fn compute_live_vm_state(chunk: &LuaProto) -> Vec<LiveVmState> {
    let code_len = chunk.code.len();
    let mut live_in = vec![LiveVmState::default(); code_len + 1];
    let mut changed = true;

    while changed {
        changed = false;
        for pc in (0..code_len).rev() {
            let instruction = chunk.code[pc];
            let opcode = instruction.get_opcode();
            let mut next_state = LiveVmState::default();

            for succ in instruction_successors(pc, instruction, opcode, code_len) {
                merge_live_vm_state(&mut next_state, &live_in[succ]);
            }

            remove_defined_operands(
                &mut next_state,
                instruction_must_writes(instruction, opcode),
            );
            add_read_operands(
                &mut next_state,
                instruction_reads(pc as u32, instruction, opcode),
            );

            if live_in[pc] != next_state {
                live_in[pc] = next_state;
                changed = true;
            }
        }
    }

    live_in
}

fn merge_live_vm_state(target: &mut LiveVmState, source: &LiveVmState) {
    target.registers.extend(source.registers.iter().copied());
    target.upvalues.extend(source.upvalues.iter().copied());
}

fn add_read_operands(target: &mut LiveVmState, operands: Vec<TraceIrOperand>) {
    for operand in operands {
        match operand {
            TraceIrOperand::Register(reg) => {
                target.registers.insert(reg);
            }
            TraceIrOperand::RegisterRange { start, count } => {
                for reg in start..start.saturating_add(count) {
                    target.registers.insert(reg);
                }
            }
            TraceIrOperand::Upvalue(index) => {
                target.upvalues.insert(index);
            }
            TraceIrOperand::ConstantIndex(_)
            | TraceIrOperand::SignedImmediate(_)
            | TraceIrOperand::UnsignedImmediate(_)
            | TraceIrOperand::Bool(_)
            | TraceIrOperand::JumpTarget(_) => {}
        }
    }
}

fn remove_defined_operands(target: &mut LiveVmState, operands: Vec<TraceIrOperand>) {
    for operand in operands {
        match operand {
            TraceIrOperand::Register(reg) => {
                target.registers.remove(&reg);
            }
            TraceIrOperand::RegisterRange { start, count } => {
                for reg in start..start.saturating_add(count) {
                    target.registers.remove(&reg);
                }
            }
            TraceIrOperand::Upvalue(index) => {
                target.upvalues.remove(&index);
            }
            TraceIrOperand::ConstantIndex(_)
            | TraceIrOperand::SignedImmediate(_)
            | TraceIrOperand::UnsignedImmediate(_)
            | TraceIrOperand::Bool(_)
            | TraceIrOperand::JumpTarget(_) => {}
        }
    }
}

fn instruction_must_writes(instruction: Instruction, opcode: OpCode) -> Vec<TraceIrOperand> {
    match opcode {
        OpCode::TestSet | OpCode::ForLoop => Vec::new(),
        _ => instruction_writes(instruction, opcode),
    }
}

fn instruction_successors(
    pc: usize,
    instruction: Instruction,
    opcode: OpCode,
    code_len: usize,
) -> Vec<usize> {
    match opcode {
        OpCode::Return | OpCode::Return0 | OpCode::Return1 => Vec::new(),
        OpCode::Jmp => bounded_successors(&[jump_target_pc(pc, instruction, opcode)], code_len),
        OpCode::ForPrep | OpCode::TForPrep => {
            bounded_successors(&[jump_target_pc(pc, instruction, opcode)], code_len)
        }
        OpCode::ForLoop | OpCode::TForLoop => {
            bounded_successors(&[pc + 1, jump_target_pc(pc, instruction, opcode)], code_len)
        }
        OpCode::Eq
        | OpCode::Lt
        | OpCode::Le
        | OpCode::EqK
        | OpCode::EqI
        | OpCode::LtI
        | OpCode::LeI
        | OpCode::GtI
        | OpCode::GeI
        | OpCode::Test
        | OpCode::TestSet => bounded_successors(&[pc + 1, pc + 2], code_len),
        _ => bounded_successors(&[pc + 1], code_len),
    }
}

fn bounded_successors(candidates: &[usize], code_len: usize) -> Vec<usize> {
    let mut successors = Vec::new();
    for candidate in candidates {
        if *candidate <= code_len && !successors.contains(candidate) {
            successors.push(*candidate);
        }
    }
    successors
}

fn jump_target_pc(pc: usize, instruction: Instruction, opcode: OpCode) -> usize {
    match opcode {
        OpCode::Jmp => ((pc + 1) as i32 + instruction.get_sj()) as usize,
        OpCode::ForPrep | OpCode::TForPrep => pc + 1 + instruction.get_bx() as usize,
        OpCode::ForLoop | OpCode::TForLoop => pc + 1 - instruction.get_bx() as usize,
        _ => pc + 1,
    }
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
    use std::collections::BTreeMap;

    use crate::Instruction;
    use crate::OpCode;
    use crate::lua_value::LuaProto;
    use crate::lua_value::LuaValue;
    use crate::lua_vm::jit::helper_plan::HelperPlan;
    use crate::lua_vm::jit::ir::{
        TraceIr, TraceIrGuard, TraceIrGuardKind, TraceIrInst, TraceIrInstKind, TraceIrOperand,
    };
    use crate::lua_vm::jit::lowering::{
        DeoptRecovery, DeoptTarget, LoweredSsaTrace, LoweredTrace, MaterializedSnapshot,
        MaterializedSnapshotOperand, SnapshotOperand, SsaMemoryEffect, SsaOperand,
        SsaTableIntRegion, SsaTableIntRewrite, SsaTableKey, SsaValue, SsaValueOrigin,
        TraceValueKind,
    };
    use crate::lua_vm::jit::trace_recorder::TraceRecorder;

    use super::{
        IntegerBitwiseKOp, IntegerShiftImmOp, collect_restore_operands,
        collect_written_restore_operands_through_guard,
    };

    #[test]
    fn lowering_creates_entry_and_exit_snapshots() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 0, 0, 1, false));
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
        assert_eq!(
            lowered.register_value_kind(0),
            Some(TraceValueKind::Unknown)
        );
        assert_eq!(lowered.snapshots[0].resume_pc, 0);
        assert_eq!(lowered.snapshots[1].resume_pc, lowered.exits[0].exit_pc);
        assert_eq!(lowered.exits[0].snapshot_id, 1);
        assert_eq!(lowered.exits[0].resume_pc, lowered.exits[0].exit_pc);
        assert_eq!(lowered.exits[0].restore_summary.register_count, 1);
        assert_eq!(lowered.exits[0].restore_summary.register_range_count, 0);
        assert_eq!(lowered.exits[0].restore_summary.upvalue_count, 0);

        let deopt = lowered
            .deopt_target_for_exit_pc(lowered.exits[0].exit_pc)
            .unwrap();
        assert_eq!(deopt.exit_index, 0);
        assert_eq!(deopt.snapshot_id, 1);
        assert_eq!(deopt.resume_pc, lowered.exits[0].exit_pc);

        let stack = [
            LuaValue::integer(11),
            LuaValue::integer(22),
            LuaValue::integer(33),
        ];
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
        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromRegister { reg: 0, source: 1 }]
        );

        let recovery = unsafe {
            lowered.recover_exit(3, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.target.exit_index, 0);
        assert!(
            recovery
                .register_restores
                .iter()
                .any(|(reg, value)| *reg == 0 && *value == LuaValue::integer(22))
        );
        assert!(recovery.register_restores.iter().all(|(reg, _)| *reg != 1));
    }

    #[test]
    fn deopt_recovery_is_noop_when_no_vm_state_needs_restoring() {
        let recovery = DeoptRecovery {
            target: DeoptTarget {
                exit_index: 0,
                snapshot_id: 0,
                resume_pc: 7,
            },
            snapshot: MaterializedSnapshot {
                id: 0,
                resume_pc: 7,
                operands: vec![
                    MaterializedSnapshotOperand::Constant {
                        index: 0,
                        value: LuaValue::integer(1),
                    },
                    MaterializedSnapshotOperand::SignedImmediate(3),
                    MaterializedSnapshotOperand::Bool(true),
                ],
                restore_operands: Vec::new(),
            },
            register_restores: Vec::new(),
            register_range_restores: Vec::new(),
            upvalue_restores: Vec::new(),
        };

        assert!(recovery.is_noop());
    }

    #[test]
    fn deopt_recovery_is_redundant_for_matching_current_stack_state() {
        let recovery = DeoptRecovery {
            target: DeoptTarget {
                exit_index: 0,
                snapshot_id: 0,
                resume_pc: 7,
            },
            snapshot: MaterializedSnapshot {
                id: 0,
                resume_pc: 7,
                operands: vec![],
                restore_operands: vec![],
            },
            register_restores: vec![(0, LuaValue::integer(11))],
            register_range_restores: vec![(1, vec![LuaValue::integer(22), LuaValue::integer(33)])],
            upvalue_restores: Vec::new(),
        };
        let stack = [
            LuaValue::integer(11),
            LuaValue::integer(22),
            LuaValue::integer(33),
            LuaValue::integer(44),
        ];

        assert!(unsafe { recovery.is_redundant_for_state(stack.as_ptr(), 0, std::ptr::null()) });
        assert!(!unsafe { recovery.is_redundant_for_state(stack.as_ptr(), 1, std::ptr::null()) });
    }

    #[test]
    fn lowering_collects_root_register_value_hints() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 0, 4));
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadF, 1, 2));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 2, 0, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::LoadTrue, 3, 0, 0));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::Closure, 4, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::NewTable, 5, 0, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 6, 0, 2, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.register_value_kind(0),
            Some(TraceValueKind::Integer)
        );
        assert_eq!(lowered.register_value_kind(1), Some(TraceValueKind::Float));
        assert_eq!(
            lowered.register_value_kind(2),
            Some(TraceValueKind::Integer)
        );
        assert_eq!(
            lowered.register_value_kind(3),
            Some(TraceValueKind::Boolean)
        );
        assert_eq!(
            lowered.register_value_kind(4),
            Some(TraceValueKind::Closure)
        );
        assert_eq!(lowered.register_value_kind(5), Some(TraceValueKind::Table));
        assert_eq!(
            lowered.register_value_kind(6),
            Some(TraceValueKind::Numeric)
        );
    }

    #[test]
    fn lowering_tracks_entry_stable_register_hints_separately() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 0, 4));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 1, 0, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 0, 0, 1, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.register_value_kind(0),
            Some(TraceValueKind::Numeric)
        );
        assert_eq!(
            lowered.register_value_kind(1),
            Some(TraceValueKind::Integer)
        );
        assert_eq!(lowered.entry_stable_register_value_kind(0), None);
        assert_eq!(lowered.entry_stable_register_value_kind(1), None);
    }

    #[test]
    fn lowering_compacts_restore_operands_by_register_range() {
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 1,
            insts: vec![
                crate::lua_vm::jit::ir::TraceIrInst {
                    pc: 0,
                    opcode: OpCode::Move,
                    raw_instruction: Instruction::create_abc(OpCode::Move, 0, 1, 0).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                    reads: vec![TraceIrOperand::Register(1)],
                    writes: vec![TraceIrOperand::Register(0)],
                },
                crate::lua_vm::jit::ir::TraceIrInst {
                    pc: 1,
                    opcode: OpCode::LoadNil,
                    raw_instruction: Instruction::create_abc(OpCode::LoadNil, 0, 2, 0).as_u32(),
                    kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                    reads: vec![],
                    writes: vec![TraceIrOperand::RegisterRange { start: 0, count: 3 }],
                },
            ],
            guards: Vec::new(),
        };

        let restore_operands = collect_restore_operands(&ir);
        assert_eq!(
            restore_operands,
            vec![
                SnapshotOperand::RestoreRegisterFromNil { reg: 0 },
                SnapshotOperand::RestoreRegisterFromNil { reg: 1 },
                SnapshotOperand::RestoreRegisterFromNil { reg: 2 }
            ]
        );
    }

    #[test]
    fn lowering_prunes_exit_restore_operands_to_live_vm_state() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 2, 3, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 4, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(lowered.exits.len(), 1);
        assert_eq!(lowered.exits[0].resume_pc, 5);
        assert_eq!(lowered.exits[0].restore_summary.register_count, 1);
        assert_eq!(lowered.exits[0].restore_summary.register_range_count, 0);
        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromRegister { reg: 0, source: 1 }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(22),
            LuaValue::integer(33),
            LuaValue::integer(44),
            LuaValue::integer(55),
        ];
        let recovery = unsafe {
            lowered.recover_exit(5, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.register_restores, vec![(0, LuaValue::integer(22))]);
        assert!(recovery.register_restores.iter().all(|(reg, _)| *reg != 2));
    }

    #[test]
    fn lowering_restores_testset_write_when_side_exit_takes_branch_target() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 1, 2, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::TestSet, 0, 1, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 2));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 3, 4, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(lowered.exits.len(), 1);
        assert_eq!(lowered.exits[0].resume_pc, 5);
        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromRegister { reg: 0, source: 2 }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(111),
            LuaValue::integer(222),
            LuaValue::integer(333),
            LuaValue::integer(444),
        ];
        let recovery = unsafe {
            lowered.recover_exit(5, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(
            recovery.register_restores,
            vec![(0, LuaValue::integer(222))]
        );
    }

    #[test]
    fn lowering_skips_testset_write_when_side_exit_is_fallthrough_arm() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 1, 2, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::TestSet, 0, 1, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 2));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 3, 4, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(lowered.exits.len(), 1);
        assert_eq!(lowered.exits[0].resume_pc, 6);
        assert_eq!(
            lowered.snapshots[1].restore_operands,
            Vec::<SnapshotOperand>::new()
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(111),
            LuaValue::integer(222),
            LuaValue::integer(333),
            LuaValue::integer(444),
        ];
        let recovery = unsafe {
            lowered.recover_exit(6, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert!(recovery.register_restores.is_empty());
    }

    #[test]
    fn lowering_restores_live_register_from_integer_immediate_source() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 0, 7));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -4));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg: 0, value: 7 }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(22),
            LuaValue::integer(33),
        ];
        let recovery = unsafe {
            lowered.recover_exit(4, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.register_restores, vec![(0, LuaValue::integer(7))]);
    }

    #[test]
    fn lowering_restores_live_register_from_integer_addi_source() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 1, 7));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::AddI, 0, 1, 131));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg: 0, value: 11 }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(7),
            LuaValue::integer(33),
            LuaValue::integer(44),
        ];
        let recovery = unsafe {
            lowered.recover_exit(5, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.register_restores, vec![(0, LuaValue::integer(11))]);
    }

    #[test]
    fn lowering_restores_live_register_from_integer_addk_source() {
        let mut chunk = LuaProto::new();
        chunk.constants.push(LuaValue::integer(5));
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 1, 7));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::AddK, 0, 1, 0, false));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg: 0, value: 12 }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(7),
            LuaValue::integer(33),
            LuaValue::integer(44),
        ];
        let recovery = unsafe {
            lowered.recover_exit(5, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.register_restores, vec![(0, LuaValue::integer(12))]);
    }

    #[test]
    fn lowering_restores_live_register_from_integer_subk_source() {
        let mut chunk = LuaProto::new();
        chunk.constants.push(LuaValue::integer(20));
        chunk.constants.push(LuaValue::integer(3));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::LoadK, 1, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::SubK, 0, 1, 1, false));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerImmediate { reg: 0, value: 17 }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(20),
            LuaValue::integer(33),
            LuaValue::integer(44),
        ];
        let recovery = unsafe {
            lowered.recover_exit(5, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.register_restores, vec![(0, LuaValue::integer(17))]);
    }

    #[test]
    fn lowering_restores_live_register_from_integer_bandk_source() {
        let mut chunk = LuaProto::new();
        chunk.constants.push(LuaValue::integer(6));
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 1, 7));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::BAndK, 0, 1, 0, false));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerBitwiseK {
                reg: 0,
                source: 1,
                index: 0,
                op: IntegerBitwiseKOp::And,
            }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(7),
            LuaValue::integer(33),
            LuaValue::integer(44),
        ];
        let recovery = unsafe {
            lowered.recover_exit(5, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.register_restores, vec![(0, LuaValue::integer(6))]);
    }

    #[test]
    fn lowering_restores_live_register_from_integer_bork_source() {
        let mut chunk = LuaProto::new();
        chunk.constants.push(LuaValue::integer(8));
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 1, 7));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::BOrK, 0, 1, 0, false));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerBitwiseK {
                reg: 0,
                source: 1,
                index: 0,
                op: IntegerBitwiseKOp::Or,
            }]
        );
    }

    #[test]
    fn lowering_restores_live_register_from_integer_bxork_source() {
        let mut chunk = LuaProto::new();
        chunk.constants.push(LuaValue::integer(3));
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 1, 7));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::BXorK, 0, 1, 0, false));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerBitwiseK {
                reg: 0,
                source: 1,
                index: 0,
                op: IntegerBitwiseKOp::Xor,
            }]
        );
    }

    #[test]
    fn lowering_restores_live_register_from_integer_shri_source() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 1, 20));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::ShrI, 0, 1, 129));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerShiftImm {
                reg: 0,
                source: 1,
                imm: 2,
                op: IntegerShiftImmOp::ShrI,
            }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(20),
            LuaValue::integer(33),
            LuaValue::integer(44),
        ];
        let recovery = unsafe {
            lowered.recover_exit(5, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.register_restores, vec![(0, LuaValue::integer(5))]);
    }

    #[test]
    fn lowering_restores_live_register_from_integer_shli_source() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 1, 3));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::ShlI, 0, 1, 130));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromIntegerShiftImm {
                reg: 0,
                source: 1,
                imm: 3,
                op: IntegerShiftImmOp::ShlI,
            }]
        );
    }

    #[test]
    fn lowering_restores_forloop_write_only_when_loop_backedge_arm_executes() {
        let ir = TraceIr {
            root_pc: 0,
            loop_tail_pc: 0,
            insts: vec![TraceIrInst {
                pc: 0,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 1, 1).as_u32(),
                kind: TraceIrInstKind::LoopBackedge,
                reads: vec![],
                writes: vec![TraceIrOperand::Register(1), TraceIrOperand::Register(3)],
            }],
            guards: Vec::new(),
        };

        let taken_guard = TraceIrGuard {
            guard_pc: 0,
            branch_pc: 0,
            exit_pc: 0,
            taken_on_trace: true,
            kind: TraceIrGuardKind::LoopBackedgeGuard,
        };
        let fallthrough_guard = TraceIrGuard {
            guard_pc: 0,
            branch_pc: 0,
            exit_pc: 1,
            taken_on_trace: false,
            kind: TraceIrGuardKind::LoopBackedgeGuard,
        };

        assert_eq!(
            collect_written_restore_operands_through_guard(&ir, &taken_guard, None, None),
            vec![SnapshotOperand::Register(1), SnapshotOperand::Register(3)]
        );
        assert_eq!(
            collect_written_restore_operands_through_guard(&ir, &fallthrough_guard, None, None),
            Vec::<SnapshotOperand>::new()
        );
    }

    #[test]
    fn lowering_uses_ssa_integer_input_kind_for_restore_source_selection() {
        let inst = TraceIrInst {
            pc: 7,
            opcode: OpCode::AddI,
            raw_instruction: Instruction::create_abc(OpCode::AddI, 0, 1, 131).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![
                TraceIrOperand::Register(1),
                TraceIrOperand::SignedImmediate(4),
            ],
            writes: vec![TraceIrOperand::Register(0)],
        };
        let ssa_trace = LoweredSsaTrace {
            values: vec![SsaValue {
                id: 0,
                kind: TraceValueKind::Integer,
                origin: SsaValueOrigin::EntryRegister(1),
            }],
            instructions: vec![crate::lua_vm::jit::lowering::SsaInstruction {
                pc: 7,
                kind: TraceIrInstKind::Arithmetic,
                opcode: OpCode::AddI,
                read_operands: inst.reads.clone(),
                inputs: vec![SsaOperand::Value(0), SsaOperand::SignedImmediate(4)],
                outputs: vec![],
                memory_effects: vec![],
                table_int_rewrite: None,
            }],
        };

        assert_eq!(
            super::restore_source_for_single_write(
                &inst,
                &BTreeMap::new(),
                None,
                Some(&ssa_trace),
                &BTreeMap::new(),
            ),
            super::RegisterRestoreSource::IntegerAddImm {
                source: 1,
                offset: 4
            }
        );
    }

    #[test]
    fn lowering_restores_live_register_from_constant_source() {
        let mut chunk = LuaProto::new();
        chunk.constants.push(LuaValue::integer(41));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::LoadK, 0, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -4));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.snapshots[1].restore_operands,
            vec![SnapshotOperand::RestoreRegisterFromConstantIndex { reg: 0, index: 0 }]
        );

        let stack = [
            LuaValue::integer(999),
            LuaValue::integer(22),
            LuaValue::integer(33),
        ];
        let recovery = unsafe {
            lowered.recover_exit(4, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert_eq!(recovery.register_restores, vec![(0, LuaValue::integer(41))]);
    }

    #[test]
    fn lowering_ignores_writes_after_side_exit_guard_when_building_restore_set() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 2, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 3));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 4, 5, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 6, 7, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -6));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 4, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(lowered.exits.len(), 1);
        assert_eq!(lowered.exits[0].guard_pc, 1);
        assert_eq!(lowered.exits[0].resume_pc, 6);
        assert_eq!(
            lowered.snapshots[1].restore_operands,
            Vec::<SnapshotOperand>::new()
        );
        assert_eq!(lowered.exits[0].restore_summary.register_count, 0);

        let stack = [
            LuaValue::integer(11),
            LuaValue::integer(22),
            LuaValue::integer(33),
            LuaValue::integer(44),
            LuaValue::integer(55),
            LuaValue::integer(66),
            LuaValue::integer(77),
            LuaValue::integer(88),
        ];
        let recovery = unsafe {
            lowered.recover_exit(6, stack.as_ptr(), 0, &chunk.constants, std::ptr::null())
        }
        .unwrap();
        assert!(recovery.register_restores.is_empty());
    }

    #[test]
    fn lowering_builds_minimal_ssa_trace() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 0, 4));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 1, 0, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 2, 0, 1, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(lowered.ssa_trace.instructions.len(), 4);
        assert_eq!(lowered.ssa_trace.values.len(), 3);
        assert!(
            lowered
                .ssa_trace
                .values
                .iter()
                .all(|value| matches!(value.origin, SsaValueOrigin::InstructionOutput { .. }))
        );
        assert_eq!(lowered.ssa_value_summary().entry_count, 0);
        assert_eq!(lowered.ssa_value_summary().derived_count, 3);
        assert_eq!(lowered.ssa_value_summary().integer_count, 2);
        assert_eq!(lowered.ssa_value_summary().numeric_count, 1);
        assert_eq!(
            lowered.ssa_trace.instructions[2].inputs,
            vec![SsaOperand::Value(0), SsaOperand::Value(1)]
        );
    }

    #[test]
    fn lowering_creates_entry_ssa_values_for_live_in_reads() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(lowered.ssa_trace.values.len(), 2);
        assert!(
            lowered
                .ssa_trace
                .values
                .iter()
                .any(|value| matches!(value.origin, SsaValueOrigin::EntryRegister(1)))
        );
        assert_eq!(lowered.ssa_value_summary().entry_count, 1);
        assert_eq!(lowered.ssa_value_summary().derived_count, 1);
        assert_eq!(
            lowered.ssa_trace.instructions[0].inputs,
            vec![SsaOperand::Value(0)]
        );
        assert_eq!(lowered.entry_ssa_register_hints().len(), 1);
        assert_eq!(lowered.entry_ssa_register_hints()[0].reg, 1);
    }

    #[test]
    fn lowering_attaches_ssa_memory_effects() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::GetTable, 0, 1, 2, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::GetI, 3, 1, 6));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::SetI, 1, 6, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::SetUpval, 0, 3, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.ssa_trace.instructions[0].memory_effects,
            vec![SsaMemoryEffect::TableIntRead {
                region: Some(SsaTableIntRegion {
                    table: 0,
                    version: 0,
                }),
                key: SsaTableKey::Value(1),
            }]
        );
        assert_eq!(
            lowered.ssa_trace.instructions[1].memory_effects,
            vec![SsaMemoryEffect::TableIntRead {
                region: Some(SsaTableIntRegion {
                    table: 0,
                    version: 0,
                }),
                key: SsaTableKey::UnsignedImmediate(6),
            }]
        );
        assert_eq!(
            lowered.ssa_trace.instructions[2].memory_effects,
            vec![SsaMemoryEffect::TableIntWrite {
                region: Some(SsaTableIntRegion {
                    table: 0,
                    version: 0,
                }),
                key: SsaTableKey::UnsignedImmediate(6),
                value: Some(2),
            }]
        );
        assert_eq!(
            lowered.ssa_trace.instructions[3].memory_effects,
            vec![SsaMemoryEffect::UpvalueWrite {
                index: 3,
                value: Some(2),
            }]
        );
        assert_eq!(lowered.ssa_memory_effect_summary().table_read_count, 0);
        assert_eq!(lowered.ssa_memory_effect_summary().table_int_read_count, 2);
        assert_eq!(lowered.ssa_memory_effect_summary().table_int_write_count, 1);
        assert_eq!(lowered.ssa_memory_effect_summary().upvalue_write_count, 1);
    }

    #[test]
    fn lowering_skips_fused_arithmetic_metamethod_in_ssa_trace() {
        let mut chunk = LuaProto::new();
        chunk.constants.push(LuaValue::integer(1));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::AddK, 0, 0, 0, false));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::MmBinK, 0, 0, 6, false));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::ForLoop, 0, 3));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(lowered.ssa_trace.instructions.len(), 2);
        assert!(
            lowered
                .ssa_trace
                .instructions
                .iter()
                .all(|instruction| !matches!(
                    instruction.opcode,
                    OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK
                ))
        );
        assert_eq!(lowered.ssa_memory_effect_summary().metamethod_count, 0);
    }

    #[test]
    fn lowering_bumps_ssa_table_int_region_version_after_generic_table_write() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::NewTable, 0, 0, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::GetI, 1, 0, 6));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::SetField, 0, 7, 1, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::GetI, 2, 0, 6));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.ssa_trace.instructions[1].memory_effects,
            vec![SsaMemoryEffect::TableIntRead {
                region: Some(SsaTableIntRegion {
                    table: 0,
                    version: 0,
                }),
                key: SsaTableKey::UnsignedImmediate(6),
            }]
        );
        assert_eq!(
            lowered.ssa_trace.instructions[3].memory_effects,
            vec![SsaMemoryEffect::TableIntRead {
                region: Some(SsaTableIntRegion {
                    table: 0,
                    version: 1,
                }),
                key: SsaTableKey::UnsignedImmediate(6),
            }]
        );
        assert_eq!(
            lowered
                .ssa_table_int_optimization_summary()
                .forwardable_read_count,
            0
        );
    }

    #[test]
    fn lowering_detects_ssa_table_int_forwardable_reads_and_dead_stores() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::NewTable, 0, 0, 0));
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 1, 11));
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 2, 22));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::SetI, 0, 6, 1));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::GetI, 3, 0, 6));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::SetI, 0, 7, 1));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::SetI, 0, 7, 2));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.ssa_trace.instructions[4].table_int_rewrite,
            Some(SsaTableIntRewrite::ForwardFromRegister { reg: 1, value: 1 })
        );
        assert_eq!(
            lowered.ssa_trace.instructions[5].table_int_rewrite,
            Some(SsaTableIntRewrite::DeadStore)
        );
        assert_eq!(lowered.ssa_trace.instructions[6].table_int_rewrite, None);
        assert_eq!(
            lowered
                .ssa_table_int_optimization_summary()
                .forwardable_read_count,
            1
        );
        assert_eq!(
            lowered
                .ssa_table_int_optimization_summary()
                .dead_store_count,
            1
        );
    }

    #[test]
    fn lowering_normalizes_ssa_table_keys_across_move_and_addi_aliases() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::NewTable, 0, 0, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 1, 0, 0));
        chunk
            .code
            .push(Instruction::create_asbx(OpCode::LoadI, 2, 4));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::AddI, 3, 2, 128));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 6, 3, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::GetTable, 4, 1, 3, false));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::GetTable, 5, 0, 6, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));
        let chunk_ptr = &chunk as *const LuaProto;

        let artifact = TraceRecorder::record_root(chunk_ptr, 0).unwrap();
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            lowered.ssa_trace.instructions[5].memory_effects,
            vec![SsaMemoryEffect::TableIntRead {
                region: Some(SsaTableIntRegion {
                    table: 0,
                    version: 0,
                }),
                key: SsaTableKey::AffineValue { base: 2, offset: 1 },
            }]
        );
        assert_eq!(
            lowered.ssa_trace.instructions[6].memory_effects,
            vec![SsaMemoryEffect::TableIntRead {
                region: Some(SsaTableIntRegion {
                    table: 0,
                    version: 0,
                }),
                key: SsaTableKey::AffineValue { base: 2, offset: 1 },
            }]
        );
    }
}

fn collect_root_register_hints(ir: &TraceIr) -> Vec<RegisterValueHint> {
    let mut hints = BTreeMap::new();

    for inst in &ir.insts {
        apply_written_register_hints(inst, &mut hints);
    }

    hints
        .into_iter()
        .map(|(reg, kind)| RegisterValueHint { reg, kind })
        .collect()
}

fn collect_entry_stable_register_hints(ir: &TraceIr) -> Vec<RegisterValueHint> {
    let written_registers = collect_written_registers(ir);
    collect_root_register_hints(ir)
        .into_iter()
        .filter(|hint| !written_registers.contains(&hint.reg))
        .collect()
}

fn build_minimal_ssa_trace(ir: &TraceIr) -> LoweredSsaTrace {
    let entry_hints = collect_entry_stable_register_hints(ir)
        .into_iter()
        .map(|hint| (hint.reg, hint.kind))
        .collect::<BTreeMap<_, _>>();
    let mut current_hints = BTreeMap::<u32, TraceValueKind>::new();
    let mut current_values = BTreeMap::<u32, SsaValueId>::new();
    let mut values = Vec::<SsaValue>::new();
    let mut instructions = Vec::with_capacity(ir.insts.len());
    let mut value_producers = BTreeMap::<SsaValueId, usize>::new();
    let mut table_int_versions = SsaTableIntVersionState::default();
    let mut next_value_id = 0u32;

    for (inst_index, inst) in ir.insts.iter().enumerate() {
        if is_fused_arithmetic_metamethod_fallback(&ir.insts, inst_index) {
            continue;
        }

        let inputs = inst
            .reads
            .iter()
            .map(|operand| {
                ssa_operand_for_read(
                    *operand,
                    &entry_hints,
                    &mut current_values,
                    &mut values,
                    &mut next_value_id,
                )
            })
            .collect::<Vec<_>>();

        apply_written_register_hints(inst, &mut current_hints);
        let outputs = collect_ssa_output_values(
            inst,
            &current_hints,
            &mut current_values,
            &mut values,
            &mut next_value_id,
        );
        let memory_effects = ssa_memory_effects_for_instruction(
            inst,
            &inputs,
            &value_producers,
            &instructions,
            &mut table_int_versions,
        );
        let instruction_index = instructions.len();
        for &output in &outputs {
            value_producers.insert(output, instruction_index);
        }

        instructions.push(SsaInstruction {
            pc: inst.pc,
            kind: inst.kind,
            opcode: inst.opcode,
            read_operands: inst.reads.clone(),
            inputs,
            outputs,
            memory_effects,
            table_int_rewrite: None,
        });
    }

    apply_ssa_table_int_rewrites(&mut instructions);

    LoweredSsaTrace {
        values,
        instructions,
    }
}

fn ssa_operand_for_read(
    operand: TraceIrOperand,
    entry_hints: &BTreeMap<u32, TraceValueKind>,
    current_values: &mut BTreeMap<u32, SsaValueId>,
    values: &mut Vec<SsaValue>,
    next_value_id: &mut SsaValueId,
) -> SsaOperand {
    match operand {
        TraceIrOperand::Register(reg) => SsaOperand::Value(ensure_entry_ssa_value(
            reg,
            entry_hints,
            current_values,
            values,
            next_value_id,
        )),
        TraceIrOperand::RegisterRange { start, count } => {
            let mut range_values = Vec::with_capacity(count as usize);
            for reg in start..start.saturating_add(count) {
                range_values.push(ensure_entry_ssa_value(
                    reg,
                    entry_hints,
                    current_values,
                    values,
                    next_value_id,
                ));
            }
            SsaOperand::ValueRange {
                start_reg: start,
                values: range_values,
            }
        }
        TraceIrOperand::ConstantIndex(index) => SsaOperand::ConstantIndex(index),
        TraceIrOperand::Upvalue(index) => SsaOperand::Upvalue(index),
        TraceIrOperand::SignedImmediate(value) => SsaOperand::SignedImmediate(value),
        TraceIrOperand::UnsignedImmediate(value) => SsaOperand::UnsignedImmediate(value),
        TraceIrOperand::Bool(value) => SsaOperand::Bool(value),
        TraceIrOperand::JumpTarget(target) => SsaOperand::JumpTarget(target),
    }
}

fn ensure_entry_ssa_value(
    reg: u32,
    entry_hints: &BTreeMap<u32, TraceValueKind>,
    current_values: &mut BTreeMap<u32, SsaValueId>,
    values: &mut Vec<SsaValue>,
    next_value_id: &mut SsaValueId,
) -> SsaValueId {
    if let Some(&value_id) = current_values.get(&reg) {
        return value_id;
    }

    let value_id = *next_value_id;
    *next_value_id = next_value_id.saturating_add(1);
    values.push(SsaValue {
        id: value_id,
        kind: entry_hints
            .get(&reg)
            .copied()
            .unwrap_or(TraceValueKind::Unknown),
        origin: SsaValueOrigin::EntryRegister(reg),
    });
    current_values.insert(reg, value_id);
    value_id
}

fn collect_ssa_output_values(
    inst: &TraceIrInst,
    current_hints: &BTreeMap<u32, TraceValueKind>,
    current_values: &mut BTreeMap<u32, SsaValueId>,
    values: &mut Vec<SsaValue>,
    next_value_id: &mut SsaValueId,
) -> Vec<SsaValueId> {
    let mut outputs = Vec::new();

    if let Some(reg) = single_written_register(inst) {
        outputs.push(push_ssa_output_value(
            inst,
            reg,
            current_hints,
            current_values,
            values,
            next_value_id,
        ));
    }

    if let Some((start, count)) = written_register_range(inst) {
        for reg in start..start.saturating_add(count) {
            outputs.push(push_ssa_output_value(
                inst,
                reg,
                current_hints,
                current_values,
                values,
                next_value_id,
            ));
        }
    }

    outputs
}

fn push_ssa_output_value(
    inst: &TraceIrInst,
    reg: u32,
    current_hints: &BTreeMap<u32, TraceValueKind>,
    current_values: &mut BTreeMap<u32, SsaValueId>,
    values: &mut Vec<SsaValue>,
    next_value_id: &mut SsaValueId,
) -> SsaValueId {
    let value_id = *next_value_id;
    *next_value_id = next_value_id.saturating_add(1);
    values.push(SsaValue {
        id: value_id,
        kind: current_hints
            .get(&reg)
            .copied()
            .unwrap_or(TraceValueKind::Unknown),
        origin: SsaValueOrigin::InstructionOutput {
            pc: inst.pc,
            kind: inst.kind,
        },
    });
    current_values.insert(reg, value_id);
    value_id
}

fn summarize_ssa_values(values: &[SsaValue]) -> SsaValueSummary {
    let mut summary = SsaValueSummary::default();
    for value in values {
        match value.origin {
            SsaValueOrigin::EntryRegister(_) => {
                summary.entry_count = summary.entry_count.saturating_add(1);
            }
            SsaValueOrigin::InstructionOutput { .. } => {
                summary.derived_count = summary.derived_count.saturating_add(1);
            }
        }

        match value.kind {
            TraceValueKind::Unknown => {
                summary.unknown_count = summary.unknown_count.saturating_add(1);
            }
            TraceValueKind::Integer => {
                summary.integer_count = summary.integer_count.saturating_add(1);
            }
            TraceValueKind::Float => {
                summary.float_count = summary.float_count.saturating_add(1);
            }
            TraceValueKind::Numeric => {
                summary.numeric_count = summary.numeric_count.saturating_add(1);
            }
            TraceValueKind::Boolean => {
                summary.boolean_count = summary.boolean_count.saturating_add(1);
            }
            TraceValueKind::Table => {
                summary.table_count = summary.table_count.saturating_add(1);
            }
            TraceValueKind::Closure => {
                summary.closure_count = summary.closure_count.saturating_add(1);
            }
        }
    }
    summary
}

fn ssa_memory_effects_for_instruction(
    inst: &TraceIrInst,
    inputs: &[SsaOperand],
    value_producers: &BTreeMap<SsaValueId, usize>,
    instructions: &[SsaInstruction],
    table_int_versions: &mut SsaTableIntVersionState,
) -> Vec<SsaMemoryEffect> {
    let raw = Instruction::from_u32(inst.raw_instruction);
    match inst.opcode {
        OpCode::GetTable if !raw.get_k() => vec![SsaMemoryEffect::TableIntRead {
            region: inputs
                .first()
                .and_then(ssa_operand_value_id)
                .map(|value_id| {
                    current_ssa_table_int_region(
                        normalize_ssa_passthrough_value(value_id, value_producers, instructions),
                        table_int_versions,
                    )
                }),
            key: inputs
                .get(1)
                .map(|operand| ssa_table_key_from_operand(operand, value_producers, instructions))
                .unwrap_or(SsaTableKey::Unknown),
        }],
        OpCode::GetI => vec![SsaMemoryEffect::TableIntRead {
            region: inputs
                .first()
                .and_then(ssa_operand_value_id)
                .map(|value_id| {
                    current_ssa_table_int_region(
                        normalize_ssa_passthrough_value(value_id, value_producers, instructions),
                        table_int_versions,
                    )
                }),
            key: inputs
                .get(1)
                .map(|operand| ssa_table_key_from_operand(operand, value_producers, instructions))
                .unwrap_or(SsaTableKey::Unknown),
        }],
        OpCode::GetTable | OpCode::GetField => vec![SsaMemoryEffect::TableRead {
            table: inputs
                .first()
                .and_then(ssa_operand_value_id)
                .map(|value_id| {
                    normalize_ssa_passthrough_value(value_id, value_producers, instructions)
                }),
            key: inputs
                .get(1)
                .map(|operand| ssa_table_key_from_operand(operand, value_producers, instructions))
                .unwrap_or(SsaTableKey::Unknown),
        }],
        OpCode::SetTable if !raw.get_k() => vec![SsaMemoryEffect::TableIntWrite {
            region: inputs
                .first()
                .and_then(ssa_operand_value_id)
                .map(|value_id| {
                    current_ssa_table_int_region(
                        normalize_ssa_passthrough_value(value_id, value_producers, instructions),
                        table_int_versions,
                    )
                }),
            key: inputs
                .get(1)
                .map(|operand| ssa_table_key_from_operand(operand, value_producers, instructions))
                .unwrap_or(SsaTableKey::Unknown),
            value: inputs.get(2).and_then(ssa_operand_value_id),
        }],
        OpCode::SetI => vec![SsaMemoryEffect::TableIntWrite {
            region: inputs
                .first()
                .and_then(ssa_operand_value_id)
                .map(|value_id| {
                    current_ssa_table_int_region(
                        normalize_ssa_passthrough_value(value_id, value_producers, instructions),
                        table_int_versions,
                    )
                }),
            key: inputs
                .get(1)
                .map(|operand| ssa_table_key_from_operand(operand, value_producers, instructions))
                .unwrap_or(SsaTableKey::Unknown),
            value: inputs.get(2).and_then(ssa_operand_value_id),
        }],
        OpCode::SetTable | OpCode::SetField => {
            let table = inputs
                .first()
                .and_then(ssa_operand_value_id)
                .map(|value_id| {
                    normalize_ssa_passthrough_value(value_id, value_producers, instructions)
                });
            let effects = vec![SsaMemoryEffect::TableWrite {
                table,
                key: inputs
                    .get(1)
                    .map(|operand| {
                        ssa_table_key_from_operand(operand, value_producers, instructions)
                    })
                    .unwrap_or(SsaTableKey::Unknown),
                value: inputs.get(2).and_then(ssa_operand_value_id),
            }];
            if let Some(table) = table {
                invalidate_ssa_table_int_region(table, table_int_versions);
            }
            effects
        }
        OpCode::GetUpval => inputs
            .iter()
            .find_map(|operand| match operand {
                SsaOperand::Upvalue(index) => Some(*index),
                _ => None,
            })
            .map(|index| vec![SsaMemoryEffect::UpvalueRead { index }])
            .unwrap_or_default(),
        OpCode::SetUpval => inputs
            .iter()
            .find_map(|operand| match operand {
                SsaOperand::Upvalue(index) => Some(*index),
                _ => None,
            })
            .map(|index| {
                vec![SsaMemoryEffect::UpvalueWrite {
                    index,
                    value: inputs.first().and_then(ssa_operand_value_id),
                }]
            })
            .unwrap_or_default(),
        OpCode::Call | OpCode::TForCall => vec![SsaMemoryEffect::Call],
        OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK => {
            vec![SsaMemoryEffect::MetamethodFallback]
        }
        _ => Vec::new(),
    }
}

fn ssa_operand_value_id(operand: &SsaOperand) -> Option<SsaValueId> {
    match operand {
        SsaOperand::Value(value_id) => Some(*value_id),
        SsaOperand::ValueRange { .. }
        | SsaOperand::ConstantIndex(_)
        | SsaOperand::Upvalue(_)
        | SsaOperand::SignedImmediate(_)
        | SsaOperand::UnsignedImmediate(_)
        | SsaOperand::Bool(_)
        | SsaOperand::JumpTarget(_) => None,
    }
}

fn ssa_table_key_from_operand(
    operand: &SsaOperand,
    value_producers: &BTreeMap<SsaValueId, usize>,
    instructions: &[SsaInstruction],
) -> SsaTableKey {
    match operand {
        SsaOperand::Value(value_id) => {
            let (base, offset) =
                normalize_ssa_affine_value(*value_id, value_producers, instructions);
            if offset == 0 {
                SsaTableKey::Value(base)
            } else {
                SsaTableKey::AffineValue { base, offset }
            }
        }
        SsaOperand::ConstantIndex(index) => SsaTableKey::ConstantIndex(*index),
        SsaOperand::UnsignedImmediate(value) => SsaTableKey::UnsignedImmediate(*value),
        SsaOperand::SignedImmediate(value) => SsaTableKey::SignedImmediate(*value),
        SsaOperand::Bool(value) => SsaTableKey::Bool(*value),
        SsaOperand::ValueRange { .. } | SsaOperand::Upvalue(_) | SsaOperand::JumpTarget(_) => {
            SsaTableKey::Unknown
        }
    }
}

fn normalize_ssa_passthrough_value(
    mut value_id: SsaValueId,
    value_producers: &BTreeMap<SsaValueId, usize>,
    instructions: &[SsaInstruction],
) -> SsaValueId {
    loop {
        let Some(&instruction_index) = value_producers.get(&value_id) else {
            return value_id;
        };
        let instruction = &instructions[instruction_index];
        if instruction.opcode != OpCode::Move {
            return value_id;
        }
        let Some(next_value_id) = instruction.inputs.first().and_then(ssa_operand_value_id) else {
            return value_id;
        };
        value_id = next_value_id;
    }
}

fn normalize_ssa_affine_value(
    mut value_id: SsaValueId,
    value_producers: &BTreeMap<SsaValueId, usize>,
    instructions: &[SsaInstruction],
) -> (SsaValueId, i32) {
    let mut offset = 0i32;

    loop {
        let Some(&instruction_index) = value_producers.get(&value_id) else {
            return (value_id, offset);
        };
        let instruction = &instructions[instruction_index];
        match instruction.opcode {
            OpCode::Move => {
                let Some(next_value_id) = instruction.inputs.first().and_then(ssa_operand_value_id)
                else {
                    return (value_id, offset);
                };
                value_id = next_value_id;
            }
            OpCode::AddI => {
                let Some(next_value_id) = instruction.inputs.first().and_then(ssa_operand_value_id)
                else {
                    return (value_id, offset);
                };
                let Some(imm) = instruction.inputs.get(1).and_then(|operand| match operand {
                    SsaOperand::SignedImmediate(value) => Some(*value),
                    _ => None,
                }) else {
                    return (value_id, offset);
                };
                value_id = next_value_id;
                offset = offset.saturating_add(imm);
            }
            _ => return (value_id, offset),
        }
    }
}

fn current_ssa_table_int_region(
    table: SsaValueId,
    table_int_versions: &SsaTableIntVersionState,
) -> SsaTableIntRegion {
    SsaTableIntRegion {
        table,
        version: table_int_versions
            .table_versions
            .get(&table)
            .copied()
            .unwrap_or(table_int_versions.default_version),
    }
}

fn invalidate_ssa_table_int_region(
    table: SsaValueId,
    table_int_versions: &mut SsaTableIntVersionState,
) {
    table_int_versions.next_version = table_int_versions.next_version.saturating_add(1);
    table_int_versions
        .table_versions
        .insert(table, table_int_versions.next_version);
}

fn summarize_ssa_memory_effects(instructions: &[SsaInstruction]) -> SsaMemoryEffectSummary {
    let mut summary = SsaMemoryEffectSummary::default();
    for instruction in instructions {
        for effect in &instruction.memory_effects {
            match effect {
                SsaMemoryEffect::TableRead { .. } => {
                    summary.table_read_count = summary.table_read_count.saturating_add(1);
                }
                SsaMemoryEffect::TableWrite { .. } => {
                    summary.table_write_count = summary.table_write_count.saturating_add(1);
                }
                SsaMemoryEffect::TableIntRead { .. } => {
                    summary.table_int_read_count = summary.table_int_read_count.saturating_add(1);
                }
                SsaMemoryEffect::TableIntWrite { .. } => {
                    summary.table_int_write_count = summary.table_int_write_count.saturating_add(1);
                }
                SsaMemoryEffect::UpvalueRead { .. } => {
                    summary.upvalue_read_count = summary.upvalue_read_count.saturating_add(1);
                }
                SsaMemoryEffect::UpvalueWrite { .. } => {
                    summary.upvalue_write_count = summary.upvalue_write_count.saturating_add(1);
                }
                SsaMemoryEffect::Call => {
                    summary.call_count = summary.call_count.saturating_add(1);
                }
                SsaMemoryEffect::MetamethodFallback => {
                    summary.metamethod_count = summary.metamethod_count.saturating_add(1);
                }
            }
        }
    }
    summary
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SsaTableIntSlotKey {
    region: SsaTableIntRegion,
    key: SsaTableKey,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SsaTableIntLastWrite {
    instruction_index: usize,
    value: Option<SsaValueId>,
    source_register: Option<u32>,
    read_since_write: bool,
}

fn apply_ssa_table_int_rewrites(instructions: &mut [SsaInstruction]) {
    let mut last_writes = BTreeMap::<SsaTableIntSlotKey, SsaTableIntLastWrite>::new();

    for instruction_index in 0..instructions.len() {
        let effects = instructions[instruction_index].memory_effects.clone();
        for effect in effects {
            match effect {
                SsaMemoryEffect::TableIntRead {
                    region: Some(region),
                    key,
                } if key != SsaTableKey::Unknown => {
                    let slot = SsaTableIntSlotKey { region, key };
                    if let Some(state) = last_writes.get_mut(&slot) {
                        if let (Some(value), Some(reg)) = (state.value, state.source_register) {
                            instructions[instruction_index].table_int_rewrite =
                                Some(SsaTableIntRewrite::ForwardFromRegister { reg, value });
                        }
                        state.read_since_write = true;
                    }
                }
                SsaMemoryEffect::TableIntWrite {
                    region: Some(region),
                    key,
                    value,
                } if key != SsaTableKey::Unknown => {
                    let slot = SsaTableIntSlotKey { region, key };
                    if let Some(previous) = last_writes.get(&slot)
                        && !previous.read_since_write
                    {
                        instructions[previous.instruction_index].table_int_rewrite =
                            Some(SsaTableIntRewrite::DeadStore);
                    }
                    last_writes.insert(
                        slot,
                        SsaTableIntLastWrite {
                            instruction_index,
                            value,
                            source_register: source_register_for_table_int_write(
                                &instructions[instruction_index],
                            ),
                            read_since_write: false,
                        },
                    );
                }
                SsaMemoryEffect::TableWrite { table: None, .. }
                | SsaMemoryEffect::Call
                | SsaMemoryEffect::MetamethodFallback => {
                    last_writes.clear();
                }
                _ => {}
            }
        }
    }
}

fn source_register_for_table_int_write(instruction: &SsaInstruction) -> Option<u32> {
    match instruction.read_operands.get(2) {
        Some(TraceIrOperand::Register(reg)) => Some(*reg),
        _ => None,
    }
}

fn summarize_ssa_table_int_optimizations(
    instructions: &[SsaInstruction],
) -> SsaTableIntOptimizationSummary {
    let mut summary = SsaTableIntOptimizationSummary::default();
    for instruction in instructions {
        match instruction.table_int_rewrite {
            Some(SsaTableIntRewrite::ForwardFromRegister { .. }) => {
                summary.forwardable_read_count = summary.forwardable_read_count.saturating_add(1);
            }
            Some(SsaTableIntRewrite::DeadStore) => {
                summary.dead_store_count = summary.dead_store_count.saturating_add(1);
            }
            None => {}
        }
    }

    summary
}

fn collect_written_registers(ir: &TraceIr) -> BTreeSet<u32> {
    let mut written = BTreeSet::new();
    for inst in &ir.insts {
        if let Some(reg) = single_written_register(inst) {
            written.insert(reg);
        }
        if let Some((start, count)) = written_register_range(inst) {
            for reg in start..start.saturating_add(count) {
                written.insert(reg);
            }
        }
    }
    written
}

fn apply_written_register_hints(inst: &TraceIrInst, hints: &mut BTreeMap<u32, TraceValueKind>) {
    match inst.opcode {
        OpCode::Move => {
            let Some(dst) = single_written_register(inst) else {
                return;
            };
            let kind = inst
                .reads
                .first()
                .and_then(|operand| operand_value_kind(*operand, hints))
                .unwrap_or(TraceValueKind::Unknown);
            hints.insert(dst, kind);
        }
        OpCode::LoadI => {
            if let Some(dst) = single_written_register(inst) {
                hints.insert(dst, TraceValueKind::Integer);
            }
        }
        OpCode::LoadF => {
            if let Some(dst) = single_written_register(inst) {
                hints.insert(dst, TraceValueKind::Float);
            }
        }
        OpCode::LoadFalse | OpCode::LoadTrue | OpCode::Not => {
            if let Some(dst) = single_written_register(inst) {
                hints.insert(dst, TraceValueKind::Boolean);
            }
        }
        OpCode::NewTable => {
            if let Some(dst) = single_written_register(inst) {
                hints.insert(dst, TraceValueKind::Table);
            }
        }
        OpCode::Closure => {
            if let Some(dst) = single_written_register(inst) {
                hints.insert(dst, TraceValueKind::Closure);
            }
        }
        OpCode::LoadK
        | OpCode::LoadKX
        | OpCode::GetUpval
        | OpCode::GetTabUp
        | OpCode::GetTable
        | OpCode::GetI
        | OpCode::GetField
        | OpCode::Len
        | OpCode::Concat
        | OpCode::TestSet => {
            if let Some(dst) = single_written_register(inst) {
                hints.insert(dst, TraceValueKind::Unknown);
            }
        }
        OpCode::LoadNil => {
            if let Some((start, count)) = written_register_range(inst) {
                for reg in start..start.saturating_add(count) {
                    hints.insert(reg, TraceValueKind::Unknown);
                }
            }
        }
        OpCode::Self_ => {
            if let Some((start, count)) = written_register_range(inst) {
                if count >= 1 {
                    hints.insert(start, TraceValueKind::Unknown);
                }
                if count >= 2 {
                    hints.insert(start + 1, TraceValueKind::Table);
                }
            }
        }
        OpCode::AddI
        | OpCode::AddK
        | OpCode::SubK
        | OpCode::MulK
        | OpCode::ModK
        | OpCode::PowK
        | OpCode::DivK
        | OpCode::IDivK
        | OpCode::Add
        | OpCode::Sub
        | OpCode::Mul
        | OpCode::Mod
        | OpCode::Pow
        | OpCode::Div
        | OpCode::IDiv
        | OpCode::Unm => {
            if let Some(dst) = single_written_register(inst) {
                hints.insert(dst, TraceValueKind::Numeric);
            }
        }
        OpCode::BAndK
        | OpCode::BOrK
        | OpCode::BXorK
        | OpCode::ShlI
        | OpCode::ShrI
        | OpCode::BAnd
        | OpCode::BOr
        | OpCode::BXor
        | OpCode::Shl
        | OpCode::Shr
        | OpCode::BNot => {
            if let Some(dst) = single_written_register(inst) {
                hints.insert(dst, TraceValueKind::Integer);
            }
        }
        _ => {}
    }
}

fn single_written_register(inst: &TraceIrInst) -> Option<u32> {
    match inst.writes.as_slice() {
        [TraceIrOperand::Register(reg)] => Some(*reg),
        _ => None,
    }
}

fn written_register_range(inst: &TraceIrInst) -> Option<(u32, u32)> {
    match inst.writes.as_slice() {
        [TraceIrOperand::RegisterRange { start, count }] => Some((*start, *count)),
        _ => None,
    }
}

fn operand_value_kind(
    operand: TraceIrOperand,
    hints: &BTreeMap<u32, TraceValueKind>,
) -> Option<TraceValueKind> {
    match operand {
        TraceIrOperand::Register(reg) => hints.get(&reg).copied(),
        TraceIrOperand::SignedImmediate(_) | TraceIrOperand::UnsignedImmediate(_) => {
            Some(TraceValueKind::Integer)
        }
        TraceIrOperand::Bool(_) => Some(TraceValueKind::Boolean),
        TraceIrOperand::JumpTarget(_)
        | TraceIrOperand::ConstantIndex(_)
        | TraceIrOperand::Upvalue(_)
        | TraceIrOperand::RegisterRange { .. } => None,
    }
}
