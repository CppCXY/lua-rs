use super::*;

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
                raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 13, false)
                    .as_u32(),
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
                writes: vec![TraceIrOperand::RegisterRange {
                    start: 12,
                    count: 2,
                }],
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
                writes: vec![TraceIrOperand::RegisterRange {
                    start: 12,
                    count: 2,
                }],
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

    match backend.compile_test(&ir, &helper_plan) {
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
                raw_instruction: Instruction::create_abck(OpCode::Test, 10, 0, 0, false)
                    .as_u32(),
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
                raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 11, false)
                    .as_u32(),
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
                reads: vec![
                    TraceIrOperand::Upvalue(0),
                    TraceIrOperand::ConstantIndex(15),
                ],
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
                writes: vec![TraceIrOperand::RegisterRange {
                    start: 12,
                    count: 2,
                }],
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
                reads: vec![
                    TraceIrOperand::Upvalue(0),
                    TraceIrOperand::ConstantIndex(15),
                ],
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
                writes: vec![TraceIrOperand::RegisterRange {
                    start: 12,
                    count: 2,
                }],
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

    match backend.compile_test(&ir, &helper_plan) {
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
