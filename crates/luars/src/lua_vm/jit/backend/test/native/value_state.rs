use super::*;
use super::shared::*;

#[test]
fn compile_native_numeric_for_loop_uses_integer_self_update_value_flow() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(2),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(2),
            }),
        },
    };
    let lowered_trace = lowered_trace_with_entry_hints(&[(4, TraceValueKind::Integer)]);

    let entry = match backend
        .compile_native_numeric_for_loop(5, &lowering, &lowered_trace)
        .expect("expected native numeric forloop")
    {
        NativeCompiledTrace::NumericForLoop { entry } => entry,
        other => panic!("expected native numeric forloop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 8];
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(2);
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
    assert_eq!(result.hits, 3);
    assert_eq!(stack[4].as_integer(), Some(7));
}

#[test]
fn compile_native_numeric_for_loop_materializes_carried_integer_only_on_fallback_boundary() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_with_entry_hints(&[(4, TraceValueKind::Integer)]);

    let entry = match backend
        .compile_native_numeric_for_loop(5, &lowering, &lowered_trace)
        .expect("expected native numeric forloop")
    {
        NativeCompiledTrace::NumericForLoop { entry } => entry,
        other => panic!("expected native numeric forloop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 8];
    stack[4] = LuaValue::integer(i64::MAX - 1);
    stack[5] = LuaValue::integer(2);
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

    assert_eq!(result.status, NativeTraceStatus::Fallback);
    assert_eq!(result.hits, 1);
    assert_eq!(stack[4].as_integer(), Some(i64::MAX));
}

#[test]
fn compile_native_numeric_jmp_loop_uses_integer_self_update_value_flow() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        400,
        402,
        &[(401, 402, 405)],
        &[
            (4, TraceValueKind::Integer),
            (8, TraceValueKind::Integer),
            (9, TraceValueKind::Integer),
            (10, TraceValueKind::Integer),
        ],
    );
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
            exit_pc: 405,
        },
    }];

    let entry = match backend
        .compile_native_numeric_jmp_loop(&[], &lowering, &tail_blocks, 0, &lowered_trace)
        .expect("expected native numeric jmp loop")
    {
        NativeCompiledTrace::NumericJmpLoop { entry } => entry,
        other => panic!("expected native numeric jmp loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
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
    assert_eq!(result.exit_pc, 405);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(2));
}

#[test]
fn compile_native_numeric_jmp_loop_materializes_carried_integer_only_on_fallback_boundary() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        410,
        411,
        &[(410, 411, 415)],
        &[(4, TraceValueKind::Integer)],
    );

    let entry = match backend
        .compile_native_numeric_jmp_loop(
            &[],
            &lowering,
            &[NumericJmpLoopGuardBlock {
                pre_steps: Vec::new(),
                guard: NumericJmpLoopGuard::Tail {
                    cond: NumericIfElseCond::RegCompare {
                        op: LinearIntGuardOp::Lt,
                        lhs: 12,
                        rhs: 13,
                    },
                    continue_when: true,
                    continue_preset: None,
                    exit_preset: None,
                    exit_pc: 415,
                },
            }],
            0,
            &lowered_trace,
        )
        .expect("expected native numeric jmp loop")
    {
        NativeCompiledTrace::NumericJmpLoop { entry } => entry,
        other => panic!("expected native numeric jmp loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(i64::MAX);
    stack[12] = LuaValue::integer(0);
    stack[13] = LuaValue::integer(1);

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
    assert_eq!(stack[4].as_integer(), Some(i64::MAX));
}

#[test]
fn compile_native_numeric_for_loop_does_not_hoist_overwritten_stable_rhs() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(10),
                op: NumericBinaryOp::Add,
            },
            NumericStep::LoadI { dst: 10, imm: 1 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::StableReg {
                    reg: 10,
                    kind: TraceValueKind::Integer,
                },
            }),
        },
    };
    let lowered_trace = lowered_trace_with_entry_hints(&[
        (4, TraceValueKind::Integer),
        (10, TraceValueKind::Integer),
    ]);

    let entry = match backend
        .compile_native_numeric_for_loop(5, &lowering, &lowered_trace)
        .expect("expected native numeric forloop")
    {
        NativeCompiledTrace::NumericForLoop { entry } => entry,
        other => panic!("expected native numeric forloop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(2);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);
    stack[10] = LuaValue::integer(2);

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
    assert_eq!(result.hits, 3);
    assert_eq!(stack[4].as_integer(), Some(5));
    assert_eq!(stack[10].as_integer(), Some(1));
}

#[test]
fn compile_native_numeric_for_loop_with_short_string_field_steps() {
    let mut vm = LuaVM::new(SafeOption::default());
    let key = vm.create_string("value").unwrap();
    let table = vm.create_table(0, 1).unwrap();
    assert!(vm.raw_set(&table, key, LuaValue::integer(1)));

    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::GetTableField {
                dst: 4,
                table: 3,
                key: 0,
            },
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::ImmI(1),
                op: NumericBinaryOp::Add,
            },
            NumericStep::SetTableField {
                table: 3,
                key: 0,
                value: 4,
            },
        ],
        value_state: NumericValueState::default(),
    };
    let lowered_trace =
        lowered_trace_with_entry_hints_and_constants(&[(3, TraceValueKind::Table)], vec![key]);

    let entry = match backend
        .compile_native_numeric_for_loop(5, &lowering, &lowered_trace)
        .expect("expected native numeric forloop")
    {
        NativeCompiledTrace::NumericForLoop { entry } => entry,
        other => panic!("expected native numeric forloop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 8];
    stack[3] = table;
    stack[5] = LuaValue::integer(2);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            lowered_trace.constants.as_ptr(),
            lowered_trace.constants.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(stack[4].as_integer(), Some(4));
    assert_eq!(
        vm.raw_get(&stack[3], &key)
            .and_then(|value| value.as_integer()),
        Some(4)
    );
}

#[test]
fn compile_native_numeric_for_loop_with_short_string_tabup_field_steps() {
    let mut vm = LuaVM::new(SafeOption::default());
    let key = vm.create_string("value").unwrap();
    let table = vm.create_table(0, 1).unwrap();
    assert!(vm.raw_set(&table, key, LuaValue::integer(1)));
    let upvalue = vm.create_upvalue_closed(table).unwrap();

    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::GetTabUpField {
                dst: 4,
                upvalue: 0,
                key: 0,
            },
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::ImmI(1),
                op: NumericBinaryOp::Add,
            },
            NumericStep::SetTabUpField {
                upvalue: 0,
                key: 0,
                value: 4,
            },
        ],
        value_state: NumericValueState::default(),
    };
    let lowered_trace = lowered_trace_with_entry_hints_and_constants(&[], vec![key]);

    let entry = match backend
        .compile_native_numeric_for_loop(5, &lowering, &lowered_trace)
        .expect("expected native numeric forloop")
    {
        NativeCompiledTrace::NumericForLoop { entry } => entry,
        other => panic!("expected native numeric forloop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 8];
    stack[5] = LuaValue::integer(2);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);

    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack.as_mut_ptr(),
            0,
            lowered_trace.constants.as_ptr(),
            lowered_trace.constants.len(),
            std::ptr::null_mut(),
            std::slice::from_ref(&upvalue).as_ptr(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(stack[4].as_integer(), Some(4));
    assert_eq!(
        vm.raw_get(&table, &key)
            .and_then(|value| value.as_integer()),
        Some(4)
    );
}

#[test]
fn compile_native_numeric_for_loop_with_tabup_field_function_index_steps() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    vm.execute(
        r#"
            native_tabup_index_target = setmetatable({}, {
                __index = function(_, k)
                    if k == "value" then
                        return 2
                    end
                    return 0
                end
            })
            "#,
    )
    .unwrap();
    let key = vm.create_string("value").unwrap();
    let table = vm.get_global("native_tabup_index_target").unwrap().unwrap();
    let upvalue = vm.create_upvalue_closed(table).unwrap();

    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::GetTabUpField {
                dst: 4,
                upvalue: 0,
                key: 0,
            },
            NumericStep::Binary {
                dst: 2,
                lhs: NumericOperand::Reg(2),
                rhs: NumericOperand::Reg(4),
                op: NumericBinaryOp::Add,
            },
        ],
        value_state: NumericValueState::default(),
    };
    let lowered_trace = lowered_trace_with_entry_hints_and_constants(&[], vec![key]);

    let entry = match backend
        .compile_native_numeric_for_loop(5, &lowering, &lowered_trace)
        .expect("expected native numeric forloop")
    {
        NativeCompiledTrace::NumericForLoop { entry } => entry,
        other => panic!("expected native numeric forloop, got {other:?}"),
    };

    let state = vm.main_state();
    state.push_c_frame(0, 12, -1).unwrap();
    {
        let stack = state.stack_mut();
        stack[2] = LuaValue::integer(0);
        stack[5] = LuaValue::integer(2);
        stack[6] = LuaValue::integer(1);
        stack[7] = LuaValue::integer(0);
    }

    let lua_state_ptr: *mut crate::lua_vm::LuaState = state;
    let stack_ptr = state.stack_mut().as_mut_ptr();
    let mut result = NativeTraceResult::default();
    unsafe {
        entry(
            stack_ptr,
            0,
            lowered_trace.constants.as_ptr(),
            lowered_trace.constants.len(),
            lua_state_ptr,
            std::slice::from_ref(&upvalue).as_ptr(),
            &mut result,
        );
    }

    assert_eq!(result.status, NativeTraceStatus::LoopExit);
    assert_eq!(result.hits, 3);
    assert_eq!(
        state.stack_get(2).and_then(|value| value.as_integer()),
        Some(6)
    );
}

#[test]
fn compile_native_numeric_for_loop_keeps_integer_self_update_with_residual_move() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::ImmI(2),
                op: NumericBinaryOp::Add,
            },
            NumericStep::Move { dst: 11, src: 4 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(2),
            }),
        },
    };
    let lowered_trace = lowered_trace_with_entry_hints(&[(4, TraceValueKind::Integer)]);

    let entry = match backend
        .compile_native_numeric_for_loop(5, &lowering, &lowered_trace)
        .expect("expected native numeric forloop")
    {
        NativeCompiledTrace::NumericForLoop { entry } => entry,
        other => panic!("expected native numeric forloop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(2);
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
    assert_eq!(result.hits, 3);
    assert_eq!(stack[4].as_integer(), Some(7));
    assert_eq!(stack[11].as_integer(), Some(7));
}

#[test]
fn compile_native_numeric_for_loop_keeps_float_self_update_with_residual_move() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(8),
                op: NumericBinaryOp::Mul,
            },
            NumericStep::Move { dst: 11, src: 4 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Mul,
                kind: NumericSelfUpdateValueKind::Float,
                rhs: NumericValueFlowRhs::StableReg {
                    reg: 8,
                    kind: TraceValueKind::Integer,
                },
            }),
        },
    };
    let lowered_trace =
        lowered_trace_with_entry_hints(&[(4, TraceValueKind::Float), (8, TraceValueKind::Integer)]);

    let entry = match backend
        .compile_native_numeric_for_loop(5, &lowering, &lowered_trace)
        .expect("expected native numeric forloop")
    {
        NativeCompiledTrace::NumericForLoop { entry } => entry,
        other => panic!("expected native numeric forloop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.5);
    stack[5] = LuaValue::integer(2);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);
    stack[8] = LuaValue::integer(2);

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
    assert_eq!(result.hits, 3);
    assert_eq!(stack[4].as_float(), Some(12.0));
    assert_eq!(stack[11].as_float(), Some(12.0));
}

#[test]
fn compile_native_numeric_jmp_loop_tail_guard_reads_carried_integer_reg() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        416,
        418,
        &[(417, 418, 419)],
        &[(4, TraceValueKind::Integer), (9, TraceValueKind::Integer)],
    );
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
            exit_pc: 419,
        },
    }];

    let entry = match backend
        .compile_native_numeric_jmp_loop(&[], &lowering, &tail_blocks, 0, &lowered_trace)
        .expect("expected native numeric jmp loop")
    {
        NativeCompiledTrace::NumericJmpLoop { entry } => entry,
        other => panic!("expected native numeric jmp loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[9] = LuaValue::integer(3);

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
    assert_eq!(result.hits, 2);
    assert_eq!(result.exit_pc, 419);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(3));
}

#[test]
fn compile_native_numeric_jmp_loop_guard_prestep_reads_carried_integer_reg() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        420,
        422,
        &[(421, 422, 423)],
        &[(4, TraceValueKind::Integer), (9, TraceValueKind::Integer)],
    );
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
            exit_pc: 423,
        },
    }];

    let entry = match backend
        .compile_native_numeric_jmp_loop(&[], &lowering, &tail_blocks, 0, &lowered_trace)
        .expect("expected native numeric jmp loop")
    {
        NativeCompiledTrace::NumericJmpLoop { entry } => entry,
        other => panic!("expected native numeric jmp loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[9] = LuaValue::integer(3);

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
    assert_eq!(result.hits, 2);
    assert_eq!(result.exit_pc, 423);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(3));
    assert_eq!(stack[10].as_integer(), Some(3));
}

#[test]
fn compile_native_numeric_jmp_loop_continue_preset_reads_carried_integer_and_stable_rhs() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::Reg(6),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::StableReg {
                    reg: 6,
                    kind: TraceValueKind::Integer,
                },
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        424,
        426,
        &[(425, 426, 427)],
        &[
            (4, TraceValueKind::Integer),
            (6, TraceValueKind::Integer),
            (9, TraceValueKind::Integer),
        ],
    );
    let tail_blocks = vec![NumericJmpLoopGuardBlock {
        pre_steps: Vec::new(),
        guard: NumericJmpLoopGuard::Tail {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 4,
                rhs: 9,
            },
            continue_when: true,
            continue_preset: Some(NumericStep::Binary {
                dst: 10,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(6),
                op: NumericBinaryOp::Add,
            }),
            exit_preset: Some(NumericStep::Move { dst: 11, src: 4 }),
            exit_pc: 427,
        },
    }];

    let entry = match backend
        .compile_native_numeric_jmp_loop(&[], &lowering, &tail_blocks, 0, &lowered_trace)
        .expect("expected native numeric jmp loop")
    {
        NativeCompiledTrace::NumericJmpLoop { entry } => entry,
        other => panic!("expected native numeric jmp loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[6] = LuaValue::integer(2);
    stack[9] = LuaValue::integer(5);

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
    assert_eq!(result.hits, 2);
    assert_eq!(result.exit_pc, 427);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(5));
    assert_eq!(stack[10].as_integer(), Some(5));
    assert_eq!(stack[11].as_integer(), Some(5));
}

#[test]
fn compile_native_numeric_jmp_loop_does_not_hoist_overwritten_stable_rhs() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(10),
                op: NumericBinaryOp::Add,
            },
            NumericStep::LoadI { dst: 10, imm: 1 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::StableReg {
                    reg: 10,
                    kind: TraceValueKind::Integer,
                },
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        428,
        430,
        &[(429, 430, 431)],
        &[
            (4, TraceValueKind::Integer),
            (9, TraceValueKind::Integer),
            (10, TraceValueKind::Integer),
        ],
    );
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
            exit_pc: 431,
        },
    }];

    let entry = match backend
        .compile_native_numeric_jmp_loop(&[], &lowering, &tail_blocks, 0, &lowered_trace)
        .expect("expected native numeric jmp loop")
    {
        NativeCompiledTrace::NumericJmpLoop { entry } => entry,
        other => panic!("expected native numeric jmp loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[9] = LuaValue::integer(5);
    stack[10] = LuaValue::integer(2);

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
    assert_eq!(result.hits, 3);
    assert_eq!(result.exit_pc, 431);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(5));
    assert_eq!(stack[10].as_integer(), Some(1));
}

#[test]
fn compile_native_numeric_jmp_loop_keeps_integer_self_update_with_residual_move() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::ImmI(1),
                op: NumericBinaryOp::Add,
            },
            NumericStep::Move { dst: 11, src: 4 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        432,
        434,
        &[(433, 434, 435)],
        &[(4, TraceValueKind::Integer), (9, TraceValueKind::Integer)],
    );
    let tail_blocks = vec![NumericJmpLoopGuardBlock {
        pre_steps: Vec::new(),
        guard: NumericJmpLoopGuard::Tail {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 11,
                rhs: 9,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 435,
        },
    }];

    let entry = match backend
        .compile_native_numeric_jmp_loop(&[], &lowering, &tail_blocks, 0, &lowered_trace)
        .expect("expected native numeric jmp loop")
    {
        NativeCompiledTrace::NumericJmpLoop { entry } => entry,
        other => panic!("expected native numeric jmp loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[9] = LuaValue::integer(3);

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
    assert_eq!(result.hits, 2);
    assert_eq!(result.exit_pc, 435);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(3));
    assert_eq!(stack[11].as_integer(), Some(3));
}

#[test]
fn compile_native_numeric_jmp_loop_keeps_float_self_update_with_residual_move() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(8),
                op: NumericBinaryOp::Mul,
            },
            NumericStep::Move { dst: 11, src: 4 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Mul,
                kind: NumericSelfUpdateValueKind::Float,
                rhs: NumericValueFlowRhs::StableReg {
                    reg: 8,
                    kind: TraceValueKind::Integer,
                },
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        444,
        446,
        &[(445, 446, 447)],
        &[
            (4, TraceValueKind::Float),
            (8, TraceValueKind::Integer),
            (9, TraceValueKind::Integer),
        ],
    );
    let tail_blocks = vec![NumericJmpLoopGuardBlock {
        pre_steps: Vec::new(),
        guard: NumericJmpLoopGuard::Tail {
            cond: NumericIfElseCond::RegCompare {
                op: LinearIntGuardOp::Lt,
                lhs: 11,
                rhs: 9,
            },
            continue_when: true,
            continue_preset: None,
            exit_preset: None,
            exit_pc: 447,
        },
    }];

    let entry = match backend
        .compile_native_numeric_jmp_loop(&[], &lowering, &tail_blocks, 0, &lowered_trace)
        .expect("expected native numeric jmp loop")
    {
        NativeCompiledTrace::NumericJmpLoop { entry } => entry,
        other => panic!("expected native numeric jmp loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.0);
    stack[8] = LuaValue::integer(2);
    stack[9] = LuaValue::integer(5);

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
    assert_eq!(result.hits, 3);
    assert_eq!(result.exit_pc, 447);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_float(), Some(8.0));
    assert_eq!(stack[11].as_float(), Some(8.0));
}

#[test]
fn compile_native_guarded_numeric_for_loop_uses_integer_self_update_value_flow() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        420,
        422,
        &[(421, 422, 425)],
        &[
            (4, TraceValueKind::Integer),
            (5, TraceValueKind::Integer),
            (6, TraceValueKind::Integer),
            (8, TraceValueKind::Integer),
        ],
    );

    let entry = match backend
        .compile_native_guarded_numeric_for_loop(
            5,
            &lowering,
            NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::RegCompare {
                    op: LinearIntGuardOp::Lt,
                    lhs: 4,
                    rhs: 8,
                },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 425,
            },
            &lowered_trace,
        )
        .expect("expected native guarded numeric for loop")
    {
        NativeCompiledTrace::GuardedNumericForLoop { entry } => entry,
        other => panic!("expected native guarded numeric for loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(2);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);
    stack[8] = LuaValue::integer(3);

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
    assert_eq!(result.exit_pc, 425);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(3));
}

#[test]
fn compile_native_guarded_numeric_for_loop_materializes_carried_integer_only_on_fallback_boundary()
{
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        }],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        430,
        431,
        &[(430, 431, 435)],
        &[(4, TraceValueKind::Integer)],
    );

    let entry = match backend
        .compile_native_guarded_numeric_for_loop(
            5,
            &lowering,
            NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::RegCompare {
                    op: LinearIntGuardOp::Lt,
                    lhs: 12,
                    rhs: 13,
                },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 435,
            },
            &lowered_trace,
        )
        .expect("expected native guarded numeric for loop")
    {
        NativeCompiledTrace::GuardedNumericForLoop { entry } => entry,
        other => panic!("expected native guarded numeric for loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(i64::MAX);
    stack[5] = LuaValue::integer(2);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);
    stack[12] = LuaValue::integer(0);
    stack[13] = LuaValue::integer(1);

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
    assert_eq!(stack[4].as_integer(), Some(i64::MAX));
}

#[test]
fn compile_native_guarded_numeric_for_loop_does_not_hoist_overwritten_stable_rhs() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(10),
                op: NumericBinaryOp::Add,
            },
            NumericStep::LoadI { dst: 10, imm: 1 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::StableReg {
                    reg: 10,
                    kind: TraceValueKind::Integer,
                },
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        435,
        437,
        &[(436, 437, 439)],
        &[
            (4, TraceValueKind::Integer),
            (5, TraceValueKind::Integer),
            (6, TraceValueKind::Integer),
            (8, TraceValueKind::Integer),
            (10, TraceValueKind::Integer),
        ],
    );

    let entry = match backend
        .compile_native_guarded_numeric_for_loop(
            5,
            &lowering,
            NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::RegCompare {
                    op: LinearIntGuardOp::Lt,
                    lhs: 4,
                    rhs: 8,
                },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 439,
            },
            &lowered_trace,
        )
        .expect("expected native guarded numeric for loop")
    {
        NativeCompiledTrace::GuardedNumericForLoop { entry } => entry,
        other => panic!("expected native guarded numeric for loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(10);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);
    stack[8] = LuaValue::integer(5);
    stack[10] = LuaValue::integer(2);

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
    assert_eq!(result.hits, 2);
    assert_eq!(result.exit_pc, 439);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(5));
    assert_eq!(stack[10].as_integer(), Some(1));
}

#[test]
fn compile_native_guarded_numeric_for_loop_keeps_integer_self_update_with_residual_move() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::ImmI(1),
                op: NumericBinaryOp::Add,
            },
            NumericStep::Move { dst: 11, src: 4 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Add,
                kind: NumericSelfUpdateValueKind::Integer,
                rhs: NumericValueFlowRhs::ImmI(1),
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        440,
        442,
        &[(441, 442, 443)],
        &[
            (4, TraceValueKind::Integer),
            (5, TraceValueKind::Integer),
            (6, TraceValueKind::Integer),
            (8, TraceValueKind::Integer),
        ],
    );

    let entry = match backend
        .compile_native_guarded_numeric_for_loop(
            5,
            &lowering,
            NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::RegCompare {
                    op: LinearIntGuardOp::Lt,
                    lhs: 11,
                    rhs: 8,
                },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 443,
            },
            &lowered_trace,
        )
        .expect("expected native guarded numeric for loop")
    {
        NativeCompiledTrace::GuardedNumericForLoop { entry } => entry,
        other => panic!("expected native guarded numeric for loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::integer(1);
    stack[5] = LuaValue::integer(10);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);
    stack[8] = LuaValue::integer(3);

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
    assert_eq!(result.exit_pc, 443);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_integer(), Some(3));
    assert_eq!(stack[11].as_integer(), Some(3));
}

#[test]
fn compile_native_guarded_numeric_for_loop_keeps_float_self_update_with_residual_move() {
    let mut backend = NativeTraceBackend::default();
    let lowering = NumericLowering {
        steps: vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(8),
                op: NumericBinaryOp::Mul,
            },
            NumericStep::Move { dst: 11, src: 4 },
        ],
        value_state: NumericValueState {
            self_update: Some(NumericSelfUpdateValueFlow {
                reg: 4,
                op: NumericBinaryOp::Mul,
                kind: NumericSelfUpdateValueKind::Float,
                rhs: NumericValueFlowRhs::StableReg {
                    reg: 8,
                    kind: TraceValueKind::Integer,
                },
            }),
        },
    };
    let lowered_trace = lowered_trace_for_numeric_jmp_blocks(
        448,
        450,
        &[(449, 450, 451)],
        &[
            (4, TraceValueKind::Float),
            (5, TraceValueKind::Integer),
            (6, TraceValueKind::Integer),
            (8, TraceValueKind::Integer),
        ],
    );

    let entry = match backend
        .compile_native_guarded_numeric_for_loop(
            5,
            &lowering,
            NumericJmpLoopGuard::Tail {
                cond: NumericIfElseCond::RegCompare {
                    op: LinearIntGuardOp::Lt,
                    lhs: 11,
                    rhs: 8,
                },
                continue_when: true,
                continue_preset: None,
                exit_preset: None,
                exit_pc: 451,
            },
            &lowered_trace,
        )
        .expect("expected native guarded numeric for loop")
    {
        NativeCompiledTrace::GuardedNumericForLoop { entry } => entry,
        other => panic!("expected native guarded numeric for loop, got {other:?}"),
    };

    let mut stack = [LuaValue::nil(); 16];
    stack[4] = LuaValue::float(1.0);
    stack[5] = LuaValue::integer(10);
    stack[6] = LuaValue::integer(1);
    stack[7] = LuaValue::integer(0);
    stack[8] = LuaValue::integer(3);

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
    assert_eq!(result.hits, 0);
    assert_eq!(result.exit_pc, 451);
    assert_eq!(result.exit_index, 0);
    assert_eq!(stack[4].as_float(), Some(3.0));
    assert_eq!(stack[11].as_float(), Some(3.0));
}
