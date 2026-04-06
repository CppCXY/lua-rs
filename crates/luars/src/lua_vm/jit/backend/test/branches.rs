use super::*;
use crate::lua_vm::jit::trace_recorder::{TraceArtifact, TraceExit, TraceExitKind, TraceOp, TraceSeed};

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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 19,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 9, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 20,
                opcode: OpCode::EqI,
                raw_instruction: Instruction::create_abck(OpCode::EqI, 7, 127, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![
                    TraceIrOperand::Register(7),
                    TraceIrOperand::SignedImmediate(0),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 23,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(-1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 26,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 7, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                    cond: NumericIfElseCond::IntCompare {
                        op: LinearIntGuardOp::Eq,
                        reg: 7,
                        imm: 0,
                    },
                    then_preset: None,
                    else_preset: None,
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
                    then_on_true: true,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

fn backend_compiles_guarded_numeric_ifelse_forloop_from_artifact() {
    let mut backend = NullTraceBackend;
    let mut chunk = crate::lua_value::LuaProto::new();
    chunk
        .code
        .push(Instruction::create_abc(OpCode::ModK, 7, 4, 0));
    chunk
        .code
        .push(Instruction::create_abck(OpCode::MmBinK, 4, 0, 9, false));
    chunk
        .code
        .push(Instruction::create_abck(OpCode::EqI, 7, 127, 0, false));
    chunk.code.push(Instruction::create_sj(OpCode::Jmp, 3));
    chunk
        .code
        .push(Instruction::create_abc(OpCode::AddI, 5, 5, 128));
    chunk
        .code
        .push(Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false));
    chunk.code.push(Instruction::create_sj(OpCode::Jmp, 2));
    chunk
        .code
        .push(Instruction::create_abc(OpCode::AddI, 5, 5, 126));
    chunk
        .code
        .push(Instruction::create_abck(OpCode::MmBinI, 5, 128, 7, false));
    chunk
        .code
        .push(Instruction::create_abx(OpCode::ForLoop, 1, 9));

    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 9,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::ModK,
                raw_instruction: chunk.code[0].as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::MmBinK,
                raw_instruction: chunk.code[1].as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::EqI,
                raw_instruction: chunk.code[2].as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![
                    TraceIrOperand::Register(7),
                    TraceIrOperand::SignedImmediate(0),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 5,
                opcode: OpCode::MmBinI,
                raw_instruction: chunk.code[5].as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
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
    let artifact = TraceArtifact {
        seed: TraceSeed {
            start_pc: 0,
            root_chunk_addr: &chunk as *const _ as usize,
            instruction_budget: 10,
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
        exits: vec![TraceExit {
            guard_pc: 2,
            branch_pc: 3,
            exit_pc: 7,
            taken_on_trace: false,
            kind: TraceExitKind::GuardExit,
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

    match <NullTraceBackend as TraceBackend>::compile(
        &mut backend,
        &artifact,
        &ir,
        &helper_plan,
    ) {
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
                    cond: NumericIfElseCond::IntCompare {
                        op: LinearIntGuardOp::Eq,
                        reg: 7,
                        imm: 0,
                    },
                    then_preset: None,
                    else_preset: None,
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
                    then_on_true: true,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

fn backend_compiles_numeric_lti_ifelse_forloop() {
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 19,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 9, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 20,
                opcode: OpCode::LtI,
                raw_instruction: Instruction::create_abck(OpCode::LtI, 7, 127 + 3, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![
                    TraceIrOperand::Register(7),
                    TraceIrOperand::SignedImmediate(3),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 23,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(-1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 26,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 7, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                    cond: NumericIfElseCond::IntCompare {
                        op: LinearIntGuardOp::Lt,
                        reg: 7,
                        imm: 3,
                    },
                    then_preset: None,
                    else_preset: None,
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
                    then_on_true: true,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

fn backend_compiles_numeric_test_ifelse_forloop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 10,
        loop_tail_pc: 17,
        insts: vec![
            TraceIrInst {
                pc: 11,
                opcode: OpCode::Test,
                raw_instruction: Instruction::create_abck(OpCode::Test, 7, 0, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Bool(false)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 12,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(16)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 13,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 14,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 15,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(18)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 16,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 126).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(-1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 17,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 7, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 18,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 1, 8).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(11)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 10,
        loop_tail_pc: 17,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 8,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 2,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericIfElseForLoop {
                    loop_reg: 1,
                    pre_steps: vec![],
                    cond: NumericIfElseCond::Truthy { reg: 7 },
                    then_preset: None,
                    else_preset: None,
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
                    then_on_true: true,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_numeric_testset_ifelse_forloop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 10,
        loop_tail_pc: 17,
        insts: vec![
            TraceIrInst {
                pc: 11,
                opcode: OpCode::TestSet,
                raw_instruction: Instruction::create_abck(OpCode::TestSet, 8, 7, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Bool(false)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 12,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(16)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 13,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 14,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 15,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(18)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 16,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 8, 126).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(8),
                    TraceIrOperand::SignedImmediate(-1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 17,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 8, 128, 7, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(8),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 18,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 1, 8).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(11)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 10,
        loop_tail_pc: 17,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 8,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 2,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericIfElseForLoop {
                    loop_reg: 1,
                    pre_steps: vec![],
                    cond: NumericIfElseCond::Truthy { reg: 7 },
                    then_preset: None,
                    else_preset: Some(NumericStep::Move { dst: 8, src: 7 }),
                    then_steps: vec![NumericStep::Binary {
                        dst: 5,
                        lhs: NumericOperand::Reg(5),
                        rhs: NumericOperand::ImmI(1),
                        op: NumericBinaryOp::Add,
                    }],
                    else_steps: vec![NumericStep::Binary {
                        dst: 5,
                        lhs: NumericOperand::Reg(8),
                        rhs: NumericOperand::ImmI(-1),
                        op: NumericBinaryOp::Add,
                    }],
                    then_on_true: true,
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 54,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 81,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(10),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 52,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 53,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 4, 4, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 79,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 4, 4, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 80,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 81,
                opcode: OpCode::GeI,
                raw_instruction: Instruction::create_abck(OpCode::GeI, 4, 137, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(10),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
fn backend_compiles_head_guard_numeric_test_jmp_loop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 4,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::Test,
                raw_instruction: Instruction::create_abck(OpCode::Test, 7, 0, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Bool(false)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 4,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 0,
            branch_pc: 1,
            exit_pc: 5,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 4,
        steps: vec![],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 1,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericJmpLoop {
                    pre_steps: vec![],
                    steps: vec![NumericStep::Binary {
                        dst: 5,
                        lhs: NumericOperand::Reg(5),
                        rhs: NumericOperand::ImmI(1),
                        op: NumericBinaryOp::Add,
                    }],
                    guard: NumericJmpLoopGuard::Head {
                        cond: NumericIfElseCond::Truthy { reg: 7 },
                        continue_when: true,
                        continue_preset: None,
                        exit_preset: None,
                        exit_pc: 5,
                    },
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_head_guard_numeric_testset_jmp_loop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 4,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::TestSet,
                raw_instruction: Instruction::create_abck(OpCode::TestSet, 8, 7, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Bool(false)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 4,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 0,
            branch_pc: 1,
            exit_pc: 5,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 4,
        steps: vec![],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 1,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericJmpLoop {
                    pre_steps: vec![],
                    steps: vec![NumericStep::Binary {
                        dst: 5,
                        lhs: NumericOperand::Reg(5),
                        rhs: NumericOperand::ImmI(1),
                        op: NumericBinaryOp::Add,
                    }],
                    guard: NumericJmpLoopGuard::Head {
                        cond: NumericIfElseCond::Truthy { reg: 7 },
                        continue_when: true,
                        continue_preset: None,
                        exit_preset: Some(NumericStep::Move { dst: 8, src: 7 }),
                        exit_pc: 5,
                    },
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_head_guard_numeric_table_compare_jmp_loop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 5,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abc(OpCode::GetTable, 9, 1, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(1), TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 9, 4, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(9), TraceIrOperand::Register(4)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(6)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 4,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::SignedImmediate(1)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 5,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 1,
            branch_pc: 2,
            exit_pc: 6,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 5,
        steps: vec![],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 6,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 1,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericTableScanJmpLoop {
                    table_reg: 1,
                    index_reg: 5,
                    limit_reg: 4,
                    step_imm: 1,
                    compare_op: LinearIntGuardOp::Lt,
                    exit_pc: 6,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_numeric_table_shift_jmp_loop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 11,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::Le,
                raw_instruction: Instruction::create_abck(OpCode::Le, 5, 7, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(7)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 9).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(12)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abc(OpCode::GetTable, 8, 0, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 6, 8, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::Register(8),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 4,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(12)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 5,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 8, 7, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 6,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 7,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abc(OpCode::GetTable, 9, 0, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 8,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 0, 8, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(0),
                    TraceIrOperand::Register(8),
                    TraceIrOperand::Register(9),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 9,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 7, 7, 126).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(-1)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 10,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 7, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 11,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -12).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: vec![
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 0,
                branch_pc: 1,
                exit_pc: 12,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            },
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 3,
                branch_pc: 4,
                exit_pc: 12,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            },
        ],
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 11,
        steps: vec![],
        guard_count: 2,
        summary: HelperPlanDispatchSummary {
            steps_executed: 12,
            guards_observed: 2,
            call_steps: 0,
            metamethod_steps: 2,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericTableShiftJmpLoop {
                    table_reg: 0,
                    index_reg: 7,
                    left_bound_reg: 5,
                    value_reg: 6,
                    temp_reg: 8,
                    exit_pc: 12,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_tail_guard_numeric_test_jmp_loop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 2,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 5, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 5, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::Test,
                raw_instruction: Instruction::create_abck(OpCode::Test, 7, 0, 0, true).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Bool(true)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 2,
            branch_pc: 3,
            exit_pc: 4,
            taken_on_trace: true,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 3,
        steps: vec![],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 4,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 1,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericJmpLoop {
                    pre_steps: vec![],
                    steps: vec![NumericStep::Binary {
                        dst: 5,
                        lhs: NumericOperand::Reg(5),
                        rhs: NumericOperand::ImmI(1),
                        op: NumericBinaryOp::Add,
                    }],
                    guard: NumericJmpLoopGuard::Tail {
                        cond: NumericIfElseCond::Truthy { reg: 7 },
                        continue_when: true,
                        continue_preset: None,
                        exit_preset: None,
                        exit_pc: 4,
                    },
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_tail_guard_numeric_testset_jmp_loop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 2,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 5, 8, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(8),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 8, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(8),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::TestSet,
                raw_instruction: Instruction::create_abck(OpCode::TestSet, 8, 7, 0, true)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Bool(true)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 2,
            branch_pc: 3,
            exit_pc: 4,
            taken_on_trace: true,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 3,
        steps: vec![],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 4,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 1,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericJmpLoop {
                    pre_steps: vec![],
                    steps: vec![NumericStep::Binary {
                        dst: 5,
                        lhs: NumericOperand::Reg(8),
                        rhs: NumericOperand::ImmI(1),
                        op: NumericBinaryOp::Add,
                    }],
                    guard: NumericJmpLoopGuard::Tail {
                        cond: NumericIfElseCond::Truthy { reg: 7 },
                        continue_when: true,
                        continue_preset: Some(NumericStep::Move { dst: 8, src: 7 }),
                        exit_preset: None,
                        exit_pc: 4,
                    },
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}
