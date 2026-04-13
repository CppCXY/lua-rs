use super::*;
use super::shared::*;

#[test]
fn native_linear_int_jmp_loop_entry_executes_and_side_exits() {
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntJmpLoop { entry }) => {
                entry
            }
            other => panic!("expected native linear-int jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 8];
    stack[0] = LuaValue::integer(128);
    stack[4] = LuaValue::integer(0);
    stack[5] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    println!("head-result={result:?}");
    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 128);
    assert_eq!(result.exit_pc, 56);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(128));
    assert_eq!(stack[5].as_integer(), Some(8128));
}

#[test]
fn native_numeric_for_loop_with_idiv_by_zero_falls_back() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 140,
        loop_tail_pc: 142,
        insts: vec![
            TraceIrInst {
                pc: 140,
                opcode: OpCode::IDivK,
                raw_instruction: Instruction::create_abc(OpCode::IDivK, 4, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 141,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 14, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 142,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(140)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 140,
        loop_tail_pc: 142,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(140)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
            other => panic!("expected native numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 8];
    let constants = [LuaValue::integer(0)];
    stack[4] = LuaValue::integer(7);
    stack[5] = LuaValue::integer(0);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            constants.as_ptr(),
            constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::Fallback);
    assert_eq!(result.hits, 0);
    assert_eq!(stack[4].as_integer(), Some(7));
}

#[test]
fn native_numeric_for_loop_with_mod_by_zero_falls_back() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 150,
        loop_tail_pc: 152,
        insts: vec![
            TraceIrInst {
                pc: 150,
                opcode: OpCode::ModK,
                raw_instruction: Instruction::create_abc(OpCode::ModK, 4, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 151,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 152,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(150)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 150,
        loop_tail_pc: 152,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(150)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
            other => panic!("expected native numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 8];
    let constants = [LuaValue::integer(0)];
    stack[4] = LuaValue::integer(7);
    stack[5] = LuaValue::integer(0);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            constants.as_ptr(),
            constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::Fallback);
    assert_eq!(result.hits, 0);
    assert_eq!(stack[4].as_integer(), Some(7));
}

#[test]
fn native_linear_int_forloop_entry_executes_and_loop_exits() {
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

    let entry =
        match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::integer(2)]) {
            BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => {
                    entry
                }
                other => panic!("expected native linear-int forloop execution, got {other:?}"),
            },
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    stack[7] = LuaValue::integer(3);
    stack[6] = LuaValue::integer(10);
    stack[10] = LuaValue::integer(2);
    stack[11] = LuaValue::integer(1);
    stack[12] = LuaValue::integer(100);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(
        result.status,
        NativeTraceStatus::LoopExit,
        "hits={} r5={:?} r6={:?} r13={:?} r14={:?} r15={:?}",
        result.hits,
        stack[5].as_integer(),
        stack[6].as_integer(),
        stack[13].as_integer(),
        stack[14].as_integer(),
        stack[15].as_integer()
    );
    assert_eq!(result.hits, 3);
    assert_eq!(stack[5].as_integer(), Some(3));
    assert_eq!(stack[6].as_integer(), Some(313));
    assert_eq!(stack[10].as_integer(), Some(0));
    assert_eq!(stack[12].as_integer(), Some(102));
}

#[test]
fn native_linear_int_forloop_reuses_fresh_integer_writes_within_iteration() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 30,
        loop_tail_pc: 33,
        insts: vec![
            TraceIrInst {
                pc: 30,
                opcode: OpCode::LoadI,
                raw_instruction: Instruction::create_asbx(OpCode::LoadI, 5, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: Vec::new(),
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 31,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 6, 5, 130).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            TraceIrInst {
                pc: 32,
                opcode: OpCode::Mul,
                raw_instruction: Instruction::create_abck(OpCode::Mul, 7, 6, 5, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(6), TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 33,
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
        loop_tail_pc: 33,
        steps: vec![
            HelperPlanStep::LoadMove {
                reads: Vec::new(),
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(6), TraceIrOperand::Register(5)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(30)],
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

    let entry =
        match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::integer(2)]) {
            BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => {
                    entry
                }
                other => panic!("expected native linear-int forloop execution, got {other:?}"),
            },
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    stack[10] = LuaValue::integer(1);
    stack[11] = LuaValue::integer(1);
    stack[12] = LuaValue::integer(100);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 2);
    assert_eq!(stack[5].as_integer(), Some(2));
    assert_eq!(stack[6].as_integer(), Some(5));
    assert_eq!(stack[7].as_integer(), Some(10));
    assert_eq!(stack[10].as_integer(), Some(0));
    assert_eq!(stack[12].as_integer(), Some(101));
}

#[test]
fn native_linear_int_forloop_executes_bitwise_and_shift_steps() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 34,
        loop_tail_pc: 39,
        insts: vec![
            TraceIrInst {
                pc: 34,
                opcode: OpCode::BAnd,
                raw_instruction: Instruction::create_abck(OpCode::BAnd, 5, 12, 13, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(12), TraceIrOperand::Register(13)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 35,
                opcode: OpCode::BOr,
                raw_instruction: Instruction::create_abck(OpCode::BOr, 6, 5, 14, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(14)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            TraceIrInst {
                pc: 36,
                opcode: OpCode::BXor,
                raw_instruction: Instruction::create_abck(OpCode::BXor, 7, 6, 15, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(6), TraceIrOperand::Register(15)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            TraceIrInst {
                pc: 37,
                opcode: OpCode::Shl,
                raw_instruction: Instruction::create_abck(OpCode::Shl, 8, 16, 17, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(16), TraceIrOperand::Register(17)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 38,
                opcode: OpCode::ShrI,
                raw_instruction: Instruction::create_abc(OpCode::ShrI, 9, 8, 129).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(8)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 39,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 10, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(34)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 34,
        loop_tail_pc: 39,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(12), TraceIrOperand::Register(13)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(14)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(6), TraceIrOperand::Register(15)],
                writes: vec![TraceIrOperand::Register(7)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(16), TraceIrOperand::Register(17)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(8)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(34)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 6,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 0,
        },
    };

    let entry =
        match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::integer(2)]) {
            BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => {
                    entry
                }
                other => panic!("expected native linear-int forloop execution, got {other:?}"),
            },
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 18];
    stack[10] = LuaValue::integer(0);
    stack[11] = LuaValue::integer(1);
    stack[12] = LuaValue::integer(6);
    stack[13] = LuaValue::integer(3);
    stack[14] = LuaValue::integer(1);
    stack[15] = LuaValue::integer(7);
    stack[16] = LuaValue::integer(1);
    stack[17] = LuaValue::integer(2);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[5].as_integer(), Some(2));
    assert_eq!(stack[6].as_integer(), Some(3));
    assert_eq!(stack[7].as_integer(), Some(4));
    assert_eq!(stack[8].as_integer(), Some(4));
    assert_eq!(stack[9].as_integer(), Some(1));
}

#[test]
fn native_linear_int_forloop_executes_bnot_step() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 43,
        loop_tail_pc: 44,
        insts: vec![
            TraceIrInst {
                pc: 43,
                opcode: OpCode::BNot,
                raw_instruction: Instruction::create_abc(OpCode::BNot, 5, 13, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(13)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 44,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 20, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(43)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 43,
        loop_tail_pc: 44,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(13)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(43)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => {
                entry
            }
            other => panic!("expected native linear-int forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 24];
    stack[13] = LuaValue::integer(6);
    stack[20] = LuaValue::integer(0);
    stack[21] = LuaValue::integer(1);
    stack[22] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[5].as_integer(), Some(!6_i64));
}

#[test]
fn native_linear_int_forloop_executes_integer_idiv_and_mod_steps() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 40,
        loop_tail_pc: 42,
        insts: vec![
            TraceIrInst {
                pc: 40,
                opcode: OpCode::IDiv,
                raw_instruction: Instruction::create_abck(OpCode::IDiv, 5, 13, 15, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(13), TraceIrOperand::Register(15)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 41,
                opcode: OpCode::Mod,
                raw_instruction: Instruction::create_abck(OpCode::Mod, 6, 14, 16, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(14), TraceIrOperand::Register(16)],
                writes: vec![TraceIrOperand::Register(6)],
            },
            TraceIrInst {
                pc: 42,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 20, 3).as_u32(),
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
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(13), TraceIrOperand::Register(15)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(14), TraceIrOperand::Register(16)],
                writes: vec![TraceIrOperand::Register(6)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => {
                entry
            }
            other => panic!("expected native linear-int forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 24];
    stack[13] = LuaValue::integer(14);
    stack[14] = LuaValue::integer(14);
    stack[15] = LuaValue::integer(5);
    stack[16] = LuaValue::integer(5);
    stack[20] = LuaValue::integer(0);
    stack[21] = LuaValue::integer(1);
    stack[22] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[5].as_integer(), Some(2));
    assert_eq!(stack[6].as_integer(), Some(4));
}

#[test]
fn native_linear_int_forloop_with_idiv_by_zero_falls_back() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 43,
        loop_tail_pc: 44,
        insts: vec![
            TraceIrInst {
                pc: 43,
                opcode: OpCode::IDiv,
                raw_instruction: Instruction::create_abck(OpCode::IDiv, 5, 5, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(6)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            TraceIrInst {
                pc: 44,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 20, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(43)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 43,
        loop_tail_pc: 44,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(5), TraceIrOperand::Register(6)],
                writes: vec![TraceIrOperand::Register(5)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(43)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => {
                entry
            }
            other => panic!("expected native linear-int forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 24];
    stack[5] = LuaValue::integer(7);
    stack[6] = LuaValue::integer(0);
    stack[20] = LuaValue::integer(0);
    stack[21] = LuaValue::integer(1);
    stack[22] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::Fallback);
    assert_eq!(result.hits, 0);
    assert_eq!(stack[5].as_integer(), Some(7));
}

#[test]
fn native_numeric_for_loop_entry_executes_and_loop_exits() {
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

    let entry =
        match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::float(2.0)]) {
            BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
                CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => {
                    entry
                }
                other => panic!("expected native numeric forloop execution, got {other:?}"),
            },
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    let mut constants = [LuaValue::nil(); 11];
    constants[10] = LuaValue::integer(2);
    stack[4] = LuaValue::integer(10);
    stack[5] = LuaValue::integer(2);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(100);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            constants.as_ptr(),
            constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(stack[4].as_integer(), Some(16));
    assert_eq!(stack[5].as_integer(), Some(0));
    assert_eq!(stack[7].as_integer(), Some(102));
}

#[test]
fn native_guarded_numeric_for_loop_entry_executes_and_side_exits() {
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::GuardedNumericForLoop {
                entry,
            }) => entry,
            other => panic!("expected native guarded numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 16];
    let mut constants = [LuaValue::nil(); 4];
    constants[3] = LuaValue::integer(3);
    stack[4] = LuaValue::integer(0);
    stack[5] = LuaValue::integer(10);
    stack[6] = LuaValue::integer(10);
    stack[7] = LuaValue::integer(100);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            constants.as_ptr(),
            constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    println!("tail-result={result:?}");
    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 0);
    assert_eq!(result.exit_pc, 105);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(3));
}

#[test]
fn native_guarded_numeric_for_loop_with_float_mul_entry_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 200,
        loop_tail_pc: 203,
        insts: vec![
            TraceIrInst {
                pc: 200,
                opcode: OpCode::MulK,
                raw_instruction: Instruction::create_abc(OpCode::MulK, 4, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 201,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 8, 9, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 202,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(205)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 203,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(200)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 201,
            branch_pc: 202,
            exit_pc: 205,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 200,
        loop_tail_pc: 203,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(9)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(205)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(200)],
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

    let entry =
        match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::float(2.0)]) {
            BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
                CompiledTraceExecution::Native(NativeCompiledTrace::GuardedNumericForLoop {
                    entry,
                }) => entry,
                other => panic!("expected native guarded numeric forloop execution, got {other:?}"),
            },
            BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.5);
    stack[5] = LuaValue::integer(0);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);
    stack[8] = LuaValue::integer(2);
    stack[9] = LuaValue::integer(1);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[4].as_float(), Some(3.0));
}

#[test]
fn native_numeric_jmp_loop_entry_executes_and_side_exits() {
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 16];
    let mut constants = [LuaValue::nil(); 3];
    constants[2] = LuaValue::integer(2);
    stack[4] = LuaValue::integer(0);
    stack[5] = LuaValue::integer(5);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            constants.as_ptr(),
            constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 3);
    assert_eq!(result.exit_pc, 115);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(6));
}

#[test]
fn native_numeric_jmp_loop_with_negative_mod_entry_executes_and_side_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 210,
        loop_tail_pc: 213,
        insts: vec![
            TraceIrInst {
                pc: 210,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 211,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(215)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 212,
                opcode: OpCode::ModK,
                raw_instruction: Instruction::create_abc(OpCode::ModK, 4, 4, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 213,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(210)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 210,
            branch_pc: 211,
            exit_pc: 215,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 210,
        loop_tail_pc: 213,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(215)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(210)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 16];
    let mut constants = [LuaValue::nil(); 3];
    constants[2] = LuaValue::integer(2);
    stack[4] = LuaValue::integer(-5);
    stack[5] = LuaValue::integer(-4);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            constants.as_ptr(),
            constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 1);
    assert_eq!(result.exit_pc, 215);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(1));
}

#[test]
fn native_numeric_jmp_loop_with_negative_idiv_entry_executes_and_side_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 220,
        loop_tail_pc: 223,
        insts: vec![
            TraceIrInst {
                pc: 220,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 221,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(225)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 222,
                opcode: OpCode::IDivK,
                raw_instruction: Instruction::create_abc(OpCode::IDivK, 4, 4, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 223,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(220)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 220,
            branch_pc: 221,
            exit_pc: 225,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 220,
        loop_tail_pc: 223,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(225)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(220)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 16];
    let mut constants = [LuaValue::nil(); 3];
    constants[2] = LuaValue::integer(2);
    stack[4] = LuaValue::integer(-5);
    stack[5] = LuaValue::integer(-4);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            constants.as_ptr(),
            constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 1);
    assert_eq!(result.exit_pc, 225);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(-3));
}

#[test]
fn native_numeric_jmp_loop_with_float_guard_entry_executes_and_side_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 230,
        loop_tail_pc: 233,
        insts: vec![
            TraceIrInst {
                pc: 230,
                opcode: OpCode::Lt,
                raw_instruction: Instruction::create_abck(OpCode::Lt, 4, 5, 0, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Guard,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 231,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, 3).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Branch,
                reads: vec![TraceIrOperand::JumpTarget(235)],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 232,
                opcode: OpCode::LoadF,
                raw_instruction: Instruction::create_asbx(OpCode::LoadF, 4, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::SignedImmediate(4)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 233,
                opcode: OpCode::Jmp,
                raw_instruction: Instruction::create_sj(OpCode::Jmp, -4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(230)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 230,
            branch_pc: 231,
            exit_pc: 235,
            taken_on_trace: false,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    };
    let helper_plan = HelperPlan {
        root_pc: 230,
        loop_tail_pc: 233,
        steps: vec![
            HelperPlanStep::Guard {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(5)],
            },
            HelperPlanStep::Branch {
                reads: vec![TraceIrOperand::JumpTarget(235)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::SignedImmediate(4)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(230)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            other => panic!("expected native numeric jmp execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 16];
    let constants = [LuaValue::nil(); 0];
    stack[4] = LuaValue::float(1.5);
    stack[5] = LuaValue::float(3.0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            constants.as_ptr(),
            constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 1);
    assert_eq!(result.exit_pc, 235);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_float(), Some(4.0));
}

#[test]
fn native_numeric_jmp_loop_with_carried_float_and_entry_stable_rhs_executes_and_side_exits() {
    let mut backend = NativeTraceBackend::default();
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        300,
        303,
        &[(302, 303, 305)],
        &[
            (4, TraceValueKind::Float),
            (6, TraceValueKind::Float),
            (8, TraceValueKind::Integer),
            (9, TraceValueKind::Integer),
            (10, TraceValueKind::Integer),
        ],
    );
    let steps = vec![NumericStep::Binary {
        dst: 4,
        lhs: NumericOperand::Reg(4),
        rhs: NumericOperand::Reg(6),
        op: NumericBinaryOp::Mul,
    }];
    let tail_blocks = vec![NumericJmpLoopGuardBlock {
        pre_steps: vec![NumericStep::Binary {
            dst: 8,
            lhs: NumericOperand::Reg(8),
            rhs: NumericOperand::Reg(10),
            op: NumericBinaryOp::Add,
        }],
        guard: NumericJmpLoopGuard::Tail {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 8,
                rhs: 9,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 305,
        },
    }];

    let entry =
        match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
            Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            Some(other) => panic!("expected native numeric jmp execution, got {other:?}"),
            None => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.5);
    stack[6] = LuaValue::float(2.0);
    stack[8] = LuaValue::integer(0);
    stack[9] = LuaValue::integer(1);
    stack[10] = LuaValue::integer(1);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 1);
    assert_eq!(result.exit_pc, 305);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_float(), Some(3.0));
}

#[test]
fn native_numeric_jmp_loop_tail_guard_reads_carried_float_reg_and_side_exits() {
    let mut backend = NativeTraceBackend::default();
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        320,
        323,
        &[(322, 323, 325)],
        &[
            (4, TraceValueKind::Float),
            (6, TraceValueKind::Float),
            (9, TraceValueKind::Float),
        ],
    );
    let steps = vec![NumericStep::Binary {
        dst: 4,
        lhs: NumericOperand::Reg(4),
        rhs: NumericOperand::Reg(6),
        op: NumericBinaryOp::Mul,
    }];
    let tail_blocks = vec![NumericJmpLoopGuardBlock {
        pre_steps: Vec::new(),
        guard: NumericJmpLoopGuard::Tail {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 4,
                rhs: 9,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 325,
        },
    }];

    let entry =
        match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
            Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            Some(other) => panic!("expected native numeric jmp execution, got {other:?}"),
            None => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.5);
    stack[6] = LuaValue::float(2.0);
    stack[9] = LuaValue::float(2.0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 1);
    assert_eq!(result.exit_pc, 325);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_float(), Some(3.0));
}

#[test]
fn native_numeric_jmp_loop_tail_guard_reads_entry_stable_rhs_and_side_exits() {
    let mut backend = NativeTraceBackend::default();
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        330,
        333,
        &[(332, 333, 335)],
        &[
            (4, TraceValueKind::Float),
            (6, TraceValueKind::Float),
            (9, TraceValueKind::Float),
        ],
    );
    let steps = vec![NumericStep::Binary {
        dst: 4,
        lhs: NumericOperand::Reg(4),
        rhs: NumericOperand::Reg(6),
        op: NumericBinaryOp::Mul,
    }];
    let tail_blocks = vec![NumericJmpLoopGuardBlock {
        pre_steps: Vec::new(),
        guard: NumericJmpLoopGuard::Tail {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 6,
                rhs: 9,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 335,
        },
    }];

    let entry =
        match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
            Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            Some(other) => panic!("expected native numeric jmp execution, got {other:?}"),
            None => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.5);
    stack[6] = LuaValue::float(2.0);
    stack[9] = LuaValue::float(1.0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 1);
    assert_eq!(result.exit_pc, 335);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_float(), Some(3.0));
}

#[test]
fn native_numeric_jmp_loop_with_move_alias_entry_stable_rhs_uses_carried_float_path() {
    let mut backend = NativeTraceBackend::default();
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        340,
        344,
        &[(343, 344, 345)],
        &[
            (4, TraceValueKind::Float),
            (6, TraceValueKind::Float),
            (8, TraceValueKind::Integer),
            (9, TraceValueKind::Integer),
            (10, TraceValueKind::Integer),
        ],
    );
    let steps = vec![
        NumericStep::Move { dst: 11, src: 6 },
        NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::Reg(11),
            op: NumericBinaryOp::Mul,
        },
    ];
    let tail_blocks = vec![NumericJmpLoopGuardBlock {
        pre_steps: vec![NumericStep::Binary {
            dst: 8,
            lhs: NumericOperand::Reg(8),
            rhs: NumericOperand::Reg(10),
            op: NumericBinaryOp::Add,
        }],
        guard: NumericJmpLoopGuard::Tail {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 8,
                rhs: 9,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 345,
        },
    }];

    let entry =
        match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
            Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            Some(other) => panic!("expected native numeric jmp execution, got {other:?}"),
            None => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.5);
    stack[6] = LuaValue::float(2.0);
    stack[8] = LuaValue::integer(0);
    stack[9] = LuaValue::integer(1);
    stack[10] = LuaValue::integer(1);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 1);
    assert_eq!(result.exit_pc, 345);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_float(), Some(3.0));
    assert!(stack[11].is_nil());
}

#[test]
fn native_numeric_jmp_loop_guard_prestep_reads_carried_float_reg() {
    let mut backend = NativeTraceBackend::default();
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        350,
        353,
        &[(352, 353, 355)],
        &[
            (4, TraceValueKind::Float),
            (6, TraceValueKind::Float),
            (9, TraceValueKind::Float),
        ],
    );
    let steps = vec![NumericStep::Binary {
        dst: 4,
        lhs: NumericOperand::Reg(4),
        rhs: NumericOperand::Reg(6),
        op: NumericBinaryOp::Mul,
    }];
    let tail_blocks = vec![NumericJmpLoopGuardBlock {
        pre_steps: vec![NumericStep::Move { dst: 10, src: 4 }],
        guard: NumericJmpLoopGuard::Tail {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 10,
                rhs: 9,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 355,
        },
    }];

    let entry =
        match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
            Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
            Some(other) => panic!("expected native numeric jmp execution, got {other:?}"),
            None => panic!("expected compiled trace"),
        };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.5);
    stack[6] = LuaValue::float(2.0);
    stack[9] = LuaValue::float(2.0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.hits, 1);
    assert_eq!(result.exit_pc, 355);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_float(), Some(3.0));
    assert_eq!(stack[10].as_float(), Some(3.0));
}

#[test]
fn native_return0_entry_reports_returned() {
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::Return0 { entry }) => entry,
            other => panic!("expected native return0 execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 1];
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::Returned);
    assert_eq!(result.start_reg, 0);
    assert_eq!(result.result_count, 0);
}

#[test]
fn native_return1_entry_reports_returned() {
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::Return1 { entry }) => entry,
            other => panic!("expected native return1 execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 8];
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::Returned);
    assert_eq!(result.start_reg, 4);
    assert_eq!(result.result_count, 1);
}

#[test]
fn native_return_entry_reports_returned() {
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::Return { entry }) => entry,
            other => panic!("expected native return execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 8];
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::Returned);
    assert_eq!(result.start_reg, 5);
    assert_eq!(result.result_count, 3);
}
