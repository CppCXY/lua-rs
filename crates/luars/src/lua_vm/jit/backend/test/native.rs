use super::*;
use crate::gc::{Gc, GcUpvalue, UpvaluePtr};
use crate::lua_value::LuaUpvalue;
use crate::{LuaValue, LuaVM, SafeOption};
use crate::lua_vm::jit::backend::{
    CompiledTraceExecution, LinearIntGuardOp, NativeCompiledTrace, NativeTraceBackend,
    NativeTraceResult, NativeTraceStatus, NumericBinaryOp, NumericIfElseCond,
    NumericJmpLoopGuard, NumericJmpLoopGuardBlock, NumericOperand, NumericStep,
};
use crate::lua_vm::jit::lowering::{
    DeoptRestoreSummary, LoweredExit, LoweredSsaTrace, LoweredTrace, RegisterValueHint,
    TraceValueKind,
};

fn boxed_closed_upvalue(value: LuaValue) -> Box<GcUpvalue> {
    let mut upvalue = Box::new(Gc::new(
        LuaUpvalue::new_closed(value),
        0,
        std::mem::size_of::<GcUpvalue>() as u32,
    ));
    upvalue.data.fix_closed_ptr();
    upvalue
}

fn lowered_trace_for_numeric_jmp_blocks(
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 54,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(6),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(6),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(5),
                ],
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(5),
                ],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 54,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 6, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
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
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntJmpLoop { entry }) => entry,
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 141,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 14, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 151,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 9, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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

    let entry = match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::integer(2)]) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => entry,
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

    let entry = match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::integer(2)]) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => entry,
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

    let entry = match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::integer(2)]) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => entry,
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
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => entry,
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
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => entry,
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
            CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => entry,
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

    let entry = match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::float(2.0)]) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::NumericForLoop { entry }) => entry,
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(6),
                ],
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
            CompiledTraceExecution::Native(NativeCompiledTrace::GuardedNumericForLoop { entry }) => {
                entry
            }
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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

    let entry = match backend.compile_test_with_constants(&ir, &helper_plan, vec![LuaValue::float(2.0)]) {
        BackendCompileOutcome::Compiled(compiled) => match compiled.execution() {
            CompiledTraceExecution::Native(NativeCompiledTrace::GuardedNumericForLoop { entry }) => {
                entry
            }
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
                reads: vec![
                    TraceIrOperand::Register(4),
                    TraceIrOperand::Register(5),
                ],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(2)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(2)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(2)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(2)],
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

    let entry = match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
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

    let entry = match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
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

    let entry = match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
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

    let entry = match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
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

    let entry = match backend.compile_test_numeric_jmp_blocks(&[], &steps, &tail_blocks, &lowered_trace) {
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 117,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 118,
                opcode: OpCode::MulK,
                raw_instruction: Instruction::create_abc(OpCode::MulK, 4, 4, 1).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 119,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 1, 8, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(1)],
                writes: vec![],
            },
            TraceIrInst {
                pc: 120,
                opcode: OpCode::SubK,
                raw_instruction: Instruction::create_abc(OpCode::SubK, 4, 4, 2).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::Arithmetic,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(2)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 121,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 2, 7, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(2)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(1)],
            },
            HelperPlanStep::Arithmetic {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(2)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(2)],
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
    let constants = [LuaValue::float(0.5), LuaValue::float(2.0), LuaValue::float(0.5)];
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 141,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 8, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 124,
                opcode: OpCode::MmBinK,
                raw_instruction: Instruction::create_abck(OpCode::MmBinK, 4, 0, 6, false).as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            HelperPlanStep::MetamethodFallback {
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::ConstantIndex(0)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
                writes: vec![TraceIrOperand::Register(4)],
            },
            TraceIrInst {
                pc: 6,
                opcode: OpCode::MmBinI,
                raw_instruction: Instruction::create_abck(OpCode::MmBinI, 4, 128, 0, false)
                    .as_u32(),
                kind: crate::lua_vm::jit::ir::TraceIrInstKind::MetamethodFallback,
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
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
    src_table.as_table_mut().unwrap().raw_seti(1, LuaValue::integer(11));
    src_table.as_table_mut().unwrap().raw_seti(2, LuaValue::integer(22));
    src_table.as_table_mut().unwrap().raw_seti(3, LuaValue::integer(33));

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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
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

    match backend.compile_test_numeric_jmp_blocks(&head_blocks, &steps, &tail_blocks, &lowered_trace) {
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

    let entry = match backend.compile_test_numeric_jmp_blocks(&head_blocks, &steps, &tail_blocks, &lowered_trace) {
        Some(NativeCompiledTrace::NumericJmpLoop { entry }) => entry,
        other => panic!("expected native numeric jmp execution, got {other:?}"),
    };

    let mut vm = LuaVM::new(SafeOption::default());
    let src_table = vm.create_table(8, 0).unwrap();
    let dst_table = vm.create_table(8, 0).unwrap();
    src_table.as_table_mut().unwrap().raw_seti(1, LuaValue::integer(11));
    src_table.as_table_mut().unwrap().raw_seti(2, LuaValue::integer(22));
    src_table.as_table_mut().unwrap().raw_seti(3, LuaValue::integer(33));

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
    assert_eq!(dst_table.as_table().unwrap().raw_geti(1), Some(LuaValue::integer(11)));
    assert_eq!(dst_table.as_table().unwrap().raw_geti(2), Some(LuaValue::integer(22)));

    let deopt = lowered_trace
        .deopt_target_for_exit_index(result.exit_index.try_into().unwrap())
        .unwrap();
    assert_eq!(deopt.resume_pc, 702);
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
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
                reads: vec![TraceIrOperand::Register(4), TraceIrOperand::SignedImmediate(1)],
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
    src_table.as_table_mut().unwrap().raw_seti(1, LuaValue::integer(11));
    src_table.as_table_mut().unwrap().raw_seti(2, LuaValue::integer(22));
    src_table.as_table_mut().unwrap().raw_seti(3, LuaValue::integer(33));

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
    assert_eq!(dst_table.as_table().unwrap().raw_geti(1), Some(LuaValue::integer(11)));
    assert_eq!(dst_table.as_table().unwrap().raw_geti(2), Some(LuaValue::integer(22)));
    assert_eq!(dst_table.as_table().unwrap().raw_geti(3), Some(LuaValue::integer(33)));
}

