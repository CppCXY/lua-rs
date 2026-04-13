use cranelift::codegen::isa::{CallConv, TargetFrontendConfig};
use cranelift::codegen::settings;
use cranelift::prelude::*;
use cranelift_codegen::ir::FuncRef;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

use crate::gc::UpvaluePtr;
use crate::lua_value::{LUA_VNUMFLT, LUA_VNUMINT};
use crate::stdlib::math::try_call_fast_math;
use crate::lua_vm::execute::helper::{
    finishget_known_miss, lua_fmod, lua_idiv, lua_imod, luai_numpow, objlen_value, pivalue,
    pttisinteger, ttisfloat, ttisinteger,
};
use crate::lua_vm::execute::{call::precall, execute_loop::lua_execute};
use crate::lua_vm::jit::helper_plan::HelperPlan;
use crate::lua_vm::jit::ir::{TraceIr, TraceIrInst};
use crate::lua_vm::jit::lowering::{LoweredTrace, TraceValueKind};
use crate::lua_vm::jit::trace_recorder::TraceArtifact;
use crate::{Instruction, LuaState, LuaValue};
use super::compile::{
    build_numeric_value_state, extract_loop_prologue, lower_linear_int_guard_for_native,
    lower_linear_int_steps_for_native, lower_numeric_guard_for_native,
    lower_numeric_lowering_for_native, lower_numeric_steps_for_native,
    lower_numeric_steps_for_native_with_live_out,
};
use super::{
    BackendCompileOutcome, CompiledTrace, CompiledTraceExecution, LinearIntGuardOp,
    LinearIntLoopGuard, LinearIntStep, NativeCompiledTrace, NativeLoweringProfile,
    NativeTraceResult, NativeTraceStatus, NumericBinaryOp, NumericIfElseCond, NumericJmpLoopGuard,
    NumericJmpLoopGuardBlock, NumericLowering, NumericOperand, NumericSelfUpdateValueFlow,
    NumericSelfUpdateValueKind, NumericStep, NumericValueFlowRhs, TraceBackend,
    lowered_execution_for_artifact,
};

mod emit;
mod families;
mod profile;

use self::emit::*;

const LUA_VALUE_SIZE: i64 = std::mem::size_of::<LuaValue>() as i64;
const LUA_VALUE_TT_OFFSET: i32 = std::mem::offset_of!(LuaValue, tt) as i32;
const LUA_VALUE_VALUE_OFFSET: i32 = std::mem::offset_of!(LuaValue, value) as i32;
const LUA_VNIL_TAG: u8 = 0;
const LUA_VFALSE_TAG: u8 = 1;
const LUA_VTRUE_TAG: u8 = 17;
const NATIVE_HELPER_NUMERIC_GET_UPVAL_SYMBOL: &str = "jit_native_helper_numeric_get_upval";
const NATIVE_HELPER_NUMERIC_SET_UPVAL_SYMBOL: &str = "jit_native_helper_numeric_set_upval";
const NATIVE_HELPER_NUMERIC_GET_TABUP_FIELD_SYMBOL: &str =
    "jit_native_helper_numeric_get_tabup_field";
const NATIVE_HELPER_NUMERIC_SET_TABUP_FIELD_SYMBOL: &str =
    "jit_native_helper_numeric_set_tabup_field";
const NATIVE_HELPER_NUMERIC_GET_TABLE_INT_SYMBOL: &str = "jit_native_helper_numeric_get_table_int";
const NATIVE_HELPER_NUMERIC_SET_TABLE_INT_SYMBOL: &str = "jit_native_helper_numeric_set_table_int";
const NATIVE_HELPER_NUMERIC_GET_TABLE_FIELD_SYMBOL: &str =
    "jit_native_helper_numeric_get_table_field";
const NATIVE_HELPER_NUMERIC_SET_TABLE_FIELD_SYMBOL: &str =
    "jit_native_helper_numeric_set_table_field";
const NATIVE_HELPER_NUMERIC_LEN_SYMBOL: &str = "jit_native_helper_numeric_len";
const NATIVE_HELPER_NUMERIC_BINARY_SYMBOL: &str = "jit_native_helper_numeric_binary";
const NATIVE_HELPER_NUMERIC_POW_SYMBOL: &str = "jit_native_helper_numeric_pow";
const NATIVE_HELPER_SHIFT_LEFT_SYMBOL: &str = "jit_native_helper_shift_left";
const NATIVE_HELPER_SHIFT_RIGHT_SYMBOL: &str = "jit_native_helper_shift_right";
const NATIVE_HELPER_CALL_SYMBOL: &str = "jit_native_helper_call";
const NATIVE_HELPER_TFOR_CALL_SYMBOL: &str = "jit_native_helper_tfor_call";
const NATIVE_TRACE_RESULT_STATUS_OFFSET: i32 =
    std::mem::offset_of!(NativeTraceResult, status) as i32;
const NATIVE_TRACE_RESULT_HITS_OFFSET: i32 = std::mem::offset_of!(NativeTraceResult, hits) as i32;
const NATIVE_TRACE_RESULT_EXIT_PC_OFFSET: i32 =
    std::mem::offset_of!(NativeTraceResult, exit_pc) as i32;
const NATIVE_TRACE_RESULT_START_REG_OFFSET: i32 =
    std::mem::offset_of!(NativeTraceResult, start_reg) as i32;
const NATIVE_TRACE_RESULT_RESULT_COUNT_OFFSET: i32 =
    std::mem::offset_of!(NativeTraceResult, result_count) as i32;
const NATIVE_TRACE_RESULT_EXIT_INDEX_OFFSET: i32 =
    std::mem::offset_of!(NativeTraceResult, exit_index) as i32;
const NATIVE_NUMERIC_OPERAND_REG: i32 = 0;
const NATIVE_NUMERIC_OPERAND_IMM_I: i32 = 1;
const NATIVE_NUMERIC_OPERAND_CONST: i32 = 2;
const NATIVE_NUMERIC_BINARY_ADD: i32 = 0;
const NATIVE_NUMERIC_BINARY_SUB: i32 = 1;
const NATIVE_NUMERIC_BINARY_MUL: i32 = 2;
const NATIVE_NUMERIC_BINARY_DIV: i32 = 3;
const NATIVE_NUMERIC_BINARY_IDIV: i32 = 4;
const NATIVE_NUMERIC_BINARY_MOD: i32 = 5;
const NATIVE_NUMERIC_BINARY_POW: i32 = 6;
const NATIVE_CALL_FALLBACK: i32 = 0;
const NATIVE_CALL_CONTINUE: i32 = 1;
const NATIVE_TFOR_CALL_FALLBACK: i32 = 0;
const NATIVE_TFOR_CALL_C_CONTINUE: i32 = 1;
const NATIVE_TFOR_CALL_LUA_RETURNED: i32 = 2;

use crate::gc::Gc;
use crate::lua_value::lua_table::LuaTable;
use crate::lua_value::lua_table::native_table::NativeTable;

const GC_TABLE_DATA_OFFSET: i32 = std::mem::offset_of!(Gc<LuaTable>, data) as i32;
const LUA_TABLE_IMPL_OFFSET: i32 = std::mem::offset_of!(LuaTable, impl_table) as i32;
const NATIVE_TABLE_ARRAY_OFFSET: i32 = std::mem::offset_of!(NativeTable, array) as i32;
const NATIVE_TABLE_ASIZE_OFFSET: i32 = std::mem::offset_of!(NativeTable, asize) as i32;
const LUA_VTABLE_TAG: i64 = 0x45;
const ARRAY_TAG_BASE_OFFSET: i32 = 4;
const VALUE_SIZE_BYTES: i64 = 8;

pub(crate) struct NativeTraceBackend {
    modules: Vec<JITModule>,
    next_function_index: u64,
}

impl Default for NativeTraceBackend {
    fn default() -> Self {
        Self {
            modules: Vec::new(),
            next_function_index: 0,
        }
    }
}

impl TraceBackend for NativeTraceBackend {
    fn compile(
        &mut self,
        artifact: &TraceArtifact,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome {
        if let Some((execution, native_profile)) = self.compile_native_generic_trace(ir, lowered_trace)
        {
            return match CompiledTrace::from_artifact_helper_plan_with_execution(
                artifact,
                ir,
                lowered_trace,
                helper_plan,
                execution,
                native_profile,
            ) {
                Some(compiled_trace) => BackendCompileOutcome::Compiled(compiled_trace),
                None => BackendCompileOutcome::NotYetSupported,
            };
        }

        match CompiledTrace::from_artifact_helper_plan_with_execution(
            artifact,
            ir,
            lowered_trace,
            helper_plan,
            lowered_execution_for_artifact(artifact),
            None,
        ) {
            Some(compiled_trace) => BackendCompileOutcome::Compiled(compiled_trace),
            None => BackendCompileOutcome::NotYetSupported,
        }
    }
}
