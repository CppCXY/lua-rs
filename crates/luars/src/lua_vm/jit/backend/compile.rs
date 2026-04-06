use crate::{lua_value::LuaProto, Instruction};

use crate::lua_vm::jit::ir::{TraceIr, TraceIrGuard, TraceIrGuardKind, TraceIrInst, TraceIrInstKind};
use super::model::{
    CompiledTraceExecutor, LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep,
    NumericBinaryOp, NumericIfElseCond, NumericJmpLoopGuard, NumericOperand, NumericStep,
};
use crate::lua_vm::jit::trace_recorder::TraceArtifact;

include!("compile_shared.rs");
include!("compile_loops.rs");
include!("compile_branches.rs");
include!("compile_iterators.rs");

pub(super) fn compile_executor(artifact: &TraceArtifact, ir: &TraceIr) -> CompiledTraceExecutor {
    if let Some(executor) = compile_numeric_for_sort_check_sum_loop(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_table_is_sorted(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_gettable_add(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_table_mul_add_mod(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_table_copy(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_upvalue_addi(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_field_addi(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_tabup_addi(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_tabup_field_addi(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_tabup_field_load(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_builtin_unary_const_call(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_tabup_field_string_unary_call(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_for_lua_closure_addi(ir) {
        return executor;
    }

    if let Some(executor) = compile_linear_int_forloop(ir) {
        return executor;
    }

    if let Some(executor) = compile_guarded_numeric_ifelse_forloop(artifact, ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_ifelse_forloop(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_forloop(ir) {
        return executor;
    }

    if let Some(executor) = compile_linear_int_jmp_loop(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_table_shift_jmp_loop(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_table_scan_jmp_loop(ir) {
        return executor;
    }

    if let Some(executor) = compile_numeric_jmp_loop(ir) {
        return executor;
    }

    if let Some(executor) = compile_generic_for_builtin_add(ir) {
        return executor;
    }

    if let Some(executor) = compile_next_while_builtin_add(ir) {
        return executor;
    }

    CompiledTraceExecutor::SummaryOnly
}
