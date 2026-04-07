use super::*;

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
                raw_instruction: Instruction::create_abck(OpCode::Add, 6, 6, 12, false)
                    .as_u32(),
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

    match backend.compile_test(&ir, &helper_plan) {
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
                raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 12, false)
                    .as_u32(),
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

    match backend.compile_test(&ir, &helper_plan) {
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(10),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 48,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 10, 8, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(10),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(10),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(10),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::SignedImmediate(5),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 78,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 10, 132, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::SignedImmediate(5),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 79,
                opcode: OpCode::MulK,
                raw_instruction: Instruction::create_abc(OpCode::MulK, 6, 5, 12).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::ConstantIndex(12),
                ],
                writes: vec![TraceIrOperand::Register(6)],
            },
            TraceIrInst {
                pc: 80,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 5, 12, 8, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::ConstantIndex(12),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 81,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 7, 6, 124).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::SignedImmediate(-3),
                ],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 82,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 6, 130, 7, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::SignedImmediate(-3),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::SignedImmediate(5),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::SignedImmediate(5),
                ],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::ConstantIndex(12),
                ],
                writes: vec![TraceIrOperand::Register(6)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::ConstantIndex(12),
                ],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::SignedImmediate(-3),
                ],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::SignedImmediate(-3),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                reads: vec![
                    TraceIrOperand::Register(17),
                    TraceIrOperand::ConstantIndex(23),
                ],
                writes: vec![TraceIrOperand::Register(18)],
            },
            TraceIrInst {
                pc: 71,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 17, 23, 8, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(17),
                    TraceIrOperand::ConstantIndex(23),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 72,
                opcode: OpCode::AddK,
                raw_instruction: Instruction::create_abc(OpCode::AddK, 18, 18, 24).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(18),
                    TraceIrOperand::ConstantIndex(24),
                ],
                writes: vec![TraceIrOperand::Register(18)],
            },
            TraceIrInst {
                pc: 73,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 18, 24, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(18),
                    TraceIrOperand::ConstantIndex(24),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 74,
                opcode: OpCode::DivK,
                raw_instruction: Instruction::create_abc(OpCode::DivK, 14, 18, 25).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(18),
                    TraceIrOperand::ConstantIndex(25),
                ],
                writes: vec![TraceIrOperand::Register(14)],
            },
            TraceIrInst {
                pc: 75,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 18, 25, 11, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(18),
                    TraceIrOperand::ConstantIndex(25),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                reads: vec![
                    TraceIrOperand::Register(17),
                    TraceIrOperand::ConstantIndex(43),
                ],
                writes: vec![TraceIrOperand::Register(18)],
            },
            TraceIrInst {
                pc: 353,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 17, 43, 9, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(17),
                    TraceIrOperand::ConstantIndex(43),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 354,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 18, 18, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(18),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(18)],
            },
            TraceIrInst {
                pc: 355,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 18, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(18),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 356,
                opcode: OpCode::PowK,
                raw_instruction: Instruction::create_abc(OpCode::PowK, 14, 18, 44).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(18),
                    TraceIrOperand::ConstantIndex(44),
                ],
                writes: vec![TraceIrOperand::Register(14)],
            },
            TraceIrInst {
                pc: 357,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 18, 44, 10, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(18),
                    TraceIrOperand::ConstantIndex(44),
                ],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                reads: vec![
                    TraceIrOperand::Register(17),
                    TraceIrOperand::ConstantIndex(39),
                ],
                writes: vec![TraceIrOperand::Register(18)],
            },
            TraceIrInst {
                pc: 295,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 17, 39, 13, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(17),
                    TraceIrOperand::ConstantIndex(39),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 296,
                opcode: OpCode::ShrI,
                raw_instruction: Instruction::create_abc(OpCode::ShrI, 19, 17, 131).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(17),
                    TraceIrOperand::SignedImmediate(4),
                ],
                writes: vec![TraceIrOperand::Register(19)],
            },
            TraceIrInst {
                pc: 297,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 17, 131, 17, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(17),
                    TraceIrOperand::SignedImmediate(4),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 298,
                opcode: OpCode::BOr,
                raw_instruction: Instruction::create_abck(OpCode::BOr, 12, 18, 19, false)
                    .as_u32(),
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

    match backend.compile_test(&ir, &helper_plan) {
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
