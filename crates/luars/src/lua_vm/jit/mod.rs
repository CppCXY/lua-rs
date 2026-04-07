mod backend;
mod executors;
mod hotcount;
mod helper_plan;
mod ir;
mod lowering;
#[cfg(feature = "jit")]
mod runtime;
mod state;
mod trace_recorder;

use crate::lua_value::LuaProto;
use crate::lua_vm::{CallInfo, LuaState};

pub(crate) use backend::{
    CompiledTraceExecutor, LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep, NumericBinaryOp,
    NumericIfElseCond, NumericJmpLoopGuard, NumericOperand, NumericStep,
};
pub(crate) use helper_plan::HelperPlanDispatchSummary;
#[cfg(feature = "jit")]
pub(crate) use runtime::{
    dispatch_root_trace_or_record, finish_trace_exit, record_trace_hits_or_fallback,
};
pub(crate) use trace_recorder::TraceAbortReason;

pub(crate) use state::{ExecutableTraceDispatch, JitState, TraceExitDispatch};
pub use state::{JitAbortCounters, JitCounters, JitStatsSnapshot};

#[inline(always)]
fn should_track(lua_state: &LuaState) -> bool {
    if lua_state.hook_mask != 0 {
        return false;
    }

    #[cfg(feature = "sandbox")]
    if lua_state.has_active_instruction_watch() {
        return false;
    }

    true
}

#[inline(always)]
pub(crate) fn record_loop_backedge(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) {
    if chunk_ptr.is_null() || !should_track(lua_state) {
        return;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return;
    };

    lua_state.vm_mut().jit.record_loop_backedge(chunk_ptr, pc);
}

#[inline(always)]
pub(crate) fn compiled_trace_executor_or_record(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) -> Option<ExecutableTraceDispatch> {
    if chunk_ptr.is_null() || !should_track(lua_state) {
        return None;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return None;
    };

    lua_state
        .vm_mut()
        .jit
        .compiled_trace_executor_or_record(chunk_ptr, pc)
}

#[inline(always)]
pub(crate) fn record_batched_trace_execution(
    lua_state: &mut LuaState,
    checks: u32,
    hits: u32,
    summary: HelperPlanDispatchSummary,
) {
    if !should_track(lua_state) {
        return;
    }

    lua_state
        .vm_mut()
        .jit
        .record_batched_trace_execution(checks, hits, summary);
}

#[inline(always)]
pub(crate) unsafe fn resolve_trace_exit(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    parent_pc: usize,
    exit_pc: usize,
) -> Option<TraceExitDispatch> {
    if ci.chunk_ptr.is_null() || !should_track(lua_state) {
        return None;
    }

    let Ok(parent_pc) = u32::try_from(parent_pc) else {
        return None;
    };
    let Ok(exit_pc) = u32::try_from(exit_pc) else {
        return None;
    };

    let chunk = unsafe { &*ci.chunk_ptr };
    let stack = lua_state.stack().as_ptr();

    unsafe {
        lua_state.vm_mut().jit.resolve_trace_exit(
            ci.chunk_ptr,
            parent_pc,
            exit_pc,
            stack,
            base,
            &chunk.constants,
            ci.upvalue_ptrs,
        )
    }
}

#[inline(always)]
pub(crate) fn blacklist_trace(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
    reason: TraceAbortReason,
) {
    if chunk_ptr.is_null() || !should_track(lua_state) {
        return;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return;
    };

    lua_state.vm_mut().jit.blacklist_trace(chunk_ptr, pc, reason);
}