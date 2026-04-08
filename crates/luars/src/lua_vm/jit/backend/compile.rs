use crate::Instruction;

use crate::lua_vm::jit::ir::TraceIrInst;
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

pub(super) fn lower_numeric_steps_for_native(insts: &[TraceIrInst]) -> Option<Vec<NumericStep>> {
    compile_numeric_steps(insts)
}

pub(super) fn lower_numeric_guard_for_native(
    inst: &TraceIrInst,
    tail: bool,
    exit_pc: u32,
) -> Option<NumericJmpLoopGuard> {
    compile_numeric_jmp_guard(inst, tail, exit_pc)
}
