use super::*;
use crate::lua_vm::jit::CompiledTraceExecution;
use crate::lua_vm::jit::backend::{NativeCompiledTrace, NativeTraceBackend};

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
                CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntJmpLoop {
                    exit_pc: 56,
                    ..
                })
            ));
        }
        BackendCompileOutcome::NotYetSupported => panic!("expected compiled trace"),
    }
}