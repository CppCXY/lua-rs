use super::*;
use super::shared::*;

#[test]
fn native_backend_recognizes_generic_tfor_loop() {
    let mut backend = NativeTraceBackend::default();
    let ir = tfor_loop_test_ir();
    let helper_plan = tfor_loop_test_helper_plan();

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::TForLoop { .. }) => {}
            other => panic!("expected native tfor execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_tfor_loop_with_c_iterator_executes_and_side_exits_on_nil() {
    let mut backend = NativeTraceBackend::default();
    let ir = tfor_loop_test_ir();
    let helper_plan = tfor_loop_test_helper_plan();

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::TForLoop { entry }) => entry,
            other => panic!("expected native tfor execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    let state = vm.main_state();
    state.push_c_frame(0, 12, -1).unwrap();
    {
        let stack = state.stack_mut();
        stack[0] = LuaValue::cfunction(test_native_tfor_iterator);
        stack[1] = LuaValue::integer(3);
        stack[3] = LuaValue::integer(1);
        stack[4] = LuaValue::integer(10);
        stack[8] = LuaValue::integer(0);
    }

    let lua_state_ptr: *mut crate::lua_vm::LuaState = state;
    let stack_ptr = state.stack_mut().as_mut_ptr();
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack_ptr,
            0,
            std::ptr::null(),
            0,
            lua_state_ptr,
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::SideExit);
    assert_eq!(result.exit_pc, 3);
    assert_eq!(result.hits, 2);
    assert_eq!(
        state.stack_get(8).and_then(|value| value.as_integer()),
        Some(60)
    );
    assert!(state.stack_get(3).is_some_and(|value| value.is_nil()));
    assert_eq!(state.current_frame().map(|ci| ci.pc), Some(1));
}

#[test]
fn native_tfor_loop_with_lua_iterator_returns_to_interpreter() {
    let mut backend = NativeTraceBackend::default();
    let ir = tfor_loop_test_ir();
    let helper_plan = tfor_loop_test_helper_plan();

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::TForLoop { entry }) => entry,
            other => panic!("expected native tfor execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute(
        "function native_tfor_lua_iter(limit, control) local next = control + 1 if next <= limit then return next, next * 10 end end",
    )
    .unwrap();
    let iter = vm.get_global("native_tfor_lua_iter").unwrap().unwrap();
    let state = vm.main_state();
    state.push_c_frame(0, 12, -1).unwrap();
    {
        let stack = state.stack_mut();
        stack[0] = iter;
        stack[1] = LuaValue::integer(3);
        stack[3] = LuaValue::integer(1);
        stack[4] = LuaValue::integer(10);
        stack[8] = LuaValue::integer(0);
    }

    let lua_state_ptr: *mut crate::lua_vm::LuaState = state;
    let stack_ptr = state.stack_mut().as_mut_ptr();
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack_ptr,
            0,
            std::ptr::null(),
            0,
            lua_state_ptr,
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::Returned);
    assert_eq!(result.hits, 0);
    assert_eq!(
        state.stack_get(8).and_then(|value| value.as_integer()),
        Some(10)
    );
    assert_eq!(state.call_depth(), 2);
    assert_eq!(state.get_frame(0).map(|ci| ci.pc), Some(1));
    assert!(state.current_frame().is_some_and(|ci| ci.is_lua()));
    assert_eq!(
        state.get_arg(1).and_then(|value| value.as_integer()),
        Some(3)
    );
    assert_eq!(
        state.get_arg(2).and_then(|value| value.as_integer()),
        Some(1)
    );
}

#[test]
fn native_backend_compiles_call_for_loop_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = call_for_loop_test_ir();
    let helper_plan = call_for_loop_test_helper_plan();

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::CallForLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_backend_compiles_call_for_loop_with_post_call_move_to_native_execution() {
    let mut backend = NativeTraceBackend::default();
    let ir = call_for_loop_with_post_move_test_ir();
    let helper_plan = call_for_loop_with_post_move_test_helper_plan();

    match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => {
            assert!(matches!(
                compiled.execution(),
                CompiledTraceExecution::Native(NativeCompiledTrace::CallForLoop { .. })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}

#[test]
fn native_call_for_loop_with_c_callable_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = call_for_loop_test_ir();
    let helper_plan = call_for_loop_test_helper_plan();

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::CallForLoop { entry }) => entry,
            other => panic!("expected native call-for execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    let state = vm.main_state();
    state.push_c_frame(0, 16, -1).unwrap();
    {
        let stack = state.stack_mut();
        stack[4] = LuaValue::cfunction(test_native_call_increment);
        stack[8] = LuaValue::integer(2);
        stack[9] = LuaValue::integer(1);
        stack[10] = LuaValue::integer(1);
    }

    let lua_state_ptr: *mut crate::lua_vm::LuaState = state;
    let stack_ptr = state.stack_mut().as_mut_ptr();
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack_ptr,
            0,
            std::ptr::null(),
            0,
            lua_state_ptr,
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(
        state.stack_get(0).and_then(|value| value.as_integer()),
        Some(8)
    );
}

#[test]
fn native_call_for_loop_with_table_call_metamethod_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = call_for_loop_test_ir();
    let helper_plan = call_for_loop_test_helper_plan();

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::CallForLoop { entry }) => entry,
            other => panic!("expected native call-for execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    vm.execute(
        r#"
        native_call_target = setmetatable({}, {
            __call = function(self, x)
                return x + 5
            end
        })
        "#,
    )
    .unwrap();
    let callable = vm.get_global("native_call_target").unwrap().unwrap();
    let state = vm.main_state();
    state.push_c_frame(0, 16, -1).unwrap();
    {
        let stack = state.stack_mut();
        stack[4] = callable;
        stack[8] = LuaValue::integer(2);
        stack[9] = LuaValue::integer(1);
        stack[10] = LuaValue::integer(1);
    }

    let lua_state_ptr: *mut crate::lua_vm::LuaState = state;
    let stack_ptr = state.stack_mut().as_mut_ptr();
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack_ptr,
            0,
            std::ptr::null(),
            0,
            lua_state_ptr,
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(
        state.stack_get(0).and_then(|value| value.as_integer()),
        Some(8)
    );
}

#[test]
fn native_call_for_loop_with_post_call_move_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = call_for_loop_with_post_move_test_ir();
    let helper_plan = call_for_loop_with_post_move_test_helper_plan();

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::CallForLoop { entry }) => entry,
            other => panic!("expected native call-for execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    let state = vm.main_state();
    state.push_c_frame(0, 16, -1).unwrap();
    {
        let stack = state.stack_mut();
        stack[1] = LuaValue::integer(0);
        stack[4] = LuaValue::cfunction(test_native_call_increment);
        stack[8] = LuaValue::integer(2);
        stack[9] = LuaValue::integer(1);
        stack[10] = LuaValue::integer(1);
    }

    let lua_state_ptr: *mut crate::lua_vm::LuaState = state;
    let stack_ptr = state.stack_mut().as_mut_ptr();
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack_ptr,
            0,
            std::ptr::null(),
            0,
            lua_state_ptr,
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(
        state.stack_get(0).and_then(|value| value.as_integer()),
        Some(11)
    );
    assert_eq!(
        state.stack_get(2).and_then(|value| value.as_integer()),
        Some(11)
    );
}

#[test]
fn native_numeric_for_loop_with_integer_gettable_function_index_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 3,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 4, 3, 8, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::Register(8)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::Add,
                raw_instruction: Instruction::create_abck(OpCode::Add, 2, 2, 4, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(2)],
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::MmBin,
                raw_instruction: Instruction::create_abck(OpCode::MmBin, 2, 4, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 9, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 3,
        steps: vec![
            HelperPlanStep::TableAccess {
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::Register(8)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(2)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(0)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
            other => panic!("expected native numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    vm.execute(
        r#"
        native_index_target = setmetatable({}, {
            __index = function(_, k)
                return k * 2
            end
        })
        "#,
    )
    .unwrap();
    let table = vm.get_global("native_index_target").unwrap().unwrap();
    let state = vm.main_state();
    state.push_c_frame(0, 16, -1).unwrap();
    {
        let stack = state.stack_mut();
        stack[2] = LuaValue::integer(0);
        stack[3] = table;
        stack[8] = LuaValue::integer(3);
        stack[9] = LuaValue::integer(0);
        stack[10] = LuaValue::integer(1);
        stack[11] = LuaValue::integer(0);
    }

    let lua_state_ptr: *mut crate::lua_vm::LuaState = state;
    let stack_ptr = state.stack_mut().as_mut_ptr();
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack_ptr,
            0,
            std::ptr::null(),
            0,
            lua_state_ptr,
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(
        state.stack_get(2).and_then(|value| value.as_integer()),
        Some(6)
    );
}

#[test]
fn native_numeric_for_loop_with_len_metamethod_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 0,
        loop_tail_pc: 3,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::Len,
                raw_instruction: Instruction::create_abc(OpCode::Len, 4, 3, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(3)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::Add,
                raw_instruction: Instruction::create_abck(OpCode::Add, 2, 2, 4, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(2)],
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::MmBin,
                raw_instruction: Instruction::create_abck(OpCode::MmBin, 2, 4, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 9, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 0,
        loop_tail_pc: 3,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(3)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(2)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(0)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
            other => panic!("expected native numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    vm.execute(
        r#"
        native_len_target = setmetatable({}, {
            __len = function(_)
                return 7
            end
        })
        "#,
    )
    .unwrap();
    let table = vm.get_global("native_len_target").unwrap().unwrap();
    let state = vm.main_state();
    state.push_c_frame(0, 16, -1).unwrap();
    {
        let stack = state.stack_mut();
        stack[2] = LuaValue::integer(0);
        stack[3] = table;
        stack[9] = LuaValue::integer(0);
        stack[10] = LuaValue::integer(1);
        stack[11] = LuaValue::integer(0);
    }

    let lua_state_ptr: *mut crate::lua_vm::LuaState = state;
    let stack_ptr = state.stack_mut().as_mut_ptr();
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack_ptr,
            0,
            std::ptr::null(),
            0,
            lua_state_ptr,
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(
        state.stack_get(2).and_then(|value| value.as_integer()),
        Some(7)
    );
}

#[test]
fn native_numeric_for_loop_with_mixed_arithmetic_entry_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 116,
        loop_tail_pc: 122,
        insts: vec![
            TraceIrInst {
                pc: 116,
                opcode: OpCode::AddK,
                raw_instruction: Instruction::create_abc(OpCode::AddK, 4, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 117,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 118,
                opcode: OpCode::MulK,
                raw_instruction: Instruction::create_abc(OpCode::MulK, 4, 4, 1).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 119,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 1, 8, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 120,
                opcode: OpCode::SubK,
                raw_instruction: Instruction::create_abc(OpCode::SubK, 4, 4, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 121,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 2, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 122,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 7).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(116)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 116,
        loop_tail_pc: 122,
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
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(2),
                ],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(116)],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
            other => panic!("expected native numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 8];
    let constants = [
        LuaValue::float(0.5),
        LuaValue::float(2.0),
        LuaValue::float(0.5),
    ];
    stack[4] = LuaValue::integer(1);
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

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[4].as_float(), Some(2.5));
}

#[test]
fn native_numeric_for_loop_with_float_mul_entry_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 140,
        loop_tail_pc: 142,
        insts: vec![
            TraceIrInst {
                pc: 140,
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
                pc: 141,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 8, false).as_u32(),
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
    let constants = [LuaValue::float(2.0)];
    stack[4] = LuaValue::float(1.5);
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

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[4].as_float(), Some(3.0));
}

#[test]
fn native_numeric_for_loop_with_add_overflow_uses_helper_fallback_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 123,
        loop_tail_pc: 125,
        insts: vec![
            TraceIrInst {
                pc: 123,
                opcode: OpCode::AddK,
                raw_instruction: Instruction::create_abc(OpCode::AddK, 4, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 124,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 125,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(123)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 123,
        loop_tail_pc: 125,
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
                reads: vec![TraceIrOperand::JumpTarget(123)],
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
    let constants = [LuaValue::integer(1)];
    stack[4] = LuaValue::integer(i64::MAX);
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

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[4].as_integer(), Some(i64::MIN));
}

#[test]
fn native_numeric_for_loop_with_div_pow_entry_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 120,
        loop_tail_pc: 124,
        insts: vec![
            TraceIrInst {
                pc: 120,
                opcode: OpCode::DivK,
                raw_instruction: Instruction::create_abc(OpCode::DivK, 4, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 121,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 5, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(0),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 122,
                opcode: OpCode::PowK,
                raw_instruction: Instruction::create_abc(OpCode::PowK, 4, 4, 1).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 123,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 1, 10, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![],
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
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
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
            metamethod_steps: 2,
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
    let constants = [LuaValue::float(2.0), LuaValue::float(2.0)];
    stack[4] = LuaValue::integer(8);
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

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[4].as_float(), Some(16.0));
}

#[test]
fn native_numeric_for_loop_with_idiv_mod_entry_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 130,
        loop_tail_pc: 134,
        insts: vec![
            TraceIrInst {
                pc: 130,
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
                pc: 131,
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
                pc: 132,
                opcode: OpCode::ModK,
                raw_instruction: Instruction::create_abc(OpCode::ModK, 4, 4, 1).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 133,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 1, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 134,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(130)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 130,
        loop_tail_pc: 134,
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
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::ConstantIndex(1),
                ],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(130)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 0,
            call_steps: 0,
            metamethod_steps: 2,
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
    let constants = [LuaValue::integer(2), LuaValue::integer(2)];
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

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[4].as_integer(), Some(1));
}

#[test]
fn native_numeric_for_loop_with_upvalue_steps_entry_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 6,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![],
            },
            TraceIrInst {
                pc: 7,
                opcode: OpCode::SetUpval,
                raw_instruction: Instruction::create_abck(OpCode::SetUpval, 4, 0, 0, false)
                    .as_u32(),
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::SignedImmediate(1),
                ],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
            other => panic!("expected native numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let upvalue = boxed_closed_upvalue(LuaValue::integer(40));
    let upvalue_ptrs = [UpvaluePtr::new(upvalue.as_ref() as *const GcUpvalue)];
    let mut stack = [LuaValue::nil(); 8];
    stack[1] = LuaValue::integer(2);
    stack[2] = LuaValue::integer(1);
    stack[3] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            upvalue_ptrs.as_ptr(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(stack[4].as_integer(), Some(43));
    assert_eq!(upvalue.data.get_value_ref().as_integer(), Some(43));
}

#[test]
fn native_numeric_for_loop_with_table_steps_entry_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 40,
        loop_tail_pc: 42,
        insts: vec![
            TraceIrInst {
                pc: 40,
                opcode: OpCode::GetTable,
                raw_instruction: Instruction::create_abck(OpCode::GetTable, 8, 2, 7, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(2), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 41,
                opcode: OpCode::SetTable,
                raw_instruction: Instruction::create_abck(OpCode::SetTable, 3, 7, 8, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(8),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(8),
                ],
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

    let entry = match backend.compile_test(&ir, &helper_plan) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
            other => panic!("expected native numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    let src_table = vm.create_table(4, 0).unwrap();
    let dst_table = vm.create_table(4, 0).unwrap();
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

    let mut stack = [LuaValue::nil(); 10];
    stack[2] = src_table;
    stack[3] = dst_table;
    stack[5] = LuaValue::integer(2);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(1);

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

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(stack[8].as_integer(), Some(33));
    let dst = dst_table.as_table().unwrap();
    assert_eq!(dst.raw_geti(1), Some(LuaValue::integer(11)));
    assert_eq!(dst.raw_geti(2), Some(LuaValue::integer(22)));
    assert_eq!(dst.raw_geti(3), Some(LuaValue::integer(33)));
}

#[test]
fn native_numeric_for_loop_with_loadf_entry_executes_and_loop_exits() {
    let mut backend = NativeTraceBackend::default();
    let ir = TraceIr {
        root_pc: 46,
        loop_tail_pc: 47,
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
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 5, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(46)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan {
        root_pc: 46,
        loop_tail_pc: 47,
        steps: vec![
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(46)],
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
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
            other => panic!("expected native numeric forloop execution, got {other:?}"),
        },
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    };

    let mut stack = [LuaValue::nil(); 8];
    stack[5] = LuaValue::integer(0);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);

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
    assert_eq!(stack[4].as_float(), Some(1.0));
}
