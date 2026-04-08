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
fn backend_compiles_numeric_forloop_with_table_copy_steps() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 40,
        loop_tail_pc: 42,
        insts: vec![
            TraceIrInst {
                pc: 40,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 8, 2, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 41,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 3, 7, 8, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::Register(7), TraceIrOperand::Register(8)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 42,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(40)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 40,
        loop_tail_pc: 42,
        steps: vec![
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::Register(7), TraceIrOperand::Register(8)],
                writes: vec![],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(40)],
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
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![
                        NumericStep::GetTableInt {
                            dst: 8,
                            table: 2,
                            index: 7,
                        },
                        NumericStep::SetTableInt {
                            table: 3,
                            index: 7,
                            value: 8,
                        },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_numeric_forloop_with_upvalue_steps() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 4,
        loop_tail_pc: 8,
        insts: vec![
            TraceIrInst {
                pc: 4,
                opcode: OpCode::GetUpval,
                raw_instruction: Instruction::create_abc(OpCode::GetUpval, 4, 0, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::UpvalueAccess,
                reads: vec![TraceIrOperand::Upvalue(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 5,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 4, 4, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 6,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 7,
                opcode: OpCode::SetUpval,
                raw_instruction: Instruction::create_abck(OpCode::SetUpval, 4, 0, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::UpvalueMutation,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Upvalue(0)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 8,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 1, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(4)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 4,
        loop_tail_pc: 8,
        steps: vec![
            HelperPlanStep::UpvalueAccess {
                reads: vec![TraceIrOperand::Upvalue(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
            },
            HelperPlanStep::UpvalueMutation {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Upvalue(0)],
                writes: vec![],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(4)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
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
                    loop_reg: 1,
                    steps: vec![
                        NumericStep::GetUpval { dst: 4, upvalue: 0 },
                        NumericStep::Binary {
                            dst: 4,
                            lhs: NumericOperand::Reg(4),
                            rhs: NumericOperand::ImmI(1),
                            op: NumericBinaryOp::Add,
                        },
                        NumericStep::SetUpval { src: 4, upvalue: 0 },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_forwards_table_int_load_from_prior_store_in_numeric_forloop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 40,
        loop_tail_pc: 43,
        insts: vec![
            TraceIrInst {
                pc: 40,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 2, 7, 8, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(8),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 41,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 9, 2, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 42,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 10, 9, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(9)],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 43,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(40)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 40,
        loop_tail_pc: 43,
        steps: vec![
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(8),
                ],
                writes: vec![],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(9)],
                writes: vec![TraceIrOperand::Register(10)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(40)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 4,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![
                        NumericStep::SetTableInt {
                            table: 2,
                            index: 7,
                            value: 8,
                        },
                        NumericStep::Move { dst: 9, src: 8 },
                        NumericStep::Move { dst: 10, src: 9 },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_eliminates_dead_redundant_table_int_store_in_numeric_forloop() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 60,
        loop_tail_pc: 62,
        insts: vec![
            TraceIrInst {
                pc: 60,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 2, 7, 8, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(8),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 61,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 2, 7, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(9),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 62,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(60)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 60,
        loop_tail_pc: 62,
        steps: vec![
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(8),
                ],
                writes: vec![],
            },
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(60)],
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
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![NumericStep::SetTableInt {
                        table: 2,
                        index: 7,
                        value: 9,
                    }],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_forwards_table_int_load_across_moved_table_and_index_regs() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 70,
        loop_tail_pc: 74,
        insts: vec![
            TraceIrInst {
                pc: 70,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 6, 2, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(2)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            TraceIrInst {
                pc: 71,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 10, 7, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 72,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 6, 10, 8, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::Register(10),
                    TraceIrOperand::Register(8),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 73,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 9, 2, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 74,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(70)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 70,
        loop_tail_pc: 74,
        steps: vec![
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(2)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(10)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::Register(10),
                    TraceIrOperand::Register(8),
                ],
                writes: vec![],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(70)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![
                        NumericStep::Move { dst: 6, src: 2 },
                        NumericStep::Move { dst: 10, src: 7 },
                        NumericStep::SetTableInt {
                            table: 6,
                            index: 10,
                            value: 8,
                        },
                        NumericStep::Move { dst: 9, src: 8 },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_forwards_table_int_load_across_index_offset_aliases() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 80,
        loop_tail_pc: 84,
        insts: vec![
            TraceIrInst {
                pc: 80,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 8, 7, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 81,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 82,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 2, 8, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(8),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 83,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 10, 7, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 84,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 85,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 11, 2, 10, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(10)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            TraceIrInst {
                pc: 86,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(80)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 80,
        loop_tail_pc: 86,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(8),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(10)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(10)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(80)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 7,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 2,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![
                        NumericStep::Binary {
                            dst: 8,
                            lhs: NumericOperand::Reg(7),
                            rhs: NumericOperand::ImmI(1),
                            op: NumericBinaryOp::Add,
                        },
                        NumericStep::SetTableInt {
                            table: 2,
                            index: 8,
                            value: 9,
                        },
                        NumericStep::Binary {
                            dst: 10,
                            lhs: NumericOperand::Reg(7),
                            rhs: NumericOperand::ImmI(1),
                            op: NumericBinaryOp::Add,
                        },
                        NumericStep::Move { dst: 11, src: 9 },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_forwards_table_int_load_after_inplace_index_rebase() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 90,
        loop_tail_pc: 96,
        insts: vec![
            TraceIrInst {
                pc: 90,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 8, 7, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 91,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 92,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 2, 8, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(8),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 93,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 7, 7, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 94,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 95,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 11, 2, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            TraceIrInst {
                pc: 96,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(90)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 90,
        loop_tail_pc: 96,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(8),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(90)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 7,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 2,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![
                        NumericStep::Binary {
                            dst: 8,
                            lhs: NumericOperand::Reg(7),
                            rhs: NumericOperand::ImmI(1),
                            op: NumericBinaryOp::Add,
                        },
                        NumericStep::SetTableInt {
                            table: 2,
                            index: 8,
                            value: 9,
                        },
                        NumericStep::Binary {
                            dst: 7,
                            lhs: NumericOperand::Reg(7),
                            rhs: NumericOperand::ImmI(1),
                            op: NumericBinaryOp::Add,
                        },
                        NumericStep::Move { dst: 11, src: 9 },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_forwards_table_int_load_after_inplace_index_decrement_rebase() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 100,
        loop_tail_pc: 106,
        insts: vec![
            TraceIrInst {
                pc: 100,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 2, 7, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 101,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 7, 7, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 102,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 103,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 7, 7, 126).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(-1)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 104,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 105,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 11, 2, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            TraceIrInst {
                pc: 106,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(100)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 100,
        loop_tail_pc: 106,
        steps: vec![
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(-1)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::SignedImmediate(1)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(100)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 7,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 2,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![
                        NumericStep::SetTableInt {
                            table: 2,
                            index: 7,
                            value: 9,
                        },
                        NumericStep::Binary {
                            dst: 7,
                            lhs: NumericOperand::Reg(7),
                            rhs: NumericOperand::ImmI(1),
                            op: NumericBinaryOp::Add,
                        },
                        NumericStep::Binary {
                            dst: 7,
                            lhs: NumericOperand::Reg(7),
                            rhs: NumericOperand::ImmI(-1),
                            op: NumericBinaryOp::Add,
                        },
                        NumericStep::Move { dst: 11, src: 9 },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_forwards_table_int_load_after_table_root_register_clobber() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 110,
        loop_tail_pc: 114,
        insts: vec![
            TraceIrInst {
                pc: 110,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 6, 2, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(2)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            TraceIrInst {
                pc: 111,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 6, 7, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 112,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 2, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(2)],
            },
            TraceIrInst {
                pc: 113,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 11, 6, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(6), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            TraceIrInst {
                pc: 114,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(110)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 110,
        loop_tail_pc: 114,
        steps: vec![
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(2)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(2)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(6), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(110)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![
                        NumericStep::Move { dst: 6, src: 2 },
                        NumericStep::SetTableInt {
                            table: 6,
                            index: 7,
                            value: 9,
                        },
                        NumericStep::Move { dst: 2, src: 4 },
                        NumericStep::Move { dst: 11, src: 9 },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_forwards_table_int_load_after_index_root_register_clobber() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 120,
        loop_tail_pc: 124,
        insts: vec![
            TraceIrInst {
                pc: 120,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 10, 7, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 121,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 2, 10, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(10),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 122,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 7, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 123,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 11, 2, 10, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(10)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            TraceIrInst {
                pc: 124,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(120)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 120,
        loop_tail_pc: 124,
        steps: vec![
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(10)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(2),
                    TraceIrOperand::Register(10),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(10)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(120)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForLoop {
                    loop_reg: 5,
                    steps: vec![
                        NumericStep::Move { dst: 10, src: 7 },
                        NumericStep::SetTableInt {
                            table: 2,
                            index: 10,
                            value: 9,
                        },
                        NumericStep::Move { dst: 7, src: 4 },
                        NumericStep::Move { dst: 11, src: 9 },
                    ],
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_guarded_numeric_forloop_with_tail_compare() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 4,
        loop_tail_pc: 12,
        insts: vec![
            TraceIrInst {
                pc: 4,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 7, 7, 126).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 5,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 7, 128, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 6,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 8, 5, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(6)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 7,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 9, 5, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 8,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 8, 9, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 9,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(10)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 12,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 1, 9).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(4)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 8,
            branch_pc: 9,
            exit_pc: 10,
            taken_on_trace: true,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 4,
        loop_tail_pc: 12,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(6)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(10)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(4)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 7,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 1,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::GuardedNumericForLoop {
                    loop_reg: 1,
                    steps: vec![
                        NumericStep::Binary {
                            dst: 7,
                            lhs: NumericOperand::Reg(7),
                            rhs: NumericOperand::ImmI(-1),
                            op: NumericBinaryOp::Add,
                        },
                        NumericStep::GetTableInt {
                            dst: 8,
                            table: 5,
                            index: 6,
                        },
                        NumericStep::GetTableInt {
                            dst: 9,
                            table: 5,
                            index: 7,
                        },
                    ],
                    guard: NumericJmpLoopGuard::Tail {
                        cond: NumericIfElseCond::RegCompare {
                            op: LinearIntGuardOp::Lt,
                            lhs: 8,
                            rhs: 9,
                        },
                        continue_when: false,
                        continue_preset: None,
                        exit_preset: None,
                        exit_pc: 10,
                    },
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
