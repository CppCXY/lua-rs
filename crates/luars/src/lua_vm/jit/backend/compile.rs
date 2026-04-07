use crate::{lua_value::LuaProto, Instruction};

use crate::lua_vm::jit::ir::{TraceIr, TraceIrGuard, TraceIrGuardKind, TraceIrInst, TraceIrInstKind};
use crate::lua_vm::jit::lowering::{LoweredExit, LoweredTrace};
use super::model::{
    CompiledTraceExecutor, LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep,
    NumericBinaryOp, NumericIfElseCond, NumericJmpLoopGuard, NumericOperand, NumericStep,
};
use crate::lua_vm::jit::trace_recorder::TraceArtifact;

include!("compile_shared.rs");
include!("compile_loops.rs");
include!("compile_branches.rs");
include!("compile_iterators.rs");

pub(super) fn compile_executor(
    artifact: &TraceArtifact,
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<CompiledTraceExecutor> {
    if let Some(executor) = compile_linear_int_forloop(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_guarded_numeric_ifelse_forloop(artifact, ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_ifelse_forloop(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_forloop(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_linear_int_jmp_loop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_table_shift_jmp_loop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_table_scan_jmp_loop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_jmp_loop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_generic_for_builtin_add(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_next_while_builtin_add(ir) {
        return Some(executor);
    }

    None
}
