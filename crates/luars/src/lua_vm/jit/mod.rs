mod backend;
mod hotcount;
mod helper_plan;
mod ir;
mod state;
mod trace_recorder;

use crate::lua_value::LuaProto;
use crate::lua_vm::LuaState;

pub(crate) use backend::{
    CompiledTraceExecutor, LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep, NumericBinaryOp,
    NumericOperand, NumericStep,
};
pub(crate) use helper_plan::HelperPlanDispatchSummary;

pub(crate) use state::JitState;
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
pub(crate) fn try_enter_recorded_trace(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) -> bool {
    if chunk_ptr.is_null() || !should_track(lua_state) {
        return false;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return false;
    };

    lua_state.vm_mut().jit.try_enter_trace(chunk_ptr, pc)
}

#[inline(always)]
pub(crate) fn compiled_trace_executor(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) -> Option<(CompiledTraceExecutor, HelperPlanDispatchSummary)> {
    if chunk_ptr.is_null() || !should_track(lua_state) {
        return None;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return None;
    };

    lua_state.vm_mut().jit.compiled_trace_executor(chunk_ptr, pc)
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