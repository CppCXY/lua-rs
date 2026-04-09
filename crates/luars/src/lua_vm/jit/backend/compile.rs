use crate::Instruction;

use crate::lua_vm::jit::ir::TraceIrInst;
use crate::lua_vm::jit::lowering::{LoweredTrace, SsaTableIntRewrite};
use super::model::{
    LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep, NumericBinaryOp, NumericIfElseCond,
    NumericJmpLoopGuard, NumericOperand, NumericStep,
};

include!("compile_shared.rs");

pub(super) fn lower_linear_int_steps_for_native(insts: &[TraceIrInst]) -> Option<Vec<LinearIntStep>> {
    compile_linear_int_steps(insts)
}

pub(super) fn lower_linear_int_guard_for_native(
    inst: &TraceIrInst,
    tail: bool,
    exit_pc: u32,
) -> Option<LinearIntLoopGuard> {
    compile_linear_int_guard(inst, tail, exit_pc)
}

pub(super) fn lower_numeric_steps_for_native(
    insts: &[TraceIrInst],
    lowered_trace: &LoweredTrace,
) -> Option<Vec<NumericStep>> {
    compile_numeric_steps(insts, lowered_trace)
}

pub(super) fn lower_numeric_guard_for_native(
    inst: &TraceIrInst,
    tail: bool,
    exit_pc: u32,
) -> Option<NumericJmpLoopGuard> {
    compile_numeric_jmp_guard(inst, tail, exit_pc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua_vm::jit::helper_plan::HelperPlan;
    use crate::lua_vm::jit::ir::{TraceIr, TraceIrInst, TraceIrInstKind, TraceIrOperand};
    use crate::lua_vm::jit::lowering::LoweredTrace;
    use crate::lua_vm::jit::backend::model::synthetic_artifact_for_ir;

    #[test]
    fn lower_numeric_steps_consumes_ssa_table_int_rewrites() {
        let ir = TraceIr {
            root_pc: 40,
            loop_tail_pc: 43,
            insts: vec![
                TraceIrInst {
                    pc: 40,
                    opcode: crate::OpCode::SetTable,
                    raw_instruction: Instruction::create_abck(crate::OpCode::SetTable, 3, 7, 8, false)
                        .as_u32(),
                    kind: TraceIrInstKind::TableAccess,
                    reads: vec![
                        TraceIrOperand::Register(3),
                        TraceIrOperand::Register(7),
                        TraceIrOperand::Register(8),
                    ],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 41,
                    opcode: crate::OpCode::GetTable,
                    raw_instruction: Instruction::create_abck(crate::OpCode::GetTable, 9, 3, 7, false)
                        .as_u32(),
                    kind: TraceIrInstKind::TableAccess,
                    reads: vec![TraceIrOperand::Register(3), TraceIrOperand::Register(7)],
                    writes: vec![TraceIrOperand::Register(9)],
                },
                TraceIrInst {
                    pc: 42,
                    opcode: crate::OpCode::SetTable,
                    raw_instruction: Instruction::create_abck(crate::OpCode::SetTable, 3, 7, 10, false)
                        .as_u32(),
                    kind: TraceIrInstKind::TableAccess,
                    reads: vec![
                        TraceIrOperand::Register(3),
                        TraceIrOperand::Register(7),
                        TraceIrOperand::Register(10),
                    ],
                    writes: Vec::new(),
                },
                TraceIrInst {
                    pc: 43,
                    opcode: crate::OpCode::SetTable,
                    raw_instruction: Instruction::create_abck(crate::OpCode::SetTable, 3, 7, 11, false)
                        .as_u32(),
                    kind: TraceIrInstKind::TableAccess,
                    reads: vec![
                        TraceIrOperand::Register(3),
                        TraceIrOperand::Register(7),
                        TraceIrOperand::Register(11),
                    ],
                    writes: Vec::new(),
                },
            ],
            guards: Vec::new(),
        };
        let helper_plan = HelperPlan::lower(&ir);
        let artifact = synthetic_artifact_for_ir(&ir);
        let lowered_trace = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        let steps = lower_numeric_steps_for_native(&ir.insts, &lowered_trace).unwrap();

        assert_eq!(
            steps,
            vec![
                NumericStep::Move { dst: 9, src: 8 },
                NumericStep::SetTableInt {
                    table: 3,
                    index: 7,
                    value: 11,
                },
            ]
        );
    }
}
