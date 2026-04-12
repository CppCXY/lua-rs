use crate::{CallInfo, LuaState, LuaValue};
use crate::gc::UpvaluePtr;
use crate::lua_value::LuaProto;
use crate::lua_vm::execute::{
    call::{call_c_function, precall},
    execute_loop::lua_execute,
    helper::{
        lua_fmod, lua_imod, objlen_value, pfltvalue, pivalue, ptonumberns, pttisfloat,
        pttisinteger, setivalue, setobj2s, setobjs2s,
    },
};

use super::{
    CompiledTraceExecution, ExecutableTraceDispatch, HelperPlanDispatchSummary,
    JitTraceAction, ReadySideTraceDispatch, TraceExitDispatch,
};
use super::backend::{NativeCompiledTrace, NativeTraceResult, NativeTraceStatus};
use super::ir::is_fused_arithmetic_metamethod_pair;
use super::state::NativeExecutableTraceDispatch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedExitKind {
    LoopExit,
    SideExit,
}

pub(crate) struct JitExecutionContext<'a> {
    lua_state: &'a mut LuaState,
    chunk_ptr: *const LuaProto,
    upvalue_ptrs: *const UpvaluePtr,
    base: usize,
    constants: &'a [LuaValue],
}

impl<'a> JitExecutionContext<'a> {
    #[allow(unsafe_op_in_unsafe_fn)]
    unsafe fn new(lua_state: &'a mut LuaState, ci: &'a CallInfo, base: usize) -> Self {
        let constants = unsafe { &(*ci.chunk_ptr).constants };
        Self {
            lua_state,
            chunk_ptr: ci.chunk_ptr,
            upvalue_ptrs: ci.upvalue_ptrs,
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

    let stack = context.lua_state.stack().as_ptr();
    let Some(dispatch) = (unsafe {
        context.lua_state.vm_mut().jit.resolve_trace_exit(
            context.chunk_ptr,
            target_pc as u32,
            exit_pc as u32,
            stack,
            context.base,
            context.constants,
            context.upvalue_ptrs,
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

    let stack = context.lua_state.stack().as_ptr();
    let Some(dispatch) = (unsafe {
        context.lua_state.vm_mut().jit.resolve_trace_exit_by_index(
            context.chunk_ptr,
            target_pc as u32,
            exit_index,
            stack,
            context.base,
            context.constants,
            context.upvalue_ptrs,
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
            context.upvalue_ptrs,
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
                context.chunk_ptr,
                target_pc,
                result.hits,
                summary,
            );
            None
        }
        NativeTraceStatus::LoopExit => {
            if result.exit_pc != 0 {
                // Interior guard exit — return directly to interpreter at the guard's
                // exit PC, bypassing side-trace resolution and dispatch entirely.
                super::record_batched_trace_execution(context.lua_state, result.hits, result.hits, summary);
                Some(JitTraceAction::ContinueAt(result.exit_pc as usize))
            } else {
                Some(unsafe {
                    finish_trace_exit_in_context(context, target_pc, result.hits, summary, loop_exit_pc)
                })
            }
        }
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
        | NativeCompiledTrace::GuardedCallPrefix { entry }
        | NativeCompiledTrace::CallForLoop { entry }
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
        context.upvalue_ptrs,
        &mut result,
    );
    dispatch_native_trace_result(context, target_pc, summary, result, exit_pc)
}

#[inline(always)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn dispatch_lowered_trace_snippet(
    context: &mut JitExecutionContext<'_>,
    target_pc: usize,
    summary: HelperPlanDispatchSummary,
) -> Option<JitTraceAction> {
    if context.chunk_ptr.is_null() || context.lua_state.hook_mask != 0 {
        return None;
    }

    let chunk = unsafe { context.chunk_ptr.as_ref() }?;
    let code = &chunk.code;

    let mut pc = target_pc;
    let mut completed_hits = 0u32;

    let record_completed_hits = |context: &mut JitExecutionContext<'_>, hits: u32| {
        if hits > 0 {
            super::record_batched_trace_execution(context.lua_state, hits, hits, summary);
        }
    };

    while pc < code.len() {
        let op_pc = pc;
        let instr = unsafe { *code.get_unchecked(pc) };
        let opcode = instr.get_opcode();
        pc += 1;

        match opcode {
            crate::OpCode::Move => {
                setobjs2s(
                    context.lua_state,
                    context.base + instr.get_a() as usize,
                    context.base + instr.get_b() as usize,
                );
            }
            crate::OpCode::AddI => {
                let dst = context.base + instr.get_a() as usize;
                let src = context.base + instr.get_b() as usize;
                let src_ptr = unsafe { context.lua_state.stack_mut().as_mut_ptr().add(src) } as *const LuaValue;
                let dst_ptr = unsafe { context.lua_state.stack_mut().as_mut_ptr().add(dst) };
                let imm = instr.get_sc() as i64;
                if unsafe { pttisinteger(src_ptr) } {
                    unsafe { *dst_ptr = LuaValue::integer(pivalue(src_ptr).wrapping_add(imm)) };
                } else if unsafe { pttisfloat(src_ptr) } {
                    unsafe { *dst_ptr = LuaValue::float(pfltvalue(src_ptr) + imm as f64) };
                } else {
                    let mut numeric = 0.0;
                    if unsafe { ptonumberns(src_ptr, &mut numeric) } {
                        unsafe { *dst_ptr = LuaValue::float(numeric + imm as f64) };
                    } else {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                }
            }
            crate::OpCode::Sub => {
                let dst = context.base + instr.get_a() as usize;
                let lhs = context.base + instr.get_b() as usize;
                let rhs = context.base + instr.get_c() as usize;
                let sp = context.lua_state.stack_mut().as_mut_ptr();
                let lhs_ptr = unsafe { sp.add(lhs) } as *const LuaValue;
                let rhs_ptr = unsafe { sp.add(rhs) } as *const LuaValue;
                let dst_ptr = unsafe { sp.add(dst) };
                if unsafe { pttisinteger(lhs_ptr) && pttisinteger(rhs_ptr) } {
                    unsafe {
                        *dst_ptr = LuaValue::integer(pivalue(lhs_ptr).wrapping_sub(pivalue(rhs_ptr)))
                    };
                } else if unsafe { pttisfloat(lhs_ptr) && pttisfloat(rhs_ptr) } {
                    unsafe { *dst_ptr = LuaValue::float(pfltvalue(lhs_ptr) - pfltvalue(rhs_ptr)) };
                } else {
                    let mut lhs_num = 0.0;
                    let mut rhs_num = 0.0;
                    if unsafe { ptonumberns(lhs_ptr, &mut lhs_num) && ptonumberns(rhs_ptr, &mut rhs_num) } {
                        unsafe { *dst_ptr = LuaValue::float(lhs_num - rhs_num) };
                    } else {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                }
            }
            crate::OpCode::LoadI => {
                let dst = context.base + instr.get_a() as usize;
                setivalue(
                    unsafe { context.lua_state.stack_mut().get_unchecked_mut(dst) },
                    instr.get_sbx() as i64,
                );
            }
            crate::OpCode::LoadK => {
                let value = unsafe { context.constants.get_unchecked(instr.get_bx() as usize) };
                setobj2s(context.lua_state, context.base + instr.get_a() as usize, value);
            }
            crate::OpCode::GetUpval => {
                if context.upvalue_ptrs.is_null() {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }

                let upvalue_ptr = unsafe { *context.upvalue_ptrs.add(instr.get_b() as usize) };
                let src = upvalue_ptr.as_ref().data.get_value_ref();
                let dst = unsafe {
                    context
                        .lua_state
                        .stack_mut()
                        .as_mut_ptr()
                        .add(context.base + instr.get_a() as usize)
                };
                unsafe { *dst = *src };
            }
            crate::OpCode::GetTabUp => {
                if context.upvalue_ptrs.is_null() {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }

                let upvalue_ptr = unsafe { *context.upvalue_ptrs.add(instr.get_b() as usize) };
                let upvalue = upvalue_ptr.as_ref().data.get_value_ref();
                let key = unsafe { context.constants.get_unchecked(instr.get_c() as usize) };
                if !key.is_short_string() || !upvalue.is_table() {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }

                let table = upvalue.hvalue();
                if !table.impl_table.has_hash() {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }

                let dest = unsafe {
                    context
                        .lua_state
                        .stack_mut()
                        .as_mut_ptr()
                        .add(context.base + instr.get_a() as usize)
                };
                if unsafe { !table.impl_table.get_shortstr_into(key, dest) } {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }
            }
            crate::OpCode::GetTable => {
                let table_reg = context.base + instr.get_b() as usize;
                let key_reg = context.base + instr.get_c() as usize;
                let table_ptr = unsafe { context.lua_state.stack().as_ptr().add(table_reg) };
                let key_ptr = unsafe { context.lua_state.stack().as_ptr().add(key_reg) };

                if unsafe { !(*table_ptr).is_table() || !pttisinteger(key_ptr) } {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }

                let table = unsafe { (*table_ptr).hvalue() };
                let key = unsafe { pivalue(key_ptr) };
                let dst = unsafe {
                    context
                        .lua_state
                        .stack_mut()
                        .as_mut_ptr()
                        .add(context.base + instr.get_a() as usize)
                };

                if unsafe { table.impl_table.fast_geti_into(key, dst) }
                    || unsafe { table.impl_table.get_int_from_hash_into(key, dst) }
                {
                    continue;
                }

                record_completed_hits(context, completed_hits);
                return Some(JitTraceAction::ContinueAt(op_pc));
            }
            crate::OpCode::Len => {
                let src = unsafe {
                    *context
                        .lua_state
                        .stack()
                        .get_unchecked(context.base + instr.get_b() as usize)
                };
                let value = match objlen_value(context.lua_state, src) {
                    Ok(value) => value,
                    Err(_) => {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                };
                setobj2s(context.lua_state, context.base + instr.get_a() as usize, &value);
            }
            crate::OpCode::Add => {
                let dst = context.base + instr.get_a() as usize;
                let lhs = context.base + instr.get_b() as usize;
                let rhs = context.base + instr.get_c() as usize;
                let sp = context.lua_state.stack_mut().as_mut_ptr();
                let lhs_ptr = unsafe { sp.add(lhs) } as *const LuaValue;
                let rhs_ptr = unsafe { sp.add(rhs) } as *const LuaValue;
                let dst_ptr = unsafe { sp.add(dst) };
                if unsafe { pttisinteger(lhs_ptr) && pttisinteger(rhs_ptr) } {
                    unsafe { *dst_ptr = LuaValue::integer(pivalue(lhs_ptr).wrapping_add(pivalue(rhs_ptr))) };
                } else if unsafe { pttisfloat(lhs_ptr) && pttisfloat(rhs_ptr) } {
                    unsafe { *dst_ptr = LuaValue::float(pfltvalue(lhs_ptr) + pfltvalue(rhs_ptr)) };
                } else {
                    let mut lhs_num = 0.0;
                    let mut rhs_num = 0.0;
                    if unsafe { ptonumberns(lhs_ptr, &mut lhs_num) && ptonumberns(rhs_ptr, &mut rhs_num) } {
                        unsafe { *dst_ptr = LuaValue::float(lhs_num + rhs_num) };
                    } else {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                }
            }
            crate::OpCode::ModK => {
                let dst = context.base + instr.get_a() as usize;
                let lhs = context.base + instr.get_b() as usize;
                let rhs = instr.get_c() as usize;
                let sp = context.lua_state.stack_mut().as_mut_ptr();
                let lhs_ptr = unsafe { sp.add(lhs) } as *const LuaValue;
                let rhs_ptr = unsafe { context.constants.as_ptr().add(rhs) };
                let dst_ptr = unsafe { sp.add(dst) };
                if unsafe { pttisinteger(lhs_ptr) && pttisinteger(rhs_ptr) } {
                    let divisor = unsafe { pivalue(rhs_ptr) };
                    if divisor == 0 {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                    unsafe { *dst_ptr = LuaValue::integer(lua_imod(pivalue(lhs_ptr), divisor)) };
                } else if unsafe { pttisfloat(lhs_ptr) && pttisfloat(rhs_ptr) } {
                    unsafe { *dst_ptr = LuaValue::float(lua_fmod(pfltvalue(lhs_ptr), pfltvalue(rhs_ptr))) };
                } else {
                    let mut lhs_num = 0.0;
                    let mut rhs_num = 0.0;
                    if unsafe { ptonumberns(lhs_ptr, &mut lhs_num) && ptonumberns(rhs_ptr, &mut rhs_num) } {
                        unsafe { *dst_ptr = LuaValue::float(lua_fmod(lhs_num, rhs_num)) };
                    } else {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                }
            }
            crate::OpCode::SetTable => {
                let table_reg = context.base + instr.get_a() as usize;
                let key_reg = context.base + instr.get_b() as usize;
                let table_ptr = unsafe { context.lua_state.stack().as_ptr().add(table_reg) };
                let key_ptr = unsafe { context.lua_state.stack().as_ptr().add(key_reg) };

                if unsafe { !(*table_ptr).is_table() || !pttisinteger(key_ptr) } {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }

                let table_value = unsafe { *table_ptr };
                let table = unsafe { (*table_ptr).hvalue_mut() };
                let key = unsafe { pivalue(key_ptr) };
                let meta = table.meta_ptr();
                if !(meta.is_null() || meta.as_mut_ref().data.no_tm(crate::lua_vm::TmKind::NewIndex.into())) {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }

                let value = if instr.get_k() {
                    unsafe { *context.constants.get_unchecked(instr.get_c() as usize) }
                } else {
                    unsafe {
                        *context
                            .lua_state
                            .stack()
                            .get_unchecked(context.base + instr.get_c() as usize)
                    }
                };

                if table.impl_table.fast_seti(key, value) {
                    if value.is_collectable() {
                        context
                            .lua_state
                            .gc_barrier_back(unsafe { table_value.as_gc_ptr_table_unchecked() });
                    }
                    continue;
                }

                let delta = table.impl_table.set_int_slow(key, value);
                if delta != 0 {
                    context
                        .lua_state
                        .gc_track_table_resize(unsafe { table_value.as_table_ptr_unchecked() }, delta);
                }
                if value.is_collectable() {
                    context
                        .lua_state
                        .gc_barrier_back(unsafe { table_value.as_gc_ptr_table_unchecked() });
                }
            }
            crate::OpCode::Lt => {
                let lhs = context.base + instr.get_a() as usize;
                let rhs = context.base + instr.get_b() as usize;
                let sp = context.lua_state.stack_mut().as_mut_ptr();
                let lhs_ptr = unsafe { sp.add(lhs) } as *const LuaValue;
                let rhs_ptr = unsafe { sp.add(rhs) } as *const LuaValue;
                let cond = if unsafe { pttisinteger(lhs_ptr) && pttisinteger(rhs_ptr) } {
                    unsafe { pivalue(lhs_ptr) < pivalue(rhs_ptr) }
                } else if unsafe { pttisfloat(lhs_ptr) && pttisfloat(rhs_ptr) } {
                    unsafe { pfltvalue(lhs_ptr) < pfltvalue(rhs_ptr) }
                } else {
                    let mut lhs_num = 0.0;
                    let mut rhs_num = 0.0;
                    if unsafe { ptonumberns(lhs_ptr, &mut lhs_num) && ptonumberns(rhs_ptr, &mut rhs_num) } {
                        lhs_num < rhs_num
                    } else {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                };

                if cond != instr.get_k() {
                    pc += 1;
                } else {
                    let jmp = unsafe { *code.get_unchecked(pc) };
                    let exit_pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                    completed_hits = completed_hits.saturating_add(1);
                    return Some(unsafe {
                        finish_trace_exit_in_context(
                            context,
                            target_pc,
                            completed_hits,
                            summary,
                            exit_pc,
                        )
                    });
                }
            }
            crate::OpCode::Le => {
                let lhs = context.base + instr.get_a() as usize;
                let rhs = context.base + instr.get_b() as usize;
                let sp = context.lua_state.stack_mut().as_mut_ptr();
                let lhs_ptr = unsafe { sp.add(lhs) } as *const LuaValue;
                let rhs_ptr = unsafe { sp.add(rhs) } as *const LuaValue;
                let cond = if unsafe { pttisinteger(lhs_ptr) && pttisinteger(rhs_ptr) } {
                    unsafe { pivalue(lhs_ptr) <= pivalue(rhs_ptr) }
                } else if unsafe { pttisfloat(lhs_ptr) && pttisfloat(rhs_ptr) } {
                    unsafe { pfltvalue(lhs_ptr) <= pfltvalue(rhs_ptr) }
                } else {
                    let mut lhs_num = 0.0;
                    let mut rhs_num = 0.0;
                    if unsafe { ptonumberns(lhs_ptr, &mut lhs_num) && ptonumberns(rhs_ptr, &mut rhs_num) } {
                        lhs_num <= rhs_num
                    } else {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                };

                if cond != instr.get_k() {
                    pc += 1;
                } else {
                    let jmp = unsafe { *code.get_unchecked(pc) };
                    let exit_pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                    completed_hits = completed_hits.saturating_add(1);
                    return Some(unsafe {
                        finish_trace_exit_in_context(
                            context,
                            target_pc,
                            completed_hits,
                            summary,
                            exit_pc,
                        )
                    });
                }
            }
            crate::OpCode::LeI => {
                let value_reg = context.base + instr.get_a() as usize;
                let value_ptr = unsafe {
                    context
                        .lua_state
                        .stack_mut()
                        .as_mut_ptr()
                        .add(value_reg)
                } as *const LuaValue;
                let imm = instr.get_sb() as i64;
                let cond = if unsafe { pttisinteger(value_ptr) } {
                    unsafe { pivalue(value_ptr) <= imm }
                } else if unsafe { pttisfloat(value_ptr) } {
                    unsafe { pfltvalue(value_ptr) <= imm as f64 }
                } else {
                    let mut numeric = 0.0;
                    if unsafe { ptonumberns(value_ptr, &mut numeric) } {
                        numeric <= imm as f64
                    } else {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                };

                if cond != instr.get_k() {
                    pc += 1;
                } else {
                    let jmp = unsafe { *code.get_unchecked(pc) };
                    let exit_pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                    completed_hits = completed_hits.saturating_add(1);
                    return Some(unsafe {
                        finish_trace_exit_in_context(
                            context,
                            target_pc,
                            completed_hits,
                            summary,
                            exit_pc,
                        )
                    });
                }
            }
            crate::OpCode::Test => {
                let value = unsafe {
                    *context
                        .lua_state
                        .stack()
                        .get_unchecked(context.base + instr.get_a() as usize)
                };
                let cond = !value.is_nil() && !value.ttisfalse();
                if cond != instr.get_k() {
                    pc += 1;
                } else {
                    let jmp = unsafe { *code.get_unchecked(pc) };
                    let exit_pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                    completed_hits = completed_hits.saturating_add(1);
                    return Some(unsafe {
                        finish_trace_exit_in_context(
                            context,
                            target_pc,
                            completed_hits,
                            summary,
                            exit_pc,
                        )
                    });
                }
            }
            crate::OpCode::Jmp => {
                let target = (pc as isize + instr.get_sj() as isize) as usize;
                if target <= op_pc {
                    completed_hits = completed_hits.saturating_add(1);
                }
                pc = target;
            }
            crate::OpCode::Call => {
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as i32;
                let func_idx = context.base + a;
                if func_idx >= context.lua_state.stack_len() {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }

                let nargs = if b != 0 {
                    let top = func_idx + b;
                    if top > context.lua_state.stack_len() {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                    context.lua_state.set_top_raw(top);
                    b - 1
                } else {
                    let top = context.lua_state.get_top();
                    if top <= func_idx {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                    top - func_idx - 1
                };

                let caller_depth = context.lua_state.call_depth();
                let Some(ci) = context.lua_state.current_frame_mut() else {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                };
                ci.save_pc(pc);

                let func = unsafe { *context.lua_state.stack().get_unchecked(func_idx) };
                if func.is_lua_function() {
                    let (param_count, max_stack_size, chunk_ptr, upvalue_ptrs) = {
                        let lua_func = unsafe { func.as_lua_function_unchecked() };
                        let chunk = lua_func.chunk();
                        (
                            chunk.param_count,
                            chunk.max_stack_size,
                            chunk as *const _,
                            lua_func.upvalues().as_ptr(),
                        )
                    };

                    let push_result = if nargs == param_count {
                        context.lua_state.try_push_lua_frame_exact(
                            func_idx + 1,
                            c - 1,
                            max_stack_size,
                            chunk_ptr,
                            upvalue_ptrs,
                        )
                    } else {
                        Ok(false)
                    };

                    match push_result {
                        Ok(true) => {}
                        Ok(false) => {
                            if context
                                .lua_state
                                .push_lua_frame(
                                    func_idx + 1,
                                    nargs,
                                    c - 1,
                                    param_count,
                                    max_stack_size,
                                    chunk_ptr,
                                    upvalue_ptrs,
                                )
                                .is_err()
                            {
                                record_completed_hits(context, completed_hits);
                                return Some(JitTraceAction::ContinueAt(op_pc));
                            }
                        }
                        Err(_) => {
                            record_completed_hits(context, completed_hits);
                            return Some(JitTraceAction::ContinueAt(op_pc));
                        }
                    }

                    if context.lua_state.inc_n_ccalls().is_err() {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                    let result = lua_execute(context.lua_state, caller_depth);
                    context.lua_state.dec_n_ccalls();
                    if result.is_err() {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                } else if func.is_c_callable() {
                    if call_c_function(context.lua_state, func_idx, nargs, c - 1).is_err() {
                        record_completed_hits(context, completed_hits);
                        return Some(JitTraceAction::ContinueAt(op_pc));
                    }
                } else {
                    match precall(context.lua_state, func_idx, nargs, c - 1) {
                        Ok(true) => {
                            if context.lua_state.inc_n_ccalls().is_err() {
                                record_completed_hits(context, completed_hits);
                                return Some(JitTraceAction::ContinueAt(op_pc));
                            }
                            let result = lua_execute(context.lua_state, caller_depth);
                            context.lua_state.dec_n_ccalls();
                            if result.is_err() {
                                record_completed_hits(context, completed_hits);
                                return Some(JitTraceAction::ContinueAt(op_pc));
                            }
                        }
                        Ok(false) => {}
                        Err(_) => {
                            record_completed_hits(context, completed_hits);
                            return Some(JitTraceAction::ContinueAt(op_pc));
                        }
                    }
                }
            }
            crate::OpCode::ForLoop => {
                let a = instr.get_a() as usize;
                let bx = instr.get_bx() as usize;
                let ra = unsafe { context.lua_state.stack_mut().as_mut_ptr().add(context.base + a) };
                if unsafe { !pttisinteger(ra.add(1)) } {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(op_pc));
                }
                let count = unsafe { pivalue(ra) as u64 };
                completed_hits = completed_hits.saturating_add(1);
                if count > 0 {
                    let step = unsafe { pivalue(ra.add(1)) };
                    let idx = unsafe { pivalue(ra.add(2)) };
                    unsafe {
                        (*ra).value.i = count as i64 - 1;
                        (*ra.add(2)).value.i = idx.wrapping_add(step);
                    }
                    pc = pc.saturating_sub(bx);
                    continue;
                } else {
                    record_completed_hits(context, completed_hits);
                    return Some(JitTraceAction::ContinueAt(pc));
                }
            }
            crate::OpCode::Return0 => {
                completed_hits = completed_hits.saturating_add(1);
                record_completed_hits(context, completed_hits);
                return Some(JitTraceAction::ContinueAt(op_pc));
            }
            _ => {
                record_completed_hits(context, completed_hits);
                return Some(JitTraceAction::ContinueAt(op_pc));
            }
        }

        if pc < code.len() {
            let next = unsafe { *code.get_unchecked(pc) };
            if is_fused_arithmetic_metamethod_pair(opcode, instr, next.get_opcode(), next) {
                pc += 1;
            }
        }
    }

    record_completed_hits(context, completed_hits);
    Some(JitTraceAction::ContinueAt(pc))
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
        CompiledTraceExecution::LoweredSnippet => unsafe {
            dispatch_lowered_trace_snippet(context, target_pc, summary)
        },
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

    if !context.upvalue_ptrs.is_null() {
        for (index, value) in &recovery.upvalue_restores {
            let upvalue_ptr = unsafe { *context.upvalue_ptrs.add(*index as usize) };
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
    use crate::{Instruction, OpCode};
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
            chunk_ptr: ci.chunk_ptr,
            upvalue_ptrs: ci.upvalue_ptrs,
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

    #[test]
    fn lowered_trace_snippet_runs_multiple_iterations_before_loop_exit() {
        let mut vm = LuaVM::new(SafeOption::default());
        let lua_state = vm.main_state();
        lua_state.set_top(4).unwrap();
        lua_state.stack_set(0, LuaValue::integer(2)).unwrap();
        lua_state.stack_set(1, LuaValue::integer(1)).unwrap();
        lua_state.stack_set(2, LuaValue::integer(10)).unwrap();
        lua_state.stack_set(3, LuaValue::integer(0)).unwrap();

        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Add, 3, 3, 2));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));

        let mut context = JitExecutionContext {
            lua_state,
            chunk_ptr: &chunk as *const LuaProto,
            upvalue_ptrs: std::ptr::null(),
            base: 0,
            constants: &[],
        };

        let action = unsafe {
            dispatch_lowered_trace_snippet(
                &mut context,
                0,
                HelperPlanDispatchSummary {
                    steps_executed: 2,
                    guards_observed: 0,
                    call_steps: 0,
                    metamethod_steps: 0,
                },
            )
        };

        assert_eq!(action, Some(JitTraceAction::ContinueAt(2)));
        assert_eq!(context.lua_state.stack_get(0).unwrap().as_integer(), Some(0));
        assert_eq!(context.lua_state.stack_get(2).unwrap().as_integer(), Some(12));
        assert_eq!(context.lua_state.stack_get(3).unwrap().as_integer(), Some(33));

        let snapshot = context.lua_state.vm_mut().jit.stats_snapshot();
        assert_eq!(snapshot.counters.trace_enter_hits, 3);
        assert_eq!(snapshot.counters.helper_plan_dispatches, 3);
        assert_eq!(snapshot.counters.helper_plan_steps, 6);
    }

}
