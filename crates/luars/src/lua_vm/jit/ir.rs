use crate::{Instruction, OpCode};

use super::trace_recorder::{TraceArtifact, TraceExitKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceIrInstKind {
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
pub(crate) enum TraceIrOperand {
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
pub(crate) struct TraceIrInst {
    pub pc: u32,
    pub opcode: OpCode,
    pub raw_instruction: u32,
    pub kind: TraceIrInstKind,
    pub reads: Vec<TraceIrOperand>,
    pub writes: Vec<TraceIrOperand>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceIrGuardKind {
    SideExit,
    LoopBackedgeGuard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TraceIrGuard {
    pub guard_pc: u32,
    pub branch_pc: u32,
    pub exit_pc: u32,
    pub taken_on_trace: bool,
    pub kind: TraceIrGuardKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TraceIr {
    pub root_pc: u32,
    pub loop_tail_pc: u32,
    pub insts: Vec<TraceIrInst>,
    pub guards: Vec<TraceIrGuard>,
}

impl TraceIr {
    pub(crate) fn lower(artifact: &TraceArtifact) -> Self {
        let insts = artifact
            .ops
            .iter()
            .map(|op| TraceIrInst {
                pc: op.pc,
                opcode: op.opcode,
                raw_instruction: op.instruction.as_u32(),
                kind: if op.pc == artifact.loop_tail_pc
                    && matches!(op.opcode, OpCode::Jmp | OpCode::ForLoop | OpCode::TForLoop)
                {
                    TraceIrInstKind::LoopBackedge
                } else {
                    classify_opcode(op.opcode)
                },
                reads: collect_reads(op.pc, op.instruction, op.opcode),
                writes: collect_writes(op.instruction, op.opcode),
            })
            .collect();

        let guards = artifact
            .exits
            .iter()
            .map(|exit| TraceIrGuard {
                guard_pc: exit.guard_pc,
                branch_pc: exit.branch_pc,
                exit_pc: exit.exit_pc,
                taken_on_trace: exit.taken_on_trace,
                kind: match exit.kind {
                    TraceExitKind::GuardExit if exit.taken_on_trace => {
                        TraceIrGuardKind::LoopBackedgeGuard
                    }
                    TraceExitKind::GuardExit => TraceIrGuardKind::SideExit,
                },
            })
            .collect();

        Self {
            root_pc: artifact.seed.start_pc,
            loop_tail_pc: artifact.loop_tail_pc,
            insts,
            guards,
        }
    }
}

pub(crate) fn is_fused_arithmetic_metamethod_fallback(
    insts: &[TraceIrInst],
    index: usize,
) -> bool {
    let Some(inst) = insts.get(index) else {
        return false;
    };

    if !matches!(inst.opcode, OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK) {
        return false;
    }

    let Some(previous) = index.checked_sub(1).and_then(|prev| insts.get(prev)) else {
        return false;
    };

    arithmetic_metamethod_pair_matches(previous, inst)
}

pub(crate) fn is_fused_arithmetic_metamethod_pair(
    arithmetic_opcode: OpCode,
    arithmetic_instruction: Instruction,
    metamethod_opcode: OpCode,
    metamethod_instruction: Instruction,
) -> bool {
    match arithmetic_opcode {
        OpCode::Add
        | OpCode::Sub
        | OpCode::Mul
        | OpCode::Div
        | OpCode::IDiv
        | OpCode::Mod
        | OpCode::Pow
        | OpCode::BAnd
        | OpCode::BOr
        | OpCode::BXor
        | OpCode::Shl
        | OpCode::Shr
            if !arithmetic_instruction.get_k() =>
        {
            metamethod_opcode == OpCode::MmBin
                && metamethod_instruction.get_a() == arithmetic_instruction.get_b()
                && metamethod_instruction.get_b() == arithmetic_instruction.get_c()
        }
        OpCode::AddI | OpCode::ShlI | OpCode::ShrI => {
            metamethod_opcode == OpCode::MmBinI
                && metamethod_instruction.get_a() == arithmetic_instruction.get_b()
                && metamethod_instruction.get_sb()
                    == arithmetic_instruction.get_sc().unsigned_abs() as i32
        }
        OpCode::AddK
        | OpCode::SubK
        | OpCode::MulK
        | OpCode::DivK
        | OpCode::IDivK
        | OpCode::ModK
        | OpCode::PowK
        | OpCode::BAndK
        | OpCode::BOrK
        | OpCode::BXorK => {
            metamethod_opcode == OpCode::MmBinK
                && metamethod_instruction.get_a() == arithmetic_instruction.get_b()
                && metamethod_instruction.get_b() == arithmetic_instruction.get_c()
        }
        _ => false,
    }
}

fn arithmetic_metamethod_pair_matches(
    arithmetic: &TraceIrInst,
    metamethod: &TraceIrInst,
) -> bool {
    is_fused_arithmetic_metamethod_pair(
        arithmetic.opcode,
        Instruction::from_u32(arithmetic.raw_instruction),
        metamethod.opcode,
        Instruction::from_u32(metamethod.raw_instruction),
    )
}

pub(crate) fn instruction_reads(
    pc: u32,
    instruction: Instruction,
    opcode: OpCode,
) -> Vec<TraceIrOperand> {
    collect_reads(pc, instruction, opcode)
}

pub(crate) fn instruction_writes(instruction: Instruction, opcode: OpCode) -> Vec<TraceIrOperand> {
    collect_writes(instruction, opcode)
}

fn rk_operand(slot: u32, is_const: bool) -> TraceIrOperand {
    if is_const {
        TraceIrOperand::ConstantIndex(slot)
    } else {
        TraceIrOperand::Register(slot)
    }
}

fn jump_target(pc: u32, instruction: Instruction, opcode: OpCode) -> TraceIrOperand {
    let target = match opcode {
        OpCode::Jmp => ((pc + 1) as i32 + instruction.get_sj()) as u32,
        OpCode::ForLoop => pc + 1 - instruction.get_bx(),
        OpCode::TForPrep => pc + 1 + instruction.get_bx(),
        OpCode::TForLoop => pc + 1 - instruction.get_bx(),
        _ => pc,
    };
    TraceIrOperand::JumpTarget(target)
}

fn collect_reads(pc: u32, instruction: Instruction, opcode: OpCode) -> Vec<TraceIrOperand> {
    match opcode {
        OpCode::Move => vec![TraceIrOperand::Register(instruction.get_b())],
        OpCode::LoadI | OpCode::LoadF => vec![TraceIrOperand::SignedImmediate(instruction.get_sbx())],
        OpCode::LoadK => vec![TraceIrOperand::ConstantIndex(instruction.get_bx())],
        OpCode::LoadKX => vec![TraceIrOperand::UnsignedImmediate(instruction.get_a())],
        OpCode::LoadFalse | OpCode::LoadTrue => vec![TraceIrOperand::Bool(opcode == OpCode::LoadTrue)],
        OpCode::LoadNil => vec![TraceIrOperand::UnsignedImmediate(instruction.get_b())],
        OpCode::GetUpval => vec![TraceIrOperand::Upvalue(instruction.get_b())],
        OpCode::SetUpval => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::Upvalue(instruction.get_b()),
        ],
        OpCode::Close => vec![TraceIrOperand::Register(instruction.get_a())],
        OpCode::GetTabUp => vec![
            TraceIrOperand::Upvalue(instruction.get_b()),
            TraceIrOperand::ConstantIndex(instruction.get_c()),
        ],
        OpCode::GetTable => vec![
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::Register(instruction.get_c()),
        ],
        OpCode::GetI => vec![
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::UnsignedImmediate(instruction.get_c()),
        ],
        OpCode::GetField => vec![
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::ConstantIndex(instruction.get_c()),
        ],
        OpCode::SetTabUp => vec![
            TraceIrOperand::Upvalue(instruction.get_a()),
            TraceIrOperand::ConstantIndex(instruction.get_b()),
            rk_operand(instruction.get_c(), instruction.get_k()),
        ],
        OpCode::SetTable => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::Register(instruction.get_b()),
            rk_operand(instruction.get_c(), instruction.get_k()),
        ],
        OpCode::SetI => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::UnsignedImmediate(instruction.get_b()),
            rk_operand(instruction.get_c(), instruction.get_k()),
        ],
        OpCode::SetField => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::ConstantIndex(instruction.get_b()),
            rk_operand(instruction.get_c(), instruction.get_k()),
        ],
        OpCode::SetList => {
            let mut reads = vec![
                TraceIrOperand::Register(instruction.get_a()),
                TraceIrOperand::UnsignedImmediate(instruction.get_vc()),
                TraceIrOperand::Bool(instruction.get_k()),
            ];
            let vb = instruction.get_vb();
            reads.push(TraceIrOperand::UnsignedImmediate(vb));
            if vb > 0 {
                reads.push(TraceIrOperand::RegisterRange {
                    start: instruction.get_a() + 1,
                    count: vb,
                });
            }
            reads
        }
        OpCode::NewTable => vec![
            TraceIrOperand::UnsignedImmediate(instruction.get_vb()),
            TraceIrOperand::UnsignedImmediate(instruction.get_vc()),
        ],
        OpCode::Self_ => vec![
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::ConstantIndex(instruction.get_c()),
        ],
        OpCode::AddI => vec![
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::SignedImmediate(instruction.get_sc()),
        ],
        OpCode::AddK
        | OpCode::SubK
        | OpCode::MulK
        | OpCode::ModK
        | OpCode::PowK
        | OpCode::DivK
        | OpCode::IDivK
        | OpCode::BAndK
        | OpCode::BOrK
        | OpCode::BXorK => vec![
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::ConstantIndex(instruction.get_c()),
        ],
        OpCode::ShlI | OpCode::ShrI => vec![
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::SignedImmediate(instruction.get_sc()),
        ],
        OpCode::Add
        | OpCode::Sub
        | OpCode::Mul
        | OpCode::Mod
        | OpCode::Pow
        | OpCode::Div
        | OpCode::IDiv
        | OpCode::BAnd
        | OpCode::BOr
        | OpCode::BXor
        | OpCode::Shl
        | OpCode::Shr => vec![
            TraceIrOperand::Register(instruction.get_b()),
            rk_operand(instruction.get_c(), instruction.get_k()),
        ],
        OpCode::Unm | OpCode::BNot | OpCode::Not | OpCode::Len => {
            vec![TraceIrOperand::Register(instruction.get_b())]
        }
        OpCode::Concat => vec![TraceIrOperand::RegisterRange {
            start: instruction.get_a(),
            count: instruction.get_b(),
        }],
        OpCode::Eq | OpCode::Lt | OpCode::Le => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::Bool(instruction.get_k()),
        ],
        OpCode::MmBin => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::UnsignedImmediate(instruction.get_c()),
        ],
        OpCode::MmBinI => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::SignedImmediate(instruction.get_sb()),
            TraceIrOperand::Bool(instruction.get_k()),
            TraceIrOperand::UnsignedImmediate(instruction.get_c()),
        ],
        OpCode::MmBinK => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::ConstantIndex(instruction.get_b()),
            TraceIrOperand::Bool(instruction.get_k()),
            TraceIrOperand::UnsignedImmediate(instruction.get_c()),
        ],
        OpCode::EqK => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::ConstantIndex(instruction.get_b()),
            TraceIrOperand::Bool(instruction.get_k()),
        ],
        OpCode::EqI | OpCode::LtI | OpCode::LeI | OpCode::GtI | OpCode::GeI => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::SignedImmediate(instruction.get_sb()),
            TraceIrOperand::Bool(instruction.get_k()),
        ],
        OpCode::Test => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::Bool(instruction.get_k()),
        ],
        OpCode::TestSet => vec![
            TraceIrOperand::Register(instruction.get_b()),
            TraceIrOperand::Bool(instruction.get_k()),
        ],
        OpCode::Call => {
            let mut reads = vec![TraceIrOperand::Register(instruction.get_a())];
            let b = instruction.get_b();
            let c = instruction.get_c();
            reads.push(TraceIrOperand::UnsignedImmediate(b));
            reads.push(TraceIrOperand::UnsignedImmediate(c));
            if b > 1 {
                reads.push(TraceIrOperand::RegisterRange {
                    start: instruction.get_a() + 1,
                    count: b - 1,
                });
            }
            reads
        }
        OpCode::TForCall => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::Register(instruction.get_a() + 1),
            TraceIrOperand::Register(instruction.get_a() + 3),
            TraceIrOperand::UnsignedImmediate(2),
            TraceIrOperand::UnsignedImmediate(instruction.get_c()),
        ],
        OpCode::Return => {
            let result_count = instruction.get_b().saturating_sub(1);
            let mut reads = vec![
                TraceIrOperand::UnsignedImmediate(instruction.get_b()),
                TraceIrOperand::Bool(instruction.get_k()),
            ];
            if result_count != 0 {
                reads.push(TraceIrOperand::RegisterRange {
                    start: instruction.get_a(),
                    count: result_count,
                });
            }
            reads
        }
        OpCode::Return0 => Vec::new(),
        OpCode::Return1 => vec![TraceIrOperand::Register(instruction.get_a())],
        OpCode::Closure => vec![TraceIrOperand::UnsignedImmediate(instruction.get_bx())],
        OpCode::ForPrep => vec![TraceIrOperand::RegisterRange {
            start: instruction.get_a(),
            count: 3,
        }],
        OpCode::TForPrep => vec![
            TraceIrOperand::RegisterRange {
                start: instruction.get_a() + 2,
                count: 2,
            },
            jump_target(pc, instruction, opcode),
        ],
        OpCode::Jmp | OpCode::ForLoop => vec![jump_target(pc, instruction, opcode)],
        OpCode::TForLoop => vec![
            TraceIrOperand::Register(instruction.get_a() + 3),
            jump_target(pc, instruction, opcode),
        ],
        _ => Vec::new(),
    }
}

fn collect_writes(instruction: Instruction, opcode: OpCode) -> Vec<TraceIrOperand> {
    match opcode {
        OpCode::Move
        | OpCode::LoadI
        | OpCode::LoadF
        | OpCode::LoadK
        | OpCode::LoadKX
        | OpCode::LoadFalse
        | OpCode::LoadTrue
        | OpCode::GetUpval
        | OpCode::GetTabUp
        | OpCode::GetTable
        | OpCode::GetI
        | OpCode::GetField
        | OpCode::NewTable
        | OpCode::AddI
        | OpCode::AddK
        | OpCode::SubK
        | OpCode::MulK
        | OpCode::ModK
        | OpCode::PowK
        | OpCode::DivK
        | OpCode::IDivK
        | OpCode::BAndK
        | OpCode::BOrK
        | OpCode::BXorK
        | OpCode::ShlI
        | OpCode::ShrI
        | OpCode::Add
        | OpCode::Sub
        | OpCode::Mul
        | OpCode::Mod
        | OpCode::Pow
        | OpCode::Div
        | OpCode::IDiv
        | OpCode::BAnd
        | OpCode::BOr
        | OpCode::BXor
        | OpCode::Shl
        | OpCode::Shr
        | OpCode::Unm
        | OpCode::BNot
        | OpCode::Not
        | OpCode::Len
        | OpCode::Concat => vec![TraceIrOperand::Register(instruction.get_a())],
        OpCode::SetUpval => vec![TraceIrOperand::Upvalue(instruction.get_b())],
        OpCode::Close => Vec::new(),
        OpCode::LoadNil => vec![TraceIrOperand::RegisterRange {
            start: instruction.get_a(),
            count: instruction.get_b() + 1,
        }],
        OpCode::ForPrep => vec![TraceIrOperand::RegisterRange {
            start: instruction.get_a(),
            count: 3,
        }],
        OpCode::TForPrep => vec![TraceIrOperand::RegisterRange {
            start: instruction.get_a() + 2,
            count: 2,
        }],
        OpCode::SetTabUp | OpCode::SetTable | OpCode::SetI | OpCode::SetField | OpCode::SetList => {
            Vec::new()
        }
        OpCode::Self_ => vec![TraceIrOperand::RegisterRange {
            start: instruction.get_a(),
            count: 2,
        }],
        OpCode::TestSet => vec![TraceIrOperand::Register(instruction.get_a())],
        OpCode::Call => {
            let c = instruction.get_c();
            if c > 1 {
                vec![TraceIrOperand::RegisterRange {
                    start: instruction.get_a(),
                    count: c - 1,
                }]
            } else {
                Vec::new()
            }
        }
        OpCode::TForCall => {
            let c = instruction.get_c();
            if c > 0 {
                vec![TraceIrOperand::RegisterRange {
                    start: instruction.get_a() + 3,
                    count: c,
                }]
            } else {
                Vec::new()
            }
        }
        OpCode::Closure => vec![TraceIrOperand::Register(instruction.get_a())],
        OpCode::ForLoop => vec![
            TraceIrOperand::Register(instruction.get_a()),
            TraceIrOperand::Register(instruction.get_a() + 2),
        ],
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
        | OpCode::Jmp
        | OpCode::ExtraArg => Vec::new(),
        _ => Vec::new(),
    }
}

fn classify_opcode(opcode: OpCode) -> TraceIrInstKind {
    match opcode {
        OpCode::Move
        | OpCode::LoadI
        | OpCode::LoadF
        | OpCode::LoadK
        | OpCode::LoadKX
        | OpCode::LoadFalse
        | OpCode::LFalseSkip
        | OpCode::LoadTrue
        | OpCode::LoadNil
        | OpCode::ExtraArg => TraceIrInstKind::LoadMove,
        OpCode::GetUpval => TraceIrInstKind::UpvalueAccess,
        OpCode::SetUpval => TraceIrInstKind::UpvalueMutation,
        OpCode::Close | OpCode::Return | OpCode::Return0 | OpCode::Return1 => {
            TraceIrInstKind::Cleanup
        }
        OpCode::GetTabUp
        | OpCode::GetTable
        | OpCode::GetI
        | OpCode::GetField
        | OpCode::SetTabUp
        | OpCode::SetTable
        | OpCode::SetI
        | OpCode::SetField
        | OpCode::SetList
        | OpCode::NewTable
        | OpCode::Self_ => TraceIrInstKind::TableAccess,
        OpCode::AddI
        | OpCode::AddK
        | OpCode::SubK
        | OpCode::MulK
        | OpCode::ModK
        | OpCode::PowK
        | OpCode::DivK
        | OpCode::IDivK
        | OpCode::BAndK
        | OpCode::BOrK
        | OpCode::BXorK
        | OpCode::ShlI
        | OpCode::ShrI
        | OpCode::Add
        | OpCode::Sub
        | OpCode::Mul
        | OpCode::Mod
        | OpCode::Pow
        | OpCode::Div
        | OpCode::IDiv
        | OpCode::BAnd
        | OpCode::BOr
        | OpCode::BXor
        | OpCode::Shl
        | OpCode::Shr
        | OpCode::Unm
        | OpCode::BNot
        | OpCode::Not
        | OpCode::Len
        | OpCode::Concat => TraceIrInstKind::Arithmetic,
        OpCode::Call | OpCode::TForCall => TraceIrInstKind::Call,
        OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK => TraceIrInstKind::MetamethodFallback,
        OpCode::Closure => TraceIrInstKind::ClosureCreation,
        OpCode::ForPrep | OpCode::TForPrep => TraceIrInstKind::LoopPrep,
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
        | OpCode::TestSet => TraceIrInstKind::Guard,
        OpCode::Jmp => TraceIrInstKind::Branch,
        OpCode::ForLoop | OpCode::TForLoop => TraceIrInstKind::LoopBackedge,
        _ => TraceIrInstKind::Branch,
    }
}

#[cfg(test)]
mod tests {
    use crate::{Instruction, OpCode};

    use super::{
        TraceIr, TraceIrGuardKind, TraceIrInst, TraceIrInstKind, TraceIrOperand,
        is_fused_arithmetic_metamethod_fallback,
    };
    use crate::lua_vm::jit::trace_recorder::{
        TraceArtifact, TraceExit, TraceExitKind, TraceOp, TraceSeed,
    };

    #[test]
    fn lowering_marks_loop_tail_and_side_exit() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x10,
                instruction_budget: 5,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_abc(OpCode::Move, 0, 1, 0),
                    opcode: OpCode::Move,
                },
                TraceOp {
                    pc: 1,
                    instruction: Instruction::create_abck(OpCode::Test, 0, 0, 0, false),
                    opcode: OpCode::Test,
                },
                TraceOp {
                    pc: 2,
                    instruction: Instruction::create_sj(OpCode::Jmp, 1),
                    opcode: OpCode::Jmp,
                },
                TraceOp {
                    pc: 4,
                    instruction: Instruction::create_sj(OpCode::Jmp, -5),
                    opcode: OpCode::Jmp,
                },
            ],
            exits: vec![TraceExit {
                guard_pc: 1,
                branch_pc: 2,
                exit_pc: 4,
                taken_on_trace: false,
                kind: TraceExitKind::GuardExit,
            }],
            loop_tail_pc: 4,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.root_pc, 0);
        assert_eq!(ir.insts.len(), 4);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::LoadMove);
        assert_eq!(ir.insts[0].reads, vec![TraceIrOperand::Register(1)]);
        assert_eq!(ir.insts[0].writes, vec![TraceIrOperand::Register(0)]);
        assert_eq!(ir.insts[1].kind, TraceIrInstKind::Guard);
        assert_eq!(
            ir.insts[1].reads,
            vec![TraceIrOperand::Register(0), TraceIrOperand::Bool(false)]
        );
        assert_eq!(ir.insts[2].kind, TraceIrInstKind::Branch);
        assert_eq!(ir.insts[2].reads, vec![TraceIrOperand::JumpTarget(4)]);
        assert_eq!(ir.insts[3].kind, TraceIrInstKind::LoopBackedge);
        assert_eq!(ir.insts[3].reads, vec![TraceIrOperand::JumpTarget(0)]);
        assert_eq!(ir.guards.len(), 1);
        assert_eq!(ir.guards[0].kind, TraceIrGuardKind::SideExit);
    }

    #[test]
    fn lowering_marks_repeat_guard_as_loop_backedge_guard() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x20,
                instruction_budget: 3,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_abc(OpCode::Move, 0, 1, 0),
                    opcode: OpCode::Move,
                },
                TraceOp {
                    pc: 1,
                    instruction: Instruction::create_abck(OpCode::Test, 0, 0, 0, false),
                    opcode: OpCode::Test,
                },
                TraceOp {
                    pc: 2,
                    instruction: Instruction::create_sj(OpCode::Jmp, -3),
                    opcode: OpCode::Jmp,
                },
            ],
            exits: vec![TraceExit {
                guard_pc: 1,
                branch_pc: 2,
                exit_pc: 3,
                taken_on_trace: true,
                kind: TraceExitKind::GuardExit,
            }],
            loop_tail_pc: 2,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.guards.len(), 1);
        assert_eq!(ir.guards[0].kind, TraceIrGuardKind::LoopBackedgeGuard);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::LoadMove);
        assert_eq!(ir.insts[1].kind, TraceIrInstKind::Guard);
        assert_eq!(ir.insts[2].reads, vec![TraceIrOperand::JumpTarget(0)]);
        assert_eq!(ir.insts[2].kind, TraceIrInstKind::LoopBackedge);
    }

    #[test]
    fn lowering_classifies_table_and_arithmetic_ops() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x30,
                instruction_budget: 3,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_abc(OpCode::GetTable, 0, 1, 2),
                    opcode: OpCode::GetTable,
                },
                TraceOp {
                    pc: 1,
                    instruction: Instruction::create_abck(OpCode::Add, 0, 1, 2, false),
                    opcode: OpCode::Add,
                },
                TraceOp {
                    pc: 2,
                    instruction: Instruction::create_sj(OpCode::Jmp, -3),
                    opcode: OpCode::Jmp,
                },
            ],
            exits: vec![],
            loop_tail_pc: 2,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::TableAccess);
        assert_eq!(
            ir.insts[0].reads,
            vec![TraceIrOperand::Register(1), TraceIrOperand::Register(2)]
        );
        assert_eq!(ir.insts[0].writes, vec![TraceIrOperand::Register(0)]);
        assert_eq!(ir.insts[1].kind, TraceIrInstKind::Arithmetic);
        assert_eq!(
            ir.insts[1].reads,
            vec![TraceIrOperand::Register(1), TraceIrOperand::Register(2)]
        );
        assert_eq!(ir.insts[1].writes, vec![TraceIrOperand::Register(0)]);
        assert_eq!(ir.insts[2].kind, TraceIrInstKind::LoopBackedge);
    }

    #[test]
    fn detects_fused_arithmetic_metamethod_companions() {
        let insts = vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::AddK,
                raw_instruction: Instruction::create_abck(OpCode::AddK, 4, 4, 0, false).as_u32(),
                kind: TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 6, false).as_u32(),
                kind: TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![],
            },
        ];

        assert!(is_fused_arithmetic_metamethod_fallback(&insts, 1));
        assert!(!is_fused_arithmetic_metamethod_fallback(&insts, 0));
    }

    #[test]
    fn lowering_tracks_register_ranges_and_forloop_writes() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x40,
                instruction_budget: 2,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_abc(OpCode::LoadNil, 3, 2, 0),
                    opcode: OpCode::LoadNil,
                },
                TraceOp {
                    pc: 1,
                    instruction: Instruction::create_abx(OpCode::ForLoop, 5, 2),
                    opcode: OpCode::ForLoop,
                },
            ],
            exits: vec![],
            loop_tail_pc: 1,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(
            ir.insts[0].writes,
            vec![TraceIrOperand::RegisterRange { start: 3, count: 3 }]
        );
        assert_eq!(ir.insts[1].reads, vec![TraceIrOperand::JumpTarget(0)]);
        assert_eq!(
            ir.insts[1].writes,
            vec![TraceIrOperand::Register(5), TraceIrOperand::Register(7)]
        );
    }

    #[test]
    fn lowering_classifies_call_and_metamethod_fallback_ops() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x40,
                instruction_budget: 3,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_abc(OpCode::Call, 0, 2, 2),
                    opcode: OpCode::Call,
                },
                TraceOp {
                    pc: 1,
                    instruction: Instruction::create_abc(OpCode::MmBin, 1, 2, 0),
                    opcode: OpCode::MmBin,
                },
                TraceOp {
                    pc: 2,
                    instruction: Instruction::create_sj(OpCode::Jmp, -3),
                    opcode: OpCode::Jmp,
                },
            ],
            exits: Vec::new(),
            loop_tail_pc: 2,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::Call);
        assert_eq!(ir.insts[1].kind, TraceIrInstKind::MetamethodFallback);
    }

    #[test]
    fn lowering_classifies_upvalue_and_loop_prep_ops() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x50,
                instruction_budget: 3,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_abc(OpCode::GetUpval, 0, 2, 0),
                    opcode: OpCode::GetUpval,
                },
                TraceOp {
                    pc: 1,
                    instruction: Instruction::create_abx(OpCode::ForPrep, 0, 1),
                    opcode: OpCode::ForPrep,
                },
                TraceOp {
                    pc: 2,
                    instruction: Instruction::create_abx(OpCode::ForLoop, 0, 3),
                    opcode: OpCode::ForLoop,
                },
            ],
            exits: Vec::new(),
            loop_tail_pc: 2,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::UpvalueAccess);
        assert_eq!(ir.insts[0].reads, vec![TraceIrOperand::Upvalue(2)]);
        assert_eq!(ir.insts[0].writes, vec![TraceIrOperand::Register(0)]);
        assert_eq!(ir.insts[1].kind, TraceIrInstKind::LoopPrep);
        assert_eq!(
            ir.insts[1].reads,
            vec![TraceIrOperand::RegisterRange { start: 0, count: 3 }]
        );
        assert_eq!(
            ir.insts[1].writes,
            vec![TraceIrOperand::RegisterRange { start: 0, count: 3 }]
        );
    }

    #[test]
    fn lowering_tracks_tforprep_jump_target() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x60,
                instruction_budget: 2,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_abx(OpCode::TForPrep, 1, 2),
                    opcode: OpCode::TForPrep,
                },
                TraceOp {
                    pc: 3,
                    instruction: Instruction::create_sj(OpCode::Jmp, -4),
                    opcode: OpCode::Jmp,
                },
            ],
            exits: Vec::new(),
            loop_tail_pc: 3,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::LoopPrep);
        assert_eq!(
            ir.insts[0].reads,
            vec![
                TraceIrOperand::RegisterRange { start: 3, count: 2 },
                TraceIrOperand::JumpTarget(3),
            ]
        );
        assert_eq!(
            ir.insts[0].writes,
            vec![TraceIrOperand::RegisterRange { start: 3, count: 2 }]
        );
    }

    #[test]
    fn lowering_classifies_tforcall_setupval_and_closure() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x70,
                instruction_budget: 4,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_abc(OpCode::TForCall, 2, 0, 2),
                    opcode: OpCode::TForCall,
                },
                TraceOp {
                    pc: 1,
                    instruction: Instruction::create_abck(OpCode::SetUpval, 1, 3, 0, false),
                    opcode: OpCode::SetUpval,
                },
                TraceOp {
                    pc: 2,
                    instruction: Instruction::create_abx(OpCode::Closure, 4, 6),
                    opcode: OpCode::Closure,
                },
                TraceOp {
                    pc: 3,
                    instruction: Instruction::create_sj(OpCode::Jmp, -4),
                    opcode: OpCode::Jmp,
                },
            ],
            exits: Vec::new(),
            loop_tail_pc: 3,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::Call);
        assert_eq!(
            ir.insts[0].writes,
            vec![TraceIrOperand::RegisterRange { start: 5, count: 2 }]
        );
        assert_eq!(ir.insts[1].kind, TraceIrInstKind::UpvalueMutation);
        assert_eq!(ir.insts[1].writes, vec![TraceIrOperand::Upvalue(3)]);
        assert_eq!(ir.insts[2].kind, TraceIrInstKind::ClosureCreation);
        assert_eq!(ir.insts[2].reads, vec![TraceIrOperand::UnsignedImmediate(6)]);
        assert_eq!(ir.insts[2].writes, vec![TraceIrOperand::Register(4)]);
    }

    #[test]
    fn lowering_classifies_tforloop_setlist_and_close() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 0,
                root_chunk_addr: 0x80,
                instruction_budget: 4,
            },
            ops: vec![
                TraceOp {
                    pc: 0,
                    instruction: Instruction::create_vabck(OpCode::SetList, 1, 2, 3, false),
                    opcode: OpCode::SetList,
                },
                TraceOp {
                    pc: 1,
                    instruction: Instruction::create_abc(OpCode::Close, 2, 0, 0),
                    opcode: OpCode::Close,
                },
                TraceOp {
                    pc: 2,
                    instruction: Instruction::create_abc(OpCode::TForCall, 0, 0, 2),
                    opcode: OpCode::TForCall,
                },
                TraceOp {
                    pc: 3,
                    instruction: Instruction::create_abx(OpCode::TForLoop, 0, 4),
                    opcode: OpCode::TForLoop,
                },
            ],
            exits: vec![TraceExit {
                guard_pc: 3,
                branch_pc: 3,
                exit_pc: 4,
                taken_on_trace: true,
                kind: TraceExitKind::GuardExit,
            }],
            loop_tail_pc: 3,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::TableAccess);
        assert_eq!(ir.insts[1].kind, TraceIrInstKind::Cleanup);
        assert_eq!(ir.insts[1].reads, vec![TraceIrOperand::Register(2)]);
        assert_eq!(ir.insts[2].kind, TraceIrInstKind::Call);
        assert_eq!(ir.insts[3].kind, TraceIrInstKind::LoopBackedge);
        assert_eq!(
            ir.insts[3].reads,
            vec![TraceIrOperand::Register(3), TraceIrOperand::JumpTarget(0)]
        );
        assert_eq!(ir.guards.len(), 1);
        assert_eq!(ir.guards[0].kind, TraceIrGuardKind::LoopBackedgeGuard);
    }

    #[test]
    fn lowering_keeps_terminal_return_as_cleanup() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 4,
                root_chunk_addr: 0x30,
                instruction_budget: 1,
            },
            ops: vec![TraceOp {
                pc: 4,
                instruction: Instruction::create_abc(OpCode::Return1, 2, 0, 0),
                opcode: OpCode::Return1,
            }],
            exits: vec![],
            loop_tail_pc: 4,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.insts.len(), 1);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::Cleanup);
        assert_eq!(ir.insts[0].reads, vec![TraceIrOperand::Register(2)]);
    }

    #[test]
    fn lowering_keeps_fixed_arity_return_as_cleanup() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 8,
                root_chunk_addr: 0x31,
                instruction_budget: 1,
            },
            ops: vec![TraceOp {
                pc: 8,
                instruction: Instruction::create_abck(OpCode::Return, 4, 3, 0, false),
                opcode: OpCode::Return,
            }],
            exits: vec![],
            loop_tail_pc: 8,
        };

        let ir = TraceIr::lower(&artifact);
        assert_eq!(ir.insts.len(), 1);
        assert_eq!(ir.insts[0].kind, TraceIrInstKind::Cleanup);
        assert_eq!(
            ir.insts[0].reads,
            vec![
                TraceIrOperand::UnsignedImmediate(3),
                TraceIrOperand::Bool(false),
                TraceIrOperand::RegisterRange { start: 4, count: 2 },
            ]
        );
    }
}