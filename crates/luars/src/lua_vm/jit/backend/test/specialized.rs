use super::*;

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
                raw_instruction: Instruction::create_abck(OpCode::Add, 5, 5, 13, false)
                    .as_u32(),
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

    match backend.compile_test(&ir, &helper_plan) {
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
fn backend_marks_numeric_for_table_mul_add_mod_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 5,
        loop_tail_pc: 12,
        insts: vec![
            TraceIrInst {
                pc: 5,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abc(OpCode::GetTable, 5, 0, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 6,
                opcode: OpCode::Mul,
                raw_instruction: Instruction::create_abck(OpCode::Mul, 5, 5, 4, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 7,
                opcode: OpCode::MmBin,
                raw_instruction: Instruction::create_abc(OpCode::MmBin, 5, 4, 8).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(4)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 8,
                opcode: OpCode::Add,
                raw_instruction: Instruction::create_abck(OpCode::Add, 5, 1, 5, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(1), TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 9,
                opcode: OpCode::MmBin,
                raw_instruction: Instruction::create_abc(OpCode::MmBin, 1, 5, 6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(1), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 10,
                opcode: OpCode::ModK,
                raw_instruction: Instruction::create_abc(OpCode::ModK, 1, 5, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(1)],
            },
            TraceIrInst {
                pc: 11,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 5, 0, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::ConstantIndex(0)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 12,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 2, 8).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(5)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 5,
        loop_tail_pc: 12,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 8,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 3,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForTableMulAddMod {
                    loop_reg: 2,
                    table_reg: 0,
                    index_reg: 4,
                    acc_reg: 1,
                    modulo_const: 0,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_marks_numeric_for_table_copy_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 6,
        loop_tail_pc: 8,
        insts: vec![
            TraceIrInst {
                pc: 6,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abc(OpCode::GetTable, 5, 0, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 7,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 1, 4, 5, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(1),
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(5),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 8,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 2, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(6)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 6,
        loop_tail_pc: 8,
        steps: vec![],
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
                CompiledTraceExecutor::NumericForTableCopy {
                    loop_reg: 2,
                    src_table_reg: 0,
                    dst_table_reg: 1,
                    index_reg: 4,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_marks_numeric_for_table_is_sorted_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 4,
        loop_tail_pc: 10,
        insts: vec![
            TraceIrInst {
                pc: 4,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 4, 3, 126).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::SignedImmediate(-1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 5,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 3, 1, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::SignedImmediate(1)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 6,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abc(OpCode::GetTable, 4, 0, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 7,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abc(OpCode::GetTable, 5, 0, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(3)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 8,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 5, 4, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(4), TraceIrOperand::Bool(false)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 9,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(12)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 10,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 1, 7).as_u32(),
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
        loop_tail_pc: 10,
        steps: vec![],
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
            let expected = CompiledTraceExecutor::NumericForTableIsSorted {
                loop_reg: 1,
                table_reg: 0,
                index_reg: 3,
                false_exit_pc: 10,
            };
            let actual = compiled.executor();
            if actual != expected {
                panic!(
                    "actual={actual:?} ops={:?} guards={:?}",
                    ir.insts
                        .iter()
                        .map(|inst| (inst.pc, inst.opcode, inst.raw_instruction))
                        .collect::<Vec<_>>(),
                    ir.guards
                );
            }
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_compiles_real_quicksort_is_sorted_trace() {
    let mut vm = crate::LuaVM::new(crate::SafeOption::default());
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/bench_quicksort.lua"
    ))
    .unwrap();
    let chunk = vm
        .compile_with_name(&source, "@bench_quicksort.lua")
        .unwrap();

    let is_sorted = chunk
        .child_protos
        .iter()
        .find_map(|proto| {
            let proto = &proto.as_ref().data;
            (proto.linedefined == 108 && proto.lastlinedefined == 115).then_some(proto)
        })
        .unwrap();

    let artifact = crate::lua_vm::jit::trace_recorder::TraceRecorder::record_root(
        is_sorted as *const crate::lua_value::LuaProto,
        4,
    )
    .unwrap();
    let ir = crate::lua_vm::jit::ir::TraceIr::lower(&artifact);
    let helper_plan = crate::lua_vm::jit::helper_plan::HelperPlan::lower(&ir);
    let mut backend = NullTraceBackend;

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            let actual = compiled.executor();
            let expected = CompiledTraceExecutor::NumericForTableIsSorted {
                loop_reg: 1,
                table_reg: 0,
                index_reg: 3,
                false_exit_pc: 10,
            };
            if actual != expected {
                panic!(
                    "actual={actual:?} ops={:?} guards={:?}",
                    ir.insts
                        .iter()
                        .map(|inst| (inst.pc, inst.opcode, inst.raw_instruction))
                        .collect::<Vec<_>>(),
                    ir.guards
                );
            }
        }
        BackendCompileOutcome::NotYetSupported => panic!(
            "expected compiled trace, got ops={:?} guards={:?}",
            ir.insts.iter().map(|inst| inst.opcode).collect::<Vec<_>>(),
            ir.guards
        ),
    }
}

fn backend_marks_numeric_for_upvalue_addi_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 4,
        loop_tail_pc: 8,
        insts: vec![
            TraceIrInst {
                pc: 4,
                opcode: OpCode::GetUpval,
                raw_instruction: Instruction::create_abc(OpCode::GetUpval, 3, 1, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::UpvalueAccess,
                reads: vec![TraceIrOperand::Upvalue(1)],
                writes: vec![TraceIrOperand::Register(3)],
            },
            TraceIrInst {
                pc: 5,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 3, 3, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(3)],
            },
            TraceIrInst {
                pc: 6,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 3, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 7,
                opcode: OpCode::SetUpval,
                raw_instruction: Instruction::create_abck(OpCode::SetUpval, 3, 1, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::UpvalueMutation,
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::Upvalue(1)],
                writes: vec![TraceIrOperand::Upvalue(1)],
            },
            TraceIrInst {
                pc: 8,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 0, 5).as_u32(),
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
        steps: vec![],
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
                CompiledTraceExecutor::NumericForUpvalueAddI {
                    loop_reg: 0,
                    upvalue: 1,
                    value_reg: 3,
                    imm: 1,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

fn backend_marks_numeric_for_field_addi_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 9,
        loop_tail_pc: 13,
        insts: vec![
            TraceIrInst {
                pc: 9,
                opcode: OpCode::GetField,
                raw_instruction: Instruction::create_abc(OpCode::GetField, 10, 6, 16).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::ConstantIndex(16),
                ],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 10,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 10, 10, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 11,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 10, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 12,
                opcode: OpCode::SetField,
                raw_instruction: Instruction::create_abc(OpCode::SetField, 6, 16, 10).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(6),
                    TraceIrOperand::ConstantIndex(16),
                    TraceIrOperand::Register(10),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 13,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 7, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(9)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 9,
        loop_tail_pc: 13,
        steps: vec![],
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
                CompiledTraceExecutor::NumericForFieldAddI {
                    loop_reg: 7,
                    table_reg: 6,
                    value_reg: 10,
                    key_const: 16,
                    imm: 1,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

fn backend_marks_numeric_for_tabup_field_addi_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 104,
        loop_tail_pc: 110,
        insts: vec![
            TraceIrInst {
                pc: 104,
                opcode: OpCode::GetTabUp,
                raw_instruction: Instruction::create_abc(OpCode::GetTabUp, 9, 0, 21).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Upvalue(0),
                    TraceIrOperand::ConstantIndex(21),
                ],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 105,
                opcode: OpCode::GetTabUp,
                raw_instruction: Instruction::create_abc(OpCode::GetTabUp, 10, 0, 21).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Upvalue(0),
                    TraceIrOperand::ConstantIndex(21),
                ],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 106,
                opcode: OpCode::GetField,
                raw_instruction: Instruction::create_abc(OpCode::GetField, 10, 10, 22).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::ConstantIndex(22),
                ],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 107,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 10, 10, 128).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 108,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 10, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 109,
                opcode: OpCode::SetField,
                raw_instruction: Instruction::create_abc(OpCode::SetField, 9, 22, 10).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(9),
                    TraceIrOperand::ConstantIndex(22),
                    TraceIrOperand::Register(10),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 110,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 7, 6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(104)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 104,
        loop_tail_pc: 110,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 7,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 1,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForTabUpFieldAddI {
                    loop_reg: 7,
                    env_upvalue: 0,
                    table_reg: 9,
                    value_reg: 10,
                    table_key_const: 21,
                    field_key_const: 22,
                    imm: 1,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_marks_numeric_for_tabup_field_load_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 167,
        loop_tail_pc: 169,
        insts: vec![
            TraceIrInst {
                pc: 167,
                opcode: OpCode::GetTabUp,
                raw_instruction: Instruction::create_abc(OpCode::GetTabUp, 10, 0, 30).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Upvalue(0),
                    TraceIrOperand::ConstantIndex(30),
                ],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 168,
                opcode: OpCode::GetField,
                raw_instruction: Instruction::create_abc(OpCode::GetField, 10, 10, 31).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(10),
                    TraceIrOperand::ConstantIndex(31),
                ],
                writes: vec![TraceIrOperand::Register(10)],
            },
            TraceIrInst {
                pc: 169,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 8, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(167)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 167,
        loop_tail_pc: 169,
        steps: vec![],
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
                CompiledTraceExecutor::NumericForTabUpFieldLoad {
                    loop_reg: 8,
                    env_upvalue: 0,
                    value_reg: 10,
                    table_key_const: 30,
                    field_key_const: 31,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_marks_numeric_for_builtin_unary_const_call_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 196,
        loop_tail_pc: 199,
        insts: vec![
            TraceIrInst {
                pc: 196,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 11, 7, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            TraceIrInst {
                pc: 197,
                opcode: OpCode::LoadK,
                raw_instruction: Instruction::create_abx(OpCode::LoadK, 12, 23).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::ConstantIndex(23)],
                writes: vec![TraceIrOperand::Register(12)],
            },
            TraceIrInst {
                pc: 198,
                opcode: OpCode::Call,
                raw_instruction: Instruction::create_abc(OpCode::Call, 11, 2, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                reads: vec![TraceIrOperand::Register(11), TraceIrOperand::Register(12)],
                writes: vec![TraceIrOperand::Register(11)],
            },
            TraceIrInst {
                pc: 199,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 8, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(196)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 196,
        loop_tail_pc: 199,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 4,
            guards_observed: 0,
            call_steps: 1,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForBuiltinUnaryConstCall {
                    loop_reg: 8,
                    func_reg: 7,
                    result_reg: 11,
                    arg_const: 23,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_marks_numeric_for_tabup_field_string_unary_call_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 22,
        loop_tail_pc: 26,
        insts: vec![
            TraceIrInst {
                pc: 22,
                opcode: OpCode::GetTabUp,
                raw_instruction: Instruction::create_abc(OpCode::GetTabUp, 8, 0, 17).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Upvalue(0),
                    TraceIrOperand::ConstantIndex(17),
                ],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 23,
                opcode: OpCode::GetField,
                raw_instruction: Instruction::create_abc(OpCode::GetField, 8, 8, 18).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(8),
                    TraceIrOperand::ConstantIndex(18),
                ],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 24,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 9, 3, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(3)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 25,
                opcode: OpCode::Call,
                raw_instruction: Instruction::create_abc(OpCode::Call, 8, 2, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 26,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 4, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(22)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 22,
        loop_tail_pc: 26,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 0,
            call_steps: 1,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForTabUpFieldStringUnaryCall {
                    loop_reg: 4,
                    env_upvalue: 0,
                    result_reg: 8,
                    arg_reg: 3,
                    table_key_const: 17,
                    field_key_const: 18,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_marks_numeric_for_lua_closure_addi_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 18,
        loop_tail_pc: 23,
        insts: vec![
            TraceIrInst {
                pc: 18,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 7, 1, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(1)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 19,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 8, 6, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(6)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 20,
                opcode: OpCode::LoadI,
                raw_instruction: Instruction::create_asbx(OpCode::LoadI, 9, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::SignedImmediate(5)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 21,
                opcode: OpCode::Call,
                raw_instruction: Instruction::create_abc(OpCode::Call, 7, 3, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                reads: vec![
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(8),
                    TraceIrOperand::Register(9),
                ],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 22,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 3, 7, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(3)],
            },
            TraceIrInst {
                pc: 23,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 4, 6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(18)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 18,
        loop_tail_pc: 23,
        steps: vec![],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 6,
            guards_observed: 0,
            call_steps: 1,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForLuaClosureAddI {
                    loop_reg: 4,
                    func_reg: 1,
                    arg_reg: 6,
                    dst_reg: 3,
                    imm: 5,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn backend_marks_numeric_for_sort_check_sum_loop_as_executable() {
    let mut backend = NullTraceBackend;
    let ir = TraceIr {
        root_pc: 34,
        loop_tail_pc: 57,
        insts: vec![
            TraceIrInst { pc: 34, opcode: OpCode::Move, raw_instruction: Instruction::create_abc(OpCode::Move, 17, 6, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::Register(6)], writes: vec![TraceIrOperand::Register(17)] },
            TraceIrInst { pc: 35, opcode: OpCode::Move, raw_instruction: Instruction::create_abc(OpCode::Move, 18, 5, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::Register(5)], writes: vec![TraceIrOperand::Register(18)] },
            TraceIrInst { pc: 36, opcode: OpCode::Call, raw_instruction: Instruction::create_abc(OpCode::Call, 17, 2, 2).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call, reads: vec![TraceIrOperand::Register(17), TraceIrOperand::Register(18)], writes: vec![TraceIrOperand::Register(17)] },
            TraceIrInst { pc: 37, opcode: OpCode::Move, raw_instruction: Instruction::create_abc(OpCode::Move, 18, 9, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::Register(9)], writes: vec![TraceIrOperand::Register(18)] },
            TraceIrInst { pc: 38, opcode: OpCode::Move, raw_instruction: Instruction::create_abc(OpCode::Move, 19, 17, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::Register(17)], writes: vec![TraceIrOperand::Register(19)] },
            TraceIrInst { pc: 39, opcode: OpCode::LoadI, raw_instruction: Instruction::create_asbx(OpCode::LoadI, 20, 1).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::SignedImmediate(1)], writes: vec![TraceIrOperand::Register(20)] },
            TraceIrInst { pc: 40, opcode: OpCode::Len, raw_instruction: Instruction::create_abc(OpCode::Len, 21, 17, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic, reads: vec![TraceIrOperand::Register(17)], writes: vec![TraceIrOperand::Register(21)] },
            TraceIrInst { pc: 41, opcode: OpCode::Call, raw_instruction: Instruction::create_abc(OpCode::Call, 18, 4, 1).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call, reads: vec![TraceIrOperand::Register(18), TraceIrOperand::RegisterRange { start: 19, count: 3 }], writes: Vec::new() },
            TraceIrInst { pc: 42, opcode: OpCode::Move, raw_instruction: Instruction::create_abc(OpCode::Move, 18, 11, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::Register(11)], writes: vec![TraceIrOperand::Register(18)] },
            TraceIrInst { pc: 43, opcode: OpCode::Move, raw_instruction: Instruction::create_abc(OpCode::Move, 19, 17, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::Register(17)], writes: vec![TraceIrOperand::Register(19)] },
            TraceIrInst { pc: 44, opcode: OpCode::Call, raw_instruction: Instruction::create_abc(OpCode::Call, 18, 2, 2).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call, reads: vec![TraceIrOperand::Register(18), TraceIrOperand::Register(19)], writes: vec![TraceIrOperand::Register(18)] },
            TraceIrInst { pc: 45, opcode: OpCode::Test, raw_instruction: Instruction::create_abck(OpCode::Test, 18, 0, 1, true).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard, reads: vec![TraceIrOperand::Register(18), TraceIrOperand::Bool(true)], writes: Vec::new() },
            TraceIrInst { pc: 46, opcode: OpCode::Jmp, raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch, reads: vec![TraceIrOperand::JumpTarget(50)], writes: Vec::new() },
            TraceIrInst { pc: 47, opcode: OpCode::GetTabUp, raw_instruction: Instruction::create_abc(OpCode::GetTabUp, 18, 0, 7).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess, reads: vec![TraceIrOperand::Upvalue(0), TraceIrOperand::ConstantIndex(7)], writes: vec![TraceIrOperand::Register(18)] },
            TraceIrInst { pc: 48, opcode: OpCode::LoadK, raw_instruction: Instruction::create_abx(OpCode::LoadK, 19, 8).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::ConstantIndex(8)], writes: vec![TraceIrOperand::Register(19)] },
            TraceIrInst { pc: 49, opcode: OpCode::Call, raw_instruction: Instruction::create_abc(OpCode::Call, 18, 2, 1).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call, reads: vec![TraceIrOperand::Register(18), TraceIrOperand::Register(19)], writes: Vec::new() },
            TraceIrInst { pc: 50, opcode: OpCode::Move, raw_instruction: Instruction::create_abc(OpCode::Move, 18, 10, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::Register(10)], writes: vec![TraceIrOperand::Register(18)] },
            TraceIrInst { pc: 51, opcode: OpCode::Move, raw_instruction: Instruction::create_abc(OpCode::Move, 19, 17, 0).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove, reads: vec![TraceIrOperand::Register(17)], writes: vec![TraceIrOperand::Register(19)] },
            TraceIrInst { pc: 52, opcode: OpCode::Call, raw_instruction: Instruction::create_abc(OpCode::Call, 18, 2, 2).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call, reads: vec![TraceIrOperand::Register(18), TraceIrOperand::Register(19)], writes: vec![TraceIrOperand::Register(18)] },
            TraceIrInst { pc: 53, opcode: OpCode::Add, raw_instruction: Instruction::create_abck(OpCode::Add, 18, 13, 18, false).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic, reads: vec![TraceIrOperand::Register(13), TraceIrOperand::Register(18)], writes: vec![TraceIrOperand::Register(18)] },
            TraceIrInst { pc: 54, opcode: OpCode::MmBin, raw_instruction: Instruction::create_abc(OpCode::MmBin, 13, 18, 6).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback, reads: vec![TraceIrOperand::Register(13), TraceIrOperand::Register(18)], writes: Vec::new() },
            TraceIrInst { pc: 55, opcode: OpCode::ModK, raw_instruction: Instruction::create_abc(OpCode::ModK, 13, 18, 9).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic, reads: vec![TraceIrOperand::Register(18), TraceIrOperand::ConstantIndex(9)], writes: vec![TraceIrOperand::Register(13)] },
            TraceIrInst { pc: 56, opcode: OpCode::MmBinK, raw_instruction: Instruction::create_abck(OpCode::MmBinK, 18, 9, 9, false).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback, reads: vec![TraceIrOperand::Register(18), TraceIrOperand::ConstantIndex(9)], writes: Vec::new() },
            TraceIrInst { pc: 57, opcode: OpCode::ForLoop, raw_instruction: Instruction::create_abx(OpCode::ForLoop, 14, 24).as_u32(), kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge, reads: vec![TraceIrOperand::JumpTarget(34)], writes: Vec::new() },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 45,
            branch_pc: 46,
            exit_pc: 50,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 34,
        loop_tail_pc: 57,
        steps: vec![],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 24,
            guards_observed: 1,
            call_steps: 5,
            metamethod_steps: 2,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert_eq!(
                compiled.executor(),
                CompiledTraceExecutor::NumericForArraySortValidateChecksumLoop {
                    loop_reg: 14,
                    source_reg: 5,
                    work_reg: 17,
                    sum_reg: 13,
                    copy_func_reg: 6,
                    sort_func_reg: 9,
                    check_func_reg: 11,
                    checksum_func_reg: 10,
                    modulo_const: 9,
                }
            );
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}
