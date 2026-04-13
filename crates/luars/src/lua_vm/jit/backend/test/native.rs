use super::*;
use crate::gc::{Gc, GcUpvalue, UpvaluePtr};
use crate::lua_value::LuaUpvalue;
use crate::lua_vm::jit::backend::{
    CompiledTraceExecution, LinearIntGuardOp, NativeCompiledTrace, NativeTraceBackend,
    NativeTraceResult, NativeTraceStatus, NumericBinaryOp, NumericIfElseCond, NumericJmpLoopGuard,
    NumericJmpLoopGuardBlock, NumericLowering, NumericOperand, NumericSelfUpdateValueFlow,
    NumericSelfUpdateValueKind, NumericStep, NumericValueFlowRhs, NumericValueState,
};
use crate::lua_vm::jit::lowering::{
    DeoptRestoreSummary, LoweredExit, LoweredSsaTrace, LoweredTrace, RegisterValueHint,
    TraceValueKind,
};
use crate::{LuaVM, LuaValue, SafeOption};

mod compile_recognition;
mod iterator_call_access;
mod loop_families;
mod numeric_jmp;
mod shared;
mod value_state;
