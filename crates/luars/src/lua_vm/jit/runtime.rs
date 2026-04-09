use crate::{CallInfo, LuaState, LuaValue};
use crate::lua_value::LuaProto;

use super::{
    CompiledTraceExecution, ExecutableTraceDispatch, HelperPlanDispatchSummary,
    JitTraceAction, ReadySideTraceDispatch, TraceExitDispatch,
};
use super::backend::{NativeCompiledTrace, NativeTraceResult, NativeTraceStatus};
use super::state::NativeExecutableTraceDispatch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedExitKind {
    LoopExit,
    SideExit,
}

pub(crate) struct JitExecutionContext<'a> {
    lua_state: &'a mut LuaState,
    ci: &'a CallInfo,
    base: usize,
    constants: &'a [LuaValue],
}

impl<'a> JitExecutionContext<'a> {
    #[allow(unsafe_op_in_unsafe_fn)]
    unsafe fn new(lua_state: &'a mut LuaState, ci: &'a CallInfo, base: usize) -> Self {
        let constants = unsafe { &(*ci.chunk_ptr).constants };
        Self {
            lua_state,
            ci,
            base,
            constants,
        }
    }
}

#[inline(always)]
pub(crate) fn record_trace_hits_or_fallback(
    lua_state: &mut LuaState,
    chunk_ptr: *const LuaProto,
    target_pc: usize,
    trace_hits: u32,
    summary: HelperPlanDispatchSummary,
) {
    if trace_hits > 0 {
        super::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
    } else {
        super::record_loop_backedge(lua_state, chunk_ptr, target_pc);
    }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
pub(crate) unsafe fn dispatch_root_trace_or_record(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_exit_pc_override: Option<usize>,
) -> Option<JitTraceAction> {
    let Some(dispatch) = super::executable_trace_dispatch_or_record(lua_state, ci.chunk_ptr, target_pc)
    else {
        return None;
    };

    super::record_root_trace_dispatch(lua_state, &dispatch);

    let mut context = unsafe { JitExecutionContext::new(lua_state, ci, base) };
    unsafe { dispatch_executable_trace(&mut context, dispatch, loop_exit_pc_override) }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn finish_trace_exit_in_context(
    context: &mut JitExecutionContext<'_>,
    target_pc: usize,
    trace_hits: u32,
    summary: HelperPlanDispatchSummary,
    exit_pc: usize,
) -> JitTraceAction {
    super::record_batched_trace_execution(context.lua_state, trace_hits, trace_hits, summary);

    let Some(dispatch) = (unsafe {
        super::resolve_trace_exit(
            context.lua_state,
            context.ci,
            context.base,
            target_pc,
            exit_pc,
        )
    }) else {
        return JitTraceAction::ContinueAt(exit_pc);
    };

    unsafe { continue_after_resolved_exit(context, dispatch, ResolvedExitKind::LoopExit) }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn finish_trace_exit_by_index_in_context(
    context: &mut JitExecutionContext<'_>,
    target_pc: usize,
    trace_hits: u32,
    summary: HelperPlanDispatchSummary,
    exit_index: u16,
    fallback_exit_pc: usize,
) -> JitTraceAction {
    super::record_batched_trace_execution(context.lua_state, trace_hits, trace_hits, summary);

    let Some(dispatch) = (unsafe {
        super::resolve_trace_exit_by_index(
            context.lua_state,
            context.ci,
            context.base,
            target_pc,
            exit_index,
        )
    }) else {
        return JitTraceAction::ContinueAt(fallback_exit_pc);
    };

    unsafe { continue_after_resolved_exit(context, dispatch, ResolvedExitKind::SideExit) }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn continue_after_resolved_exit(
    context: &mut JitExecutionContext<'_>,
    dispatch: TraceExitDispatch,
    kind: ResolvedExitKind,
) -> JitTraceAction {
    let TraceExitDispatch {
        recovery,
        ready_side_trace,
    } = dispatch;
    let resume_pc = recovery.target.resume_pc as usize;
    let redundant_recovery = unsafe {
        recovery.is_redundant_for_state(
            context.lua_state.stack().as_ptr(),
            context.base,
            context.ci.upvalue_ptrs,
        )
    };

    if matches!(kind, ResolvedExitKind::SideExit) && redundant_recovery {
        super::record_redundant_side_exit_recovery(context.lua_state);
    }

    match ready_side_trace {
        Some(ReadySideTraceDispatch::Native(native_dispatch))
            if matches!(kind, ResolvedExitKind::SideExit) && redundant_recovery =>
        {
            let ready_dispatch = ReadySideTraceDispatch::Native(native_dispatch);
            super::record_ready_side_trace_dispatch(context.lua_state, &ready_dispatch);
            super::record_redundant_side_exit_fast_dispatch(context.lua_state);
            if let Some(action) = unsafe {
                dispatch_native_ready_side_trace_fast(context, native_dispatch)
            } {
                return action;
            }
            JitTraceAction::ContinueAt(resume_pc)
        }
        Some(ready_side_trace) => {
            if !redundant_recovery {
                unsafe { apply_deopt_recovery(context, &recovery) };
            }
            super::record_ready_side_trace_dispatch(context.lua_state, &ready_side_trace);
            if let Some(action) = unsafe { dispatch_ready_side_trace(context, ready_side_trace) } {
                return action;
            }
            JitTraceAction::ContinueAt(resume_pc)
        }
        None => {
            if !redundant_recovery {
                unsafe { apply_deopt_recovery(context, &recovery) };
            }
            JitTraceAction::ContinueAt(resume_pc)
        }
    }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn dispatch_native_trace_result(
    context: &mut JitExecutionContext<'_>,
    target_pc: usize,
    summary: HelperPlanDispatchSummary,
    result: NativeTraceResult,
    loop_exit_pc: usize,
) -> Option<JitTraceAction> {
    match result.status {
        NativeTraceStatus::Fallback => {
            record_trace_hits_or_fallback(
                context.lua_state,
                context.ci.chunk_ptr,
                target_pc,
                result.hits,
                summary,
            );
            None
        }
        NativeTraceStatus::LoopExit => Some(unsafe {
            finish_trace_exit_in_context(context, target_pc, result.hits, summary, loop_exit_pc)
        }),
        NativeTraceStatus::SideExit => Some(unsafe {
            finish_trace_exit_by_index_in_context(
                context,
                target_pc,
                result.hits,
                summary,
                result.exit_index as u16,
                result.exit_pc as usize,
            )
        }),
        NativeTraceStatus::Returned => {
            super::record_batched_trace_execution(context.lua_state, result.hits, result.hits, summary);
            Some(JitTraceAction::Returned)
        }
    }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn dispatch_ready_side_trace(
    context: &mut JitExecutionContext<'_>,
    dispatch: ReadySideTraceDispatch,
) -> Option<JitTraceAction> {
    match dispatch {
        ReadySideTraceDispatch::Executable(dispatch) => {
            unsafe { dispatch_executable_trace(context, dispatch, None) }
        }
        ReadySideTraceDispatch::Native(dispatch) => unsafe {
            dispatch_native_compiled_trace(
            context,
            dispatch.start_pc as usize,
            dispatch.summary,
            dispatch.loop_tail_pc as usize,
            dispatch.native,
            )
        },
    }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn dispatch_native_ready_side_trace_fast(
    context: &mut JitExecutionContext<'_>,
    dispatch: NativeExecutableTraceDispatch,
) -> Option<JitTraceAction> {
    unsafe {
        dispatch_native_compiled_trace(
            context,
            dispatch.start_pc as usize,
            dispatch.summary,
            dispatch.loop_tail_pc as usize,
            dispatch.native,
        )
    }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn dispatch_native_compiled_trace(
    context: &mut JitExecutionContext<'_>,
    target_pc: usize,
    summary: HelperPlanDispatchSummary,
    exit_pc: usize,
    native: NativeCompiledTrace,
) -> Option<JitTraceAction> {
    let entry = match native {
        NativeCompiledTrace::Return { entry }
        | NativeCompiledTrace::Return0 { entry }
        | NativeCompiledTrace::Return1 { entry }
        | NativeCompiledTrace::LinearIntForLoop { entry }
        | NativeCompiledTrace::LinearIntJmpLoop { entry }
        | NativeCompiledTrace::NumericForLoop { entry }
        | NativeCompiledTrace::GuardedNumericForLoop { entry }
        | NativeCompiledTrace::TForLoop { entry }
        | NativeCompiledTrace::NumericJmpLoop { entry } => entry,
    };

    let mut result = NativeTraceResult::default();
    entry(
        context.lua_state.stack_mut().as_mut_ptr(),
        context.base,
        context.constants.as_ptr(),
        context.constants.len(),
        context.lua_state,
        context.ci.upvalue_ptrs,
        &mut result,
    );
    dispatch_native_trace_result(context, target_pc, summary, result, exit_pc)
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn dispatch_executable_trace(
    context: &mut JitExecutionContext<'_>,
    dispatch: ExecutableTraceDispatch,
    loop_exit_pc_override: Option<usize>,
) -> Option<JitTraceAction> {
    let target_pc = dispatch.start_pc as usize;
    let exit_pc = loop_exit_pc_override.unwrap_or(dispatch.loop_tail_pc as usize);
    let execution = dispatch.execution;
    let summary = dispatch.summary;

    match execution {
        CompiledTraceExecution::LoweredOnly => None,
        CompiledTraceExecution::Native(native) => unsafe {
            dispatch_native_compiled_trace(context, target_pc, summary, exit_pc, native)
        },
    }
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn apply_deopt_recovery(
    context: &mut JitExecutionContext<'_>,
    recovery: &super::lowering::DeoptRecovery,
) {
    let sp = context.lua_state.stack_mut().as_mut_ptr();

    for (reg, value) in &recovery.register_restores {
        unsafe {
            *sp.add(context.base + *reg as usize) = *value;
        }
    }

    for (start, values) in &recovery.register_range_restores {
        for (offset, value) in values.iter().enumerate() {
            unsafe {
                *sp.add(context.base + *start as usize + offset) = *value;
            }
        }
    }

    if !context.ci.upvalue_ptrs.is_null() {
        for (index, value) in &recovery.upvalue_restores {
            let upvalue_ptr = unsafe { *context.ci.upvalue_ptrs.add(*index as usize) };
            upvalue_ptr
                .as_mut_ref()
                .data
                .set_value_parts(value.value, value.tt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua_vm::{LuaVM, SafeOption};
    use crate::lua_vm::jit::backend::NativeLoweringProfile;
    use crate::lua_vm::jit::lowering::{DeoptRecovery, DeoptTarget, MaterializedSnapshot};

    unsafe extern "C" fn native_return0_test_entry(
        _stack: *mut LuaValue,
        _base: usize,
        _constants: *const LuaValue,
        _constant_count: usize,
        _lua_state: *mut LuaState,
        _upvalue_ptrs: *const crate::gc::UpvaluePtr,
        result: *mut NativeTraceResult,
    ) {
        unsafe {
            *result = NativeTraceResult::fallback(1);
        }
    }

    #[test]
    fn redundant_side_exit_with_native_child_uses_fast_dispatch_path() {
        let mut vm = LuaVM::new(SafeOption::default());
        let lua_state = vm.main_state();
        lua_state.set_top(1).unwrap();
        lua_state.stack_set(0, LuaValue::integer(7)).unwrap();

        let ci = CallInfo::default();
        let dispatch = TraceExitDispatch {
            recovery: DeoptRecovery {
                target: DeoptTarget {
                    exit_index: 0,
                    snapshot_id: 0,
                    resume_pc: 99,
                },
                snapshot: MaterializedSnapshot {
                    id: 0,
                    resume_pc: 99,
                    operands: vec![],
                    restore_operands: vec![],
                },
                register_restores: vec![(0, LuaValue::integer(7))],
                register_range_restores: Vec::new(),
                upvalue_restores: Vec::new(),
            },
            ready_side_trace: Some(ReadySideTraceDispatch::Native(NativeExecutableTraceDispatch {
                start_pc: 12,
                loop_tail_pc: 12,
                native: NativeCompiledTrace::Return0 {
                    entry: native_return0_test_entry,
                },
                summary: HelperPlanDispatchSummary::default(),
                profile: NativeLoweringProfile::default(),
            })),
        };

        let mut context = JitExecutionContext {
            lua_state,
            ci: &ci,
            base: 0,
            constants: &[],
        };

        let action = unsafe {
            continue_after_resolved_exit(&mut context, dispatch, ResolvedExitKind::SideExit)
        };
        assert_eq!(action, JitTraceAction::ContinueAt(99));

        let snapshot = context.lua_state.vm_mut().jit.stats_snapshot();
        assert_eq!(snapshot.counters.side_native_dispatches, 1);
        assert_eq!(snapshot.counters.native_redundant_side_exit_recoveries, 1);
        assert_eq!(snapshot.counters.native_redundant_side_exit_fast_dispatches, 1);
    }
}
