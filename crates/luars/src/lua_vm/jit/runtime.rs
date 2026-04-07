#[cfg(feature = "jit")]
use crate::{CallInfo, LuaState, LuaValue};
#[cfg(feature = "jit")]
use crate::lua_value::LuaProto;
#[cfg(feature = "jit")]
use super::executors::{
    jit_execute_generic_for_builtin_add, jit_execute_linear_int_for_loop,
    jit_execute_linear_int_jmp_loop, jit_execute_next_while_builtin_add,
    jit_execute_numeric_for_loop, jit_execute_numeric_ifelse_for_loop,
    jit_execute_numeric_jmp_loop, jit_execute_numeric_table_scan_jmp_loop,
    jit_execute_numeric_table_shift_jmp_loop, jit_execute_return, jit_execute_return0,
    jit_execute_return1, jit_execute_guarded_numeric_for_loop,
};

#[cfg(feature = "jit")]
use super::{
    CompiledTraceExecution, CompiledTraceExecutor, ExecutableTraceDispatch,
    HelperPlanDispatchSummary, JitTraceAction, TraceExitDispatch,
};
#[cfg(feature = "jit")]
use super::backend::NativeCompiledTrace;

#[cfg(feature = "jit")]
pub(crate) struct JitExecutionContext<'a> {
    lua_state: &'a mut LuaState,
    ci: &'a CallInfo,
    base: usize,
    constants: &'a [LuaValue],
}

#[cfg(feature = "jit")]
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

#[cfg(feature = "jit")]
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

#[cfg(feature = "jit")]
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

    let mut context = unsafe { JitExecutionContext::new(lua_state, ci, base) };
    unsafe { dispatch_executable_trace(&mut context, dispatch, loop_exit_pc_override) }
}

#[cfg(feature = "jit")]
#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
pub(crate) unsafe fn finish_trace_exit(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    trace_hits: u32,
    summary: HelperPlanDispatchSummary,
    exit_pc: usize,
) -> JitTraceAction {
    super::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);

    let mut context = unsafe { JitExecutionContext::new(lua_state, ci, base) };
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

    unsafe { apply_deopt_recovery(&mut context, &dispatch) };

    if let Some(side_trace) = dispatch.side_trace
        && let Some(action) = (unsafe {
            dispatch_executable_trace(&mut context, side_trace, None)
        })
    {
        return action;
    }

    JitTraceAction::ContinueAt(dispatch.recovery.target.resume_pc as usize)
}

#[cfg(feature = "jit")]
#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn dispatch_native_trace_result(
    context: &mut JitExecutionContext<'_>,
    target_pc: usize,
    summary: HelperPlanDispatchSummary,
    encoded: u64,
    exit_pc: usize,
) -> Option<JitTraceAction> {
    let trace_hits = (encoded >> 1) as u32;

    if (encoded & 1) != 0 {
        Some(finish_trace_exit(
            context.lua_state,
            context.ci,
            context.base,
            target_pc,
            trace_hits,
            summary,
            exit_pc,
        ))
    } else {
        record_trace_hits_or_fallback(
            context.lua_state,
            context.ci.chunk_ptr,
            target_pc,
            trace_hits,
            summary,
        );
        None
    }
}

#[cfg(feature = "jit")]
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
        CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntForLoop { entry }) => unsafe {
            let encoded = entry(context.lua_state.stack_mut().as_mut_ptr(), context.base);
            dispatch_native_trace_result(context, target_pc, summary, encoded, exit_pc)
        },
        CompiledTraceExecution::Native(NativeCompiledTrace::LinearIntJmpLoop { entry, exit_pc }) => unsafe {
            let encoded = entry(context.lua_state.stack_mut().as_mut_ptr(), context.base);
            dispatch_native_trace_result(context, target_pc, summary, encoded, exit_pc as usize)
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::Return {
            start_reg,
            result_count,
        }) => Some(jit_execute_return(
            context.lua_state,
            context.ci,
            context.base,
            start_reg,
            result_count,
            summary,
        )),
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::Return0) => {
            Some(jit_execute_return0(context.lua_state, context.ci, summary))
        }
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::Return1 { src_reg }) => {
            Some(jit_execute_return1(
                context.lua_state,
                context.ci,
                context.base,
                src_reg,
                summary,
            ))
        }
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::LinearIntJmpLoop { steps, guard }) => unsafe {
            jit_execute_linear_int_jmp_loop(
                context.lua_state,
                context.ci,
                context.base,
                target_pc,
                &steps,
                guard,
                summary,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::NumericTableScanJmpLoop {
            table_reg,
            index_reg,
            limit_reg,
            step_imm,
            compare_op,
            exit_pc,
        }) => unsafe {
            jit_execute_numeric_table_scan_jmp_loop(
                context.lua_state,
                context.ci,
                context.base,
                target_pc,
                table_reg,
                index_reg,
                limit_reg,
                step_imm,
                compare_op,
                summary,
                exit_pc as usize,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::NumericTableShiftJmpLoop {
            table_reg,
            index_reg,
            left_bound_reg,
            value_reg,
            temp_reg,
            exit_pc,
        }) => unsafe {
            jit_execute_numeric_table_shift_jmp_loop(
                context.lua_state,
                context.ci,
                context.base,
                target_pc,
                table_reg,
                index_reg,
                left_bound_reg,
                value_reg,
                temp_reg,
                summary,
                exit_pc as usize,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::NumericJmpLoop {
            pre_steps,
            steps,
            guard,
        }) => unsafe {
            jit_execute_numeric_jmp_loop(
                context.lua_state,
                context.ci,
                context.base,
                context.constants,
                target_pc,
                &pre_steps,
                &steps,
                guard,
                summary,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::LinearIntForLoop { loop_reg, steps }) => unsafe {
            jit_execute_linear_int_for_loop(
                context.lua_state,
                context.ci,
                context.base,
                target_pc,
                loop_reg,
                &steps,
                summary,
                exit_pc,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::NumericForLoop { loop_reg, steps }) => unsafe {
            jit_execute_numeric_for_loop(
                context.lua_state,
                context.ci,
                context.base,
                context.constants,
                target_pc,
                loop_reg,
                &steps,
                summary,
                exit_pc,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::GuardedNumericForLoop { loop_reg, steps, guard }) => unsafe {
            jit_execute_guarded_numeric_for_loop(
                context.lua_state,
                context.ci,
                context.base,
                context.constants,
                target_pc,
                loop_reg,
                &steps,
                guard,
                summary,
                exit_pc,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::NumericIfElseForLoop {
            loop_reg,
            pre_steps,
            cond,
            then_preset,
            else_preset,
            then_steps,
            else_steps,
            then_on_true,
        }) => unsafe {
            jit_execute_numeric_ifelse_for_loop(
                context.lua_state,
                context.ci,
                context.base,
                context.constants,
                target_pc,
                loop_reg,
                &pre_steps,
                cond,
                then_preset,
                else_preset,
                &then_steps,
                &else_steps,
                then_on_true,
                summary,
                exit_pc,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::NextWhileBuiltinAdd {
            key_reg,
            value_reg,
            acc_reg,
            table_reg,
            env_upvalue,
            key_const,
        }) => unsafe {
            jit_execute_next_while_builtin_add(
                context.lua_state,
                context.ci,
                context.base,
                context.constants,
                target_pc,
                key_reg,
                value_reg,
                acc_reg,
                table_reg,
                env_upvalue,
                key_const,
                summary,
            )
        },
        CompiledTraceExecution::Interpreter(CompiledTraceExecutor::GenericForBuiltinAdd {
            tfor_reg,
            value_reg,
            acc_reg,
        }) => unsafe {
            jit_execute_generic_for_builtin_add(
                context.lua_state,
                context.ci,
                context.base,
                target_pc,
                tfor_reg,
                value_reg,
                acc_reg,
                summary,
                exit_pc,
            )
        },
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn apply_deopt_recovery(
    context: &mut JitExecutionContext<'_>,
    dispatch: &TraceExitDispatch,
) {
    let sp = context.lua_state.stack_mut().as_mut_ptr();

    for (reg, value) in &dispatch.recovery.register_restores {
        unsafe {
            *sp.add(context.base + *reg as usize) = *value;
        }
    }

    for (start, values) in &dispatch.recovery.register_range_restores {
        for (offset, value) in values.iter().enumerate() {
            unsafe {
                *sp.add(context.base + *start as usize + offset) = *value;
            }
        }
    }

    if !context.ci.upvalue_ptrs.is_null() {
        for (index, value) in &dispatch.recovery.upvalue_restores {
            let upvalue_ptr = unsafe { *context.ci.upvalue_ptrs.add(*index as usize) };
            upvalue_ptr
                .as_mut_ref()
                .data
                .set_value_parts(value.value, value.tt);
        }
    }
}
