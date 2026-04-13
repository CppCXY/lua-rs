use super::*;

pub(super) fn test_native_tfor_iterator(
    state: &mut crate::lua_vm::LuaState,
) -> crate::lua_vm::LuaResult<usize> {
    let limit = state
        .get_arg(1)
        .and_then(|value| value.as_integer())
        .unwrap_or(0);
    let control = state
        .get_arg(2)
        .and_then(|value| value.as_integer())
        .unwrap_or(0);
    if control >= limit {
        return Ok(0);
    }

    let next = control + 1;
    state.push_value(LuaValue::integer(next))?;
    state.push_value(LuaValue::integer(next * 10))?;
    Ok(2)
}

pub(super) fn test_native_call_increment(
    state: &mut crate::lua_vm::LuaState,
) -> crate::lua_vm::LuaResult<usize> {
    let arg = state
        .get_arg(1)
        .and_then(|value| value.as_integer())
        .unwrap_or(0);
    state.push_value(LuaValue::integer(arg + 5))?;
    Ok(1)
}

pub(super) fn call_for_loop_test_ir() -> TraceIr {
    TraceIr {
        root_pc: 0,
        loop_tail_pc: 3,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 0, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(0)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 1, 10, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(10)],
                writes: vec![TraceIrOperand::Register(1)],
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::Call,
                raw_instruction: Instruction::create_abc(OpCode::Call, 0, 2, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(1)],
                writes: vec![TraceIrOperand::RegisterRange { start: 0, count: 1 }],
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 8, 4).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    }
}

pub(super) fn call_for_loop_test_helper_plan() -> HelperPlan {
    HelperPlan {
        root_pc: 0,
        loop_tail_pc: 3,
        steps: vec![
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(0)],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(10)],
                writes: vec![TraceIrOperand::Register(1)],
            },
            HelperPlanStep::Call {
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(1)],
                writes: vec![TraceIrOperand::RegisterRange { start: 0, count: 1 }],
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
            call_steps: 1,
            metamethod_steps: 0,
        },
    }
}

pub(super) fn call_for_loop_with_post_move_test_ir() -> TraceIr {
    TraceIr {
        root_pc: 0,
        loop_tail_pc: 4,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 0, 4, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(0)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::AddI,
                raw_instruction: Instruction::create_abc(OpCode::AddI, 1, 1, 129).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![
                    TraceIrOperand::Register(1),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(1)],
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::Call,
                raw_instruction: Instruction::create_abc(OpCode::Call, 0, 2, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(1)],
                writes: vec![TraceIrOperand::RegisterRange { start: 0, count: 1 }],
            },
            TraceIrInst {
                pc: 3,
                opcode: OpCode::Move,
                raw_instruction: Instruction::create_abc(OpCode::Move, 2, 0, 0).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoadMove,
                reads: vec![TraceIrOperand::Register(0)],
                writes: vec![TraceIrOperand::Register(2)],
            },
            TraceIrInst {
                pc: 4,
                opcode: OpCode::ForLoop,
                raw_instruction: Instruction::create_abx(OpCode::ForLoop, 8, 5).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    }
}

pub(super) fn call_for_loop_with_post_move_test_helper_plan() -> HelperPlan {
    HelperPlan {
        root_pc: 0,
        loop_tail_pc: 4,
        steps: vec![
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(0)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![
                    TraceIrOperand::Register(1),
                    TraceIrOperand::SignedImmediate(1),
                ],
                writes: vec![TraceIrOperand::Register(1)],
            },
            HelperPlanStep::Call {
                reads: vec![TraceIrOperand::Register(0), TraceIrOperand::Register(1)],
                writes: vec![TraceIrOperand::RegisterRange { start: 0, count: 1 }],
            },
            HelperPlanStep::LoadMove {
                reads: vec![TraceIrOperand::Register(0)],
                writes: vec![TraceIrOperand::Register(2)],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guard_count: 0,
        summary: HelperPlanDispatchSummary {
            steps_executed: 5,
            guards_observed: 0,
            call_steps: 1,
            metamethod_steps: 0,
        },
    }
}

pub(super) fn tfor_loop_test_ir() -> TraceIr {
    TraceIr {
        root_pc: 0,
        loop_tail_pc: 2,
        insts: vec![
            TraceIrInst {
                pc: 0,
                opcode: OpCode::Add,
                raw_instruction: Instruction::create_abck(OpCode::Add, 8, 8, 4, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            TraceIrInst {
                pc: 1,
                opcode: OpCode::TForCall,
                raw_instruction: Instruction::create_abc(OpCode::TForCall, 0, 0, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Call,
                reads: vec![
                    TraceIrOperand::Register(0),
                    TraceIrOperand::Register(1),
                    TraceIrOperand::Register(3),
                    TraceIrOperand::UnsignedImmediate(2),
                    TraceIrOperand::UnsignedImmediate(2),
                ],
                writes: vec![TraceIrOperand::RegisterRange { start: 3, count: 2 }],
            },
            TraceIrInst {
                pc: 2,
                opcode: OpCode::TForLoop,
                raw_instruction: Instruction::create_abx(OpCode::TForLoop, 0, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::LoopBackedge,
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guards: vec![crate::lua_vm::jit::ir::TraceIrGuard {
            guard_pc: 2,
            branch_pc: 2,
            exit_pc: 3,
            taken_on_trace: true,
            kind: crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit,
        }],
    }
}

pub(super) fn tfor_loop_test_helper_plan() -> HelperPlan {
    HelperPlan {
        root_pc: 0,
        loop_tail_pc: 2,
        steps: vec![
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(8), TraceIrOperand::Register(4)],
                writes: vec![TraceIrOperand::Register(8)],
            },
            HelperPlanStep::Call {
                reads: vec![
                    TraceIrOperand::Register(0),
                    TraceIrOperand::Register(1),
                    TraceIrOperand::Register(3),
                ],
                writes: vec![TraceIrOperand::RegisterRange { start: 3, count: 2 }],
            },
            HelperPlanStep::LoopBackedge {
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::JumpTarget(0)],
                writes: Vec::new(),
            },
        ],
        guard_count: 1,
        summary: HelperPlanDispatchSummary {
            steps_executed: 3,
            guards_observed: 0,
            call_steps: 1,
            metamethod_steps: 0,
        },
    }
}

pub(super) fn boxed_closed_upvalue(value: LuaValue) -> Box<GcUpvalue> {
    let mut upvalue = Box::new(Gc::new(
        LuaUpvalue::new_closed(value),
        0,
        std::mem::size_of::<GcUpvalue>() as u32,
    ));
    upvalue.data.fix_closed_ptr();
    upvalue
}

pub(super) fn lowered_trace_for_numeric_jmp_blocks(
    root_pc: u32,
    loop_tail_pc: u32,
    exits: &[(u32, u32, u32)],
    entry_hints: &[(u32, TraceValueKind)],
) -> LoweredTrace {
    let hints = entry_hints
        .iter()
        .map(|&(reg, kind)| RegisterValueHint { reg, kind })
        .collect::<Vec<_>>();

    LoweredTrace {
        root_pc,
        loop_tail_pc,
        snapshots: Vec::new(),
        exits: exits
            .iter()
            .enumerate()
            .map(|(index, &(guard_pc, branch_pc, exit_pc))| LoweredExit {
                exit_index: index as u16,
                guard_pc,
                branch_pc,
                exit_pc,
                resume_pc: exit_pc,
                snapshot_id: (index + 1) as u16,
                is_loop_backedge: false,
                restore_summary: DeoptRestoreSummary::default(),
            })
            .collect(),
        helper_plan_step_count: 0,
        constants: Vec::new(),
        root_register_hints: hints.clone(),
        entry_stable_register_hints: hints,
        ssa_trace: LoweredSsaTrace::default(),
    }
}


pub(super) fn lowered_trace_with_entry_hints(hints: &[(u32, TraceValueKind)]) -> LoweredTrace {
    let hints = hints
        .iter()
        .map(|&(reg, kind)| crate::lua_vm::jit::lowering::RegisterValueHint { reg, kind })
        .collect::<Vec<_>>();

    LoweredTrace {
        root_pc: 0,
        loop_tail_pc: 0,
        snapshots: Vec::new(),
        exits: Vec::new(),
        helper_plan_step_count: 0,
        constants: Vec::new(),
        root_register_hints: hints.clone(),
        entry_stable_register_hints: hints,
        ssa_trace: LoweredSsaTrace::default(),
    }
}

pub(super) fn lowered_trace_with_entry_hints_and_constants(
    hints: &[(u32, TraceValueKind)],
    constants: Vec<LuaValue>,
) -> LoweredTrace {
    let mut lowered = lowered_trace_with_entry_hints(hints);
    lowered.constants = constants;
    lowered
}
