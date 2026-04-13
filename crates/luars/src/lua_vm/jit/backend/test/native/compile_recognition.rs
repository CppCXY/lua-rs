use super::*;

#[test]
fn native_backend_compiles_linear_int_forloop_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
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

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_linked_side_trace_shape_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
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
    let artifact = crate::lua_vm::jit::trace_recorder::TraceArtifact {
        seed: crate::lua_vm::jit::trace_recorder::TraceSeed {
            start_pc: 20,
            root_chunk_addr: 0,
            instruction_budget: 3,
        },
        ops: ir
            .insts
            .iter()
            .map(|inst| crate::lua_vm::jit::trace_recorder::TraceOp {
                pc: inst.pc,
                instruction: Instruction::from_u32(inst.raw_instruction),
                opcode: inst.opcode,
            })
            .collect(),
        exits: Vec::new(),
        loop_header_pc: 10,
        loop_tail_pc: 22,
    };

    match backend.compile_test_with_artifact(&artifact, &ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_return0_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 7,
        loop_tail_pc: 7,
        insts: vec![TraceIrInst {
            pc: 7,
            opcode: OpCode::Return0,
            raw_instruction: Instruction::create_abc(OpCode::Return0, 0, 0, 0).as_u32(),
            kind: crate::lua_vm::jit::ir::TraceIrInstKind::Cleanup,
            reads: vec![],
            writes: vec![],
        }],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 7,
        loop_tail_pc: 7,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 1,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::Return0 { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_return1_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 9,
        loop_tail_pc: 9,
        insts: vec![TraceIrInst {
            pc: 9,
            opcode: OpCode::Return1,
            raw_instruction: Instruction::create_abc(OpCode::Return1, 4, 0, 0).as_u32(),
            kind: crate::lua_vm::jit::ir::TraceIrInstKind::Cleanup,
            reads: vec![TraceIrOperand::Register(4)],
            writes: vec![],
        }],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 9,
        loop_tail_pc: 9,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 1,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::Return1 { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_fixed_arity_return_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 11,
        loop_tail_pc: 11,
        insts: vec![TraceIrInst {
            pc: 11,
            opcode: OpCode::Return,
            raw_instruction: Instruction::create_abck(OpCode::Return, 5, 4, 0, false).as_u32(),
            kind: crate::lua_vm::jit::ir::TraceIrInstKind::Cleanup,
            reads: vec![
                TraceIrOperand::Register(5),
                TraceIrOperand::Register(6),
                TraceIrOperand::Register(7),
            ],
            writes: vec![],
        }],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 11,
        loop_tail_pc: 11,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 1,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::Return { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_linear_int_jmp_loop_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
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
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntJmpLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_numeric_for_loop_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 90,
        loop_tail_pc: 91,
        insts: vec![
            TraceIrInst {
                pc: 90,
                opcode: OpCode::AddK,
                raw_instruction: Instruction::create_abc(OpCode::AddK, 4, 4, 10).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(10),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 91,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(90)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 90,
        loop_tail_pc: 91,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(10),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(90)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 2,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_rejects_empty_numeric_for_loop_body() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 90,
        loop_tail_pc: 90,
        insts: vec![TraceIrInst {
            pc: 90,
            opcode: OpCode::ForLoop,
            raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 1).as_u32(),
            kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
            reads: vec![TraceIrOperand::JumpTarget(90)],
            writes: Vec::new(),
        }],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 90,
        loop_tail_pc: 90,
        steps: vec![HelperPlanStep::LoopBackedge {
            reads: vec![TraceIrOperand::JumpTarget(90)],
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

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(!matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { .. })
                    | CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => {}
    }
}

#[test]
fn native_backend_compiles_guarded_numeric_for_loop_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 100,
        loop_tail_pc: 103,
        insts: vec![
            TraceIrInst {
                pc: 100,
                opcode: OpCode::AddK,
                raw_instruction: Instruction::create_abc(OpCode::AddK, 4, 4, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(3),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 101,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 6, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 102,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(105)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 103,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(100)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 101,
            branch_pc: 102,
            exit_pc: 105,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 100,
        loop_tail_pc: 103,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(3),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(105)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(100)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 4,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::GuardedNumericForLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_rejects_empty_guarded_numeric_for_loop_body() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 100,
        loop_tail_pc: 102,
        insts: vec![
            TraceIrInst {
                pc: 100,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 6, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 101,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(104)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 102,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(100)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 100,
            branch_pc: 101,
            exit_pc: 104,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 100,
        loop_tail_pc: 102,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(104)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(100)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 3,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(!matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { .. })
                    | CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { .. })
                    | CompiledTraceExecution::Native(
                        NativeCompiledTrace::GuardedNumericForLoop { .. }
                    )
            ));
        }
        BackendCompileOutcome::NotYetSupported => {}
    }
}

#[test]
fn native_backend_compiles_guarded_call_jmp_loop_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 17,
        loop_tail_pc: 24,
        insts: vec![
            TraceIrInst {
                pc: 17,
                opcode: OpCode::Test,
                raw_instruction: Instruction::create_abc(OpCode::Test, 4, 0, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 18,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(26)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 19,
                opcode: OpCode::Add,
                raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(6)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 20,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 8, 5, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 21,
                opcode: OpCode::Call,
                raw_instruction: Instruction::create_abc(OpCode::Call, 8, 2, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
                writes: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 22,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 6, 8, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(8)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            TraceIrInst {
                pc: 23,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 7, 9, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(9)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 24,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -8).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(17)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 17,
            branch_pc: 18,
            exit_pc: 26,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 17,
        loop_tail_pc: 24,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(26)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(6)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::Call {
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
                writes: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(8)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(9)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(17)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 8,
            guards_observed: 1,
            call_steps: 1,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::GuardedCallJmpLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_numeric_jmp_loop_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 110,
        loop_tail_pc: 113,
        insts: vec![
            TraceIrInst {
                pc: 110,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 111,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(115)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 112,
                opcode: OpCode::AddK,
                raw_instruction: Instruction::create_abc(OpCode::AddK, 4, 4, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 113,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(110)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 110,
            branch_pc: 111,
            exit_pc: 115,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 110,
        loop_tail_pc: 113,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(115)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(110)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 4,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

