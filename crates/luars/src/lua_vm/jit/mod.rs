mod backend;
mod helper_plan;
mod hotcount;
mod ir;
mod lowering;
mod runtime;
mod state;
mod trace_recorder;

use crate::lua_value::LuaProto;
use crate::lua_vm::{CallInfo, LuaState};

use backend::CompiledTraceExecution;
use helper_plan::HelperPlanDispatchSummary;
pub(crate) use runtime::dispatch_root_trace_or_record;
pub(crate) use state::{
    ExecutableTraceDispatch, JitState, ReadySideTraceDispatch, TraceExitDispatch,
};
pub use state::{JitAbortCounters, JitCounters, JitStatsSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JitTraceAction {
    ContinueAt(usize),
    Returned,
}

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
pub(crate) fn executable_trace_dispatch_or_record(
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
        .executable_trace_dispatch_or_record(chunk_ptr, pc)
}

#[inline(always)]
pub(crate) fn executable_trace_dispatch(
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
        .executable_trace_dispatch(chunk_ptr, pc)
}

#[inline(always)]
pub(crate) fn redirected_root_pc(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) -> Option<u32> {
    if chunk_ptr.is_null() || !should_track(lua_state) {
        return None;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return None;
    };

    lua_state.vm_mut().jit.redirected_root_pc(chunk_ptr, pc)
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
pub(crate) fn record_root_trace_dispatch(
    lua_state: &mut LuaState,
    dispatch: &ExecutableTraceDispatch,
) {
    if !should_track(lua_state) {
        return;
    }

    let jit = &mut lua_state.vm_mut().jit;
    jit.record_root_dispatch(&dispatch.execution);
    if let Some(profile) = dispatch.native_profile {
        jit.record_root_native_profile(profile);
    }
}

#[inline(always)]
pub(crate) fn record_linked_root_reentry_attempt_at(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) {
    if !should_track(lua_state) {
        return;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return;
    };

    lua_state
        .vm_mut()
        .jit
        .record_linked_root_reentry_attempt(chunk_ptr, pc);
}

#[inline(always)]
pub(crate) fn record_linked_root_reentry_hit_at(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) {
    if !should_track(lua_state) {
        return;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return;
    };

    lua_state
        .vm_mut()
        .jit
        .record_linked_root_reentry_hit(chunk_ptr, pc);
}

#[inline(always)]
pub(crate) fn prepare_linked_root_reentry_target_at(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) {
    if !should_track(lua_state) {
        return;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return;
    };

    lua_state
        .vm_mut()
        .jit
        .prepare_linked_root_reentry_target(chunk_ptr, pc);
}

#[inline(always)]
pub(crate) fn record_linked_root_reentry_fallback_at(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
) {
    if !should_track(lua_state) {
        return;
    }

    let Ok(pc) = u32::try_from(target_pc) else {
        return;
    };

    lua_state
        .vm_mut()
        .jit
        .record_linked_root_reentry_fallback(chunk_ptr, pc);
}

#[inline(always)]
pub(crate) fn record_ready_side_trace_dispatch(
    lua_state: &mut LuaState,
    dispatch: &ReadySideTraceDispatch,
) {
    if !should_track(lua_state) {
        return;
    }

    lua_state.vm_mut().jit.record_ready_side_dispatch(dispatch);
}

#[inline(always)]
pub(crate) fn record_redundant_side_exit_recovery(lua_state: &mut LuaState) {
    if !should_track(lua_state) {
        return;
    }

    lua_state.vm_mut().jit.record_redundant_side_exit_recovery();
}

#[inline(always)]
pub(crate) fn record_redundant_side_exit_fast_dispatch(lua_state: &mut LuaState) {
    if !should_track(lua_state) {
        return;
    }

    lua_state
        .vm_mut()
        .jit
        .record_redundant_side_exit_fast_dispatch();
}

#[inline(always)]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub(crate) unsafe fn resolve_trace_exit_by_index(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    parent_pc: usize,
    exit_index: u16,
) -> Option<TraceExitDispatch> {
    if ci.chunk_ptr.is_null() || !should_track(lua_state) {
        return None;
    }

    let Ok(parent_pc) = u32::try_from(parent_pc) else {
        return None;
    };

    let chunk = unsafe { &*ci.chunk_ptr };
    let stack = lua_state.stack().as_ptr();

    unsafe {
        lua_state.vm_mut().jit.resolve_trace_exit_by_index(
            ci.chunk_ptr,
            parent_pc,
            exit_index,
            stack,
            base,
            &chunk.constants,
            ci.upvalue_ptrs,
        )
    }
}
