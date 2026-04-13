use super::*;
use super::shared::*;

#[test]
fn native_backend_compiles_table_touching_numeric_jmp_loop_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 310,
        loop_tail_pc: 315,
        insts: vec![
            TraceIrInst {
                pc: 310,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 311,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(317)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 312,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 8, 2, 4, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 313,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 3, 4, 8, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(8),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 314,
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
                pc: 315,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(310)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 310,
            branch_pc: 311,
            exit_pc: 317,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 310,
        loop_tail_pc: 315,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(317)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(8),
                ],
                writes: vec![],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(310)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 6,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { .. }) => {}
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_numeric_jmp_guard_block_sequences_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let head_blocks = [NumericJmpLoopGuardBlock {
        pre_steps: vec![NumericStep::GetTableInt {
            dst: 8,
            table: 2,
            index: 4,
        }],
        guard: NumericJmpLoopGuard::Head {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 8,
                rhs: 5,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 700,
        },
    }];
    let steps = [
        NumericStep::SetTableInt {
            table: 3,
            index: 4,
            value: 8,
        },
        NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        },
        NumericStep::Binary {
            dst: 6,
            lhs: NumericOperand::Reg(6),
            rhs: NumericOperand::ImmI(-1),
            op: NumericBinaryOp::Add,
        },
    ];
    let tail_blocks = [
        NumericJmpLoopGuardBlock {
            pre_steps: Vec::new(),
            guard: NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::Truthy { reg: 9 },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 701,
            },
        },
        NumericJmpLoopGuardBlock {
            pre_steps: vec![NumericStep::Move { dst: 10, src: 6 }],
            guard: NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::RegCompare {
                    op: LinearIntGuardOp::Lt,
                    lhs: 4,
                    rhs: 10,
                },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 702,
            },
        },
    ];
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        600,
        603,
        &[(610, 611, 700), (612, 613, 701), (614, 615, 702)],
        &[
            (2, TraceValueKind::Table),
            (3, TraceValueKind::Table),
            (4, TraceValueKind::Integer),
            (5, TraceValueKind::Integer),
            (6, TraceValueKind::Integer),
            (9, TraceValueKind::Boolean),
        ],
    );

    match backend.compile_test_numeric_jmp_blocks(
        &head_blocks,
        &steps,
        &tail_blocks,
        &lowered_trace,
    ) {
        Some(NativeCompiledTrace::NumericJmpLoop { .. }) => {}
        other => panic!("expected native numeric jmp execution, got {other:?}"),
    }
}

#[test]
fn native_numeric_jmp_guard_block_sequences_execute_and_side_exit() {
    let mut backend = NativeTraceBackend::default();
    let head_blocks = [NumericJmpLoopGuardBlock {
        pre_steps: vec![NumericStep::GetTableInt {
            dst: 8,
            table: 2,
            index: 4,
        }],
        guard: NumericJmpLoopGuard::Head {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 8,
                rhs: 5,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 700,
        },
    }];
    let steps = [
        NumericStep::SetTableInt {
            table: 3,
            index: 4,
            value: 8,
        },
        NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        },
        NumericStep::Binary {
            dst: 6,
            lhs: NumericOperand::Reg(6),
            rhs: NumericOperand::ImmI(-1),
            op: NumericBinaryOp::Add,
        },
    ];
    let tail_blocks = [
        NumericJmpLoopGuardBlock {
            pre_steps: Vec::new(),
            guard: NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::Truthy { reg: 9 },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 701,
            },
        },
        NumericJmpLoopGuardBlock {
            pre_steps: vec![NumericStep::Move { dst: 10, src: 6 }],
            guard: NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::RegCompare {
                    op: LinearIntGuardOp::Lt,
                    lhs: 4,
                    rhs: 10,
                },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 702,
            },
        },
    ];
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        600,
        603,
        &[(610, 611, 700), (612, 613, 701), (614, 615, 702)],
        &[
            (2, TraceValueKind::Table),
            (3, TraceValueKind::Table),
            (4, TraceValueKind::Integer),
            (5, TraceValueKind::Integer),
            (6, TraceValueKind::Integer),
            (9, TraceValueKind::Boolean),
        ],
    );

    let entry = match backend.compile_test_numeric_jmp_blocks(
        &head_blocks,
        &steps,
        &tail_blocks,
        &lowered_trace,
    ) {
        Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
        other => panic!("expected native numeric jmp execution, got {other:?}"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    let src_table = vm.create_table(8, 0).unwrap();
    let dst_table = vm.create_table(8, 0).unwrap();
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(1, LuaValue::integer(11));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(2, LuaValue::integer(22));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(3, LuaValue::integer(33));

    let mut stack = [LuaValue::nil(); 12];
    stack[2] = src_table;
    stack[3] = dst_table;
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(100);
    stack[6] = LuaValue::integer(4);
    stack[9] = LuaValue::boolean(true);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            vm.main_state(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 2);
    assert_eq!(result.exit_pc, 702);
    assert_eq!(result.exit_index, 2);
    assert_eq!(stack[4].as_integer(), Some(3));
    assert_eq!(stack[6].as_integer(), Some(2));
    assert_eq!(stack[8].as_integer(), Some(22));
    assert_eq!(stack[10].as_integer(), Some(2));
    assert_eq!(
        dst_table.as_table().unwrap().raw_geti(1),
        Some(LuaValue::integer(11))
    );
    assert_eq!(
        dst_table.as_table().unwrap().raw_geti(2),
        Some(LuaValue::integer(22))
    );

    let deopt = lowered_trace
        .deopt_target_for_exit_index(result.exit_index.try_into().unwrap())
        .unwrap();
    assert_eq!(deopt.resume_pc, 702);
}

#[test]
fn native_numeric_jmp_head_prestep_missing_table_value_side_exits_after_advancing_index() {
    let mut backend = NativeTraceBackend::default();
    let head_blocks = [NumericJmpLoopGuardBlock {
        pre_steps: vec![NumericStep::GetTableInt {
            dst: 8,
            table: 2,
            index: 4,
        }],
        guard: NumericJmpLoopGuard::Head {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 8,
                rhs: 5,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 800,
        },
    }];
    let steps = [
        NumericStep::Move { dst: 9, src: 4 },
        NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        },
    ];
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        790,
        793,
        &[(791, 792, 800)],
        &[
            (2, TraceValueKind::Table),
            (4, TraceValueKind::Integer),
            (5, TraceValueKind::Integer),
        ],
    );

    let entry =
        match backend.compile_test_numeric_jmp_blocks(&head_blocks, &steps, &[], &lowered_trace) {
            Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        };

    let mut vm = LuaVM::new(SafeOption::default());
    let src_table = vm.create_table(4, 0).unwrap();
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(1, LuaValue::integer(1));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(2, LuaValue::integer(2));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(3, LuaValue::integer(3));

    let mut stack = [LuaValue::nil(); 10];
    stack[2] = src_table;
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(100);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            vm.main_state(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 3);
    assert_eq!(result.exit_pc, 800);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(4));
}

#[test]
fn native_backend_compiles_generic_numeric_jmp_with_multiple_head_guards() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 500,
        loop_tail_pc: 505,
        insts: vec![
            TraceIrInst {
                pc: 500,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 501,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(510)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 502,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 6, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 503,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(511)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 504,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 7, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 505,
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
                pc: 506,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(500)],
                writes: Vec::new(),
            },
        ],
        guards: vec![
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 500,
                branch_pc: 501,
                exit_pc: 510,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            },
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 502,
                branch_pc: 503,
                exit_pc: 511,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            },
        ],
    };
    let helper_plan = HelperPlan {
        root_pc: 500,
        loop_tail_pc: 506,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(510)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(511)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(500)],
                writes: Vec::new(),
            },
        ],
        guard_count: 2,
        summary: HelperPlanDispatchSummary {
            steps_executed: 7,
            guards_observed: 2,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { .. }) => {}
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_recognizes_generic_numeric_jmp_with_multiple_head_guards() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 500,
        loop_tail_pc: 506,
        insts: vec![
            TraceIrInst {
                pc: 500,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 501,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(510)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 502,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 6, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 503,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(511)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 504,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 7, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 505,
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
                pc: 506,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(500)],
                writes: Vec::new(),
            },
        ],
        guards: vec![
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 500,
                branch_pc: 501,
                exit_pc: 510,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            },
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 502,
                branch_pc: 503,
                exit_pc: 511,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            },
        ],
    };
    let helper_plan = HelperPlan {
        root_pc: 500,
        loop_tail_pc: 506,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(510)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(511)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(500)],
                writes: Vec::new(),
            },
        ],
        guard_count: 2,
        summary: HelperPlanDispatchSummary {
            steps_executed: 7,
            guards_observed: 2,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };
    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { .. }) => {}
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_recognizes_generic_numeric_jmp_with_multiple_tail_guards() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 520,
        loop_tail_pc: 526,
        insts: vec![
            TraceIrInst {
                pc: 520,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 7, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 521,
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
                pc: 522,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, true).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 523,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(530)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 524,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 6, 0, true).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 525,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(531)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 526,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(520)],
                writes: Vec::new(),
            },
        ],
        guards: vec![
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 522,
                branch_pc: 523,
                exit_pc: 530,
                taken_on_trace: true,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard,
            },
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 524,
                branch_pc: 525,
                exit_pc: 531,
                taken_on_trace: true,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard,
            },
        ],
    };
    let helper_plan = HelperPlan {
        root_pc: 520,
        loop_tail_pc: 526,
        steps: vec![
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(530)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(531)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(520)],
                writes: Vec::new(),
            },
        ],
        guard_count: 2,
        summary: HelperPlanDispatchSummary {
            steps_executed: 7,
            guards_observed: 2,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };
    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { .. }) => {}
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_recognizes_generic_numeric_jmp_with_head_and_tail_guards() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 540,
        loop_tail_pc: 547,
        insts: vec![
            TraceIrInst {
                pc: 540,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 541,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 8).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(550)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 542,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 7, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 543,
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
                pc: 544,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 8, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 545,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 6, 0, true).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 546,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(551)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 547,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -8).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(540)],
                writes: Vec::new(),
            },
        ],
        guards: vec![
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 540,
                branch_pc: 541,
                exit_pc: 550,
                taken_on_trace: false,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
            },
            crate::lua_vm::jit::ir::TraceIrGuard {
                guard_pc: 545,
                branch_pc: 546,
                exit_pc: 551,
                taken_on_trace: true,
                kind: crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard,
            },
        ],
    };
    let helper_plan = HelperPlan {
        root_pc: 540,
        loop_tail_pc: 547,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(550)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(551)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(540)],
                writes: Vec::new(),
            },
        ],
        guard_count: 2,
        summary: HelperPlanDispatchSummary {
            steps_executed: 8,
            guards_observed: 2,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { .. }) => {}
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_recognizes_generic_numeric_jmp_with_head_prestep() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 560,
        loop_tail_pc: 564,
        insts: vec![
            TraceIrInst {
                pc: 560,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 8, 2, 4, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 561,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 8, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 562,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(566)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 563,
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
                pc: 564,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(560)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 561,
            branch_pc: 562,
            exit_pc: 566,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 560,
        loop_tail_pc: 564,
        steps: vec![
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(566)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(560)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { .. }) => {}
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_generic_numeric_jmp_head_prestep_compile_test_side_exits_on_missing_value() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 560,
        loop_tail_pc: 565,
        insts: vec![
            TraceIrInst {
                pc: 560,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 7, 0, 5, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 561,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 7, 4, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Register(4)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 562,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(567)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 563,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 8, 5, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 564,
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
                pc: 565,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(560)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 561,
            branch_pc: 562,
            exit_pc: 567,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 560,
        loop_tail_pc: 565,
        steps: vec![
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(567)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(560)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 6,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    let src_table = vm.create_table(4, 0).unwrap();
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(1, LuaValue::integer(1));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(2, LuaValue::integer(2));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(3, LuaValue::integer(3));

    let mut stack = [LuaValue::nil(); 12];
    stack[0] = src_table;
    stack[4] = LuaValue::integer(100);
    stack[5] = LuaValue::integer(1);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            vm.main_state(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 3);
    assert_eq!(result.exit_pc, 567);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[5].as_integer(), Some(4));
}

#[test]
fn native_numeric_jmp_head_prestep_with_lowered_trace_from_ir_side_exits_on_missing_value() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 560,
        loop_tail_pc: 565,
        insts: vec![
            TraceIrInst {
                pc: 560,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 7, 0, 5, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 561,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 7, 4, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Register(4)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 562,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(567)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 563,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 8, 5, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 564,
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
                pc: 565,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(560)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 561,
            branch_pc: 562,
            exit_pc: 567,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 560,
        loop_tail_pc: 565,
        steps: vec![
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(567)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(5),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(560)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 6,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };
    let artifact = crate::lua_vm::jit::backend::synthetic_artifact_for_ir(&ir);
    let lowered_trace = LoweredTrace::lower(&artifact, &ir, &helper_plan);
    let head_blocks = [NumericJmpLoopGuardBlock {
        pre_steps: vec![NumericStep::GetTableInt {
            dst: 7,
            table: 0,
            index: 5,
        }],
        guard: NumericJmpLoopGuard::Head {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 7,
                rhs: 4,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 567,
        },
    }];
    let steps = [
        NumericStep::Move { dst: 8, src: 5 },
        NumericStep::Binary {
            dst: 5,
            lhs: NumericOperand::Reg(5),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        },
    ];

    let entry =
        match backend.compile_test_numeric_jmp_blocks(&head_blocks, &steps, &[], &lowered_trace) {
            Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        };

    let mut vm = LuaVM::new(SafeOption::default());
    let src_table = vm.create_table(4, 0).unwrap();
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(1, LuaValue::integer(1));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(2, LuaValue::integer(2));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(3, LuaValue::integer(3));

    let mut stack = [LuaValue::nil(); 12];
    stack[0] = src_table;
    stack[4] = LuaValue::integer(100);
    stack[5] = LuaValue::integer(1);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            vm.main_state(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 3);
    assert_eq!(result.exit_pc, 567);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[5].as_integer(), Some(4));
}

#[test]
fn native_table_touching_numeric_jmp_loop_entry_executes_and_side_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 310,
        loop_tail_pc: 315,
        insts: vec![
            TraceIrInst {
                pc: 310,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 311,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(317)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 312,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 8, 2, 4, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 313,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 3, 4, 8, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(8),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 314,
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
                pc: 315,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -6).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(310)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 310,
            branch_pc: 311,
            exit_pc: 317,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 310,
        loop_tail_pc: 315,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(317)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::TableAccess {
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(8),
                ],
                writes: vec![],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(310)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 6,
            guards_observed: 1,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    let src_table = vm.create_table(8, 0).unwrap();
    let dst_table = vm.create_table(8, 0).unwrap();
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(1, LuaValue::integer(11));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(2, LuaValue::integer(22));
    src_table
        .as_table_mut()
        .unwrap()
        .raw_seti(3, LuaValue::integer(33));

    let mut stack = [LuaValue::nil(); 12];
    stack[2] = src_table;
    stack[3] = dst_table;
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(4);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            vm.main_state(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 3);
    assert_eq!(result.exit_pc, 317);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(4));
    assert_eq!(
        dst_table.as_table().unwrap().raw_geti(1),
        Some(LuaValue::integer(11))
    );
    assert_eq!(
        dst_table.as_table().unwrap().raw_geti(2),
        Some(LuaValue::integer(22))
    );
    assert_eq!(
        dst_table.as_table().unwrap().raw_geti(3),
        Some(LuaValue::integer(33))
    );
}

