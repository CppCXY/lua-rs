#![allow(unsafe_op_in_unsafe_fn)]

#[cfg(feature = "jit")]
use crate::{CallInfo, LuaState, LuaValue};
#[cfg(feature = "jit")]
use crate::lua_vm::{
    TmKind,
    execute::{
        helper::{
            lua_fmod, lua_idiv, lua_imod, lua_shiftl, lua_shiftr, luai_numpow, pivalue,
            psetfltvalue, psetivalue, pttisinteger, setbfvalue, setbtvalue, setnilvalue,
            ttisfloat, ttisinteger,
        },
        number::{le_num, lt_num},
    },
};
#[cfg(feature = "jit")]
use crate::stdlib::basic::{ipairs_next, lua_next};

#[cfg(feature = "jit")]
use crate::lua_vm::jit as jit;
#[cfg(feature = "jit")]
use super::{finish_trace_exit, record_trace_hits_or_fallback, JitTraceAction};

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_finish_fixed_results(
    lua_state: &mut LuaState,
    res: usize,
    first_result: usize,
    nres: usize,
    wanted: i32,
) {
    match wanted {
        0 => lua_state.set_top_raw(res),
        1 => {
            unsafe {
                let sp = lua_state.stack_mut().as_mut_ptr();
                *sp.add(res) = if nres == 0 {
                    LuaValue::nil()
                } else {
                    *sp.add(first_result)
                };
            }
            lua_state.set_top_raw(res + 1);
        }
        -1 => {
            unsafe {
                let sp = lua_state.stack_mut().as_mut_ptr();
                if nres != 0 && res != first_result {
                    std::ptr::copy(sp.add(first_result), sp.add(res), nres);
                }
            }
            lua_state.set_top_raw(res + nres);
        }
        wanted => {
            let wanted = wanted as usize;
            let copy_count = nres.min(wanted);
            unsafe {
                let sp = lua_state.stack_mut().as_mut_ptr();
                if copy_count != 0 && res != first_result {
                    std::ptr::copy(sp.add(first_result), sp.add(res), copy_count);
                }
                for offset in copy_count..wanted {
                    *sp.add(res + offset) = LuaValue::nil();
                }
            }
            lua_state.set_top_raw(res + wanted);
        }
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_finish_return_results(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    first_result: usize,
    nres: usize,
) -> JitTraceAction {
    let res = ci.base - ci.func_offset as usize;
    let wanted = ci.nresults();
    jit_finish_fixed_results(lua_state, res, first_result, nres, wanted);
    lua_state.pop_call_frame();
    JitTraceAction::Returned
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn lua_next_into(
    table: &crate::LuaTable,
    current_key: &LuaValue,
    key_out: *mut LuaValue,
    value_out: *mut LuaValue,
) -> Result<bool, ()> {
    if let Some(index) = current_key.as_integer() {
        let next_index = index.wrapping_add(1);
        if unsafe { table.impl_table.fast_geti_into(next_index, value_out) } {
            unsafe {
                psetivalue(key_out, next_index);
            }
            return Ok(true);
        }
    }

    unsafe { table.next_into(current_key, key_out, value_out) }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_trace_fallback(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    target_pc: usize,
    trace_hits: u32,
    summary: jit::HelperPlanDispatchSummary,
) -> Option<JitTraceAction> {
    record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
    None
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_trace_complete_without_exit(
    lua_state: &mut LuaState,
    trace_hits: u32,
    summary: jit::HelperPlanDispatchSummary,
) -> Option<JitTraceAction> {
    jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
    None
}

#[cfg(feature = "jit")]
#[inline(always)]
unsafe fn jit_trace_exit(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    trace_hits: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<JitTraceAction> {
    Some(finish_trace_exit(
        lua_state,
        ci,
        base,
        target_pc,
        trace_hits,
        summary,
        exit_pc,
    ))
}

#[cfg(feature = "jit")]
#[inline(always)]
unsafe fn jit_execute_single_numeric_step(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    sp: *mut LuaValue,
    base: usize,
    constants: &[LuaValue],
    step: &jit::NumericStep,
) -> bool {
    unsafe { execute_numeric_steps(lua_state, ci, sp, base, constants, std::slice::from_ref(step)) }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn execute_linear_int_steps(
    sp: *mut LuaValue,
    base: usize,
    steps: &[jit::LinearIntStep],
) -> bool {
    for step in steps {
        match *step {
            jit::LinearIntStep::Move { dst, src } => {
                let src_ptr = unsafe { sp.add(base + src as usize) };
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                if unsafe { !pttisinteger(src_ptr as *const LuaValue) } {
                    return false;
                }
                unsafe { psetivalue(dst_ptr, pivalue(src_ptr as *const LuaValue)) };
            }
            jit::LinearIntStep::LoadI { dst, imm } => {
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                unsafe { psetivalue(dst_ptr, imm as i64) };
            }
            jit::LinearIntStep::Add { dst, lhs, rhs } => {
                let lhs_ptr = unsafe { sp.add(base + lhs as usize) };
                let rhs_ptr = unsafe { sp.add(base + rhs as usize) };
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                if unsafe {
                    !pttisinteger(lhs_ptr as *const LuaValue)
                        || !pttisinteger(rhs_ptr as *const LuaValue)
                } {
                    return false;
                }
                unsafe {
                    psetivalue(
                        dst_ptr,
                        pivalue(lhs_ptr as *const LuaValue)
                            .wrapping_add(pivalue(rhs_ptr as *const LuaValue)),
                    )
                };
            }
            jit::LinearIntStep::AddI { dst, src, imm } => {
                let src_ptr = unsafe { sp.add(base + src as usize) };
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                if unsafe { !pttisinteger(src_ptr as *const LuaValue) } {
                    return false;
                }
                unsafe {
                    psetivalue(
                        dst_ptr,
                        pivalue(src_ptr as *const LuaValue).wrapping_add(imm as i64),
                    )
                };
            }
            jit::LinearIntStep::Sub { dst, lhs, rhs } => {
                let lhs_ptr = unsafe { sp.add(base + lhs as usize) };
                let rhs_ptr = unsafe { sp.add(base + rhs as usize) };
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                if unsafe {
                    !pttisinteger(lhs_ptr as *const LuaValue)
                        || !pttisinteger(rhs_ptr as *const LuaValue)
                } {
                    return false;
                }
                unsafe {
                    psetivalue(
                        dst_ptr,
                        pivalue(lhs_ptr as *const LuaValue)
                            .wrapping_sub(pivalue(rhs_ptr as *const LuaValue)),
                    )
                };
            }
            jit::LinearIntStep::Mul { dst, lhs, rhs } => {
                let lhs_ptr = unsafe { sp.add(base + lhs as usize) };
                let rhs_ptr = unsafe { sp.add(base + rhs as usize) };
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                if unsafe {
                    !pttisinteger(lhs_ptr as *const LuaValue)
                        || !pttisinteger(rhs_ptr as *const LuaValue)
                } {
                    return false;
                }
                unsafe {
                    psetivalue(
                        dst_ptr,
                        pivalue(lhs_ptr as *const LuaValue)
                            .wrapping_mul(pivalue(rhs_ptr as *const LuaValue)),
                    )
                };
            }
        }
    }

    true
}

#[cfg(feature = "jit")]
#[inline(always)]
unsafe fn read_numeric_operand(
    sp: *mut LuaValue,
    base: usize,
    constants: &[LuaValue],
    operand: jit::NumericOperand,
) -> Option<LuaValue> {
    match operand {
        jit::NumericOperand::Reg(reg) => Some(unsafe { *sp.add(base + reg as usize) }),
        jit::NumericOperand::ImmI(imm) => Some(LuaValue::integer(imm as i64)),
        jit::NumericOperand::Const(index) => constants.get(index as usize).copied(),
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn numeric_binary_result(lhs: LuaValue, rhs: LuaValue, op: jit::NumericBinaryOp) -> Option<LuaValue> {
    if matches!(op, jit::NumericBinaryOp::Div | jit::NumericBinaryOp::Pow) {
        let lhs_num = lhs.as_float()?;
        let rhs_num = rhs.as_float()?;
        let value = match op {
            jit::NumericBinaryOp::Div => lhs_num / rhs_num,
            jit::NumericBinaryOp::Pow => luai_numpow(lhs_num, rhs_num),
            _ => unreachable!(),
        };
        return Some(LuaValue::float(value));
    }

    if matches!(op, jit::NumericBinaryOp::IDiv | jit::NumericBinaryOp::Mod) {
        if let (Some(lhs_int), Some(rhs_int)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
            if rhs_int == 0 {
                return None;
            }

            let value = match op {
                jit::NumericBinaryOp::IDiv => LuaValue::integer(lua_idiv(lhs_int, rhs_int)),
                jit::NumericBinaryOp::Mod => LuaValue::integer(lua_imod(lhs_int, rhs_int)),
                _ => unreachable!(),
            };
            return Some(value);
        }

        let lhs_num = lhs.as_float()?;
        let rhs_num = rhs.as_float()?;
        if rhs_num == 0.0 {
            return None;
        }

        let value = match op {
            jit::NumericBinaryOp::IDiv => LuaValue::float((lhs_num / rhs_num).floor()),
            jit::NumericBinaryOp::Mod => LuaValue::float(lua_fmod(lhs_num, rhs_num)),
            _ => unreachable!(),
        };
        return Some(value);
    }

    if matches!(
        op,
        jit::NumericBinaryOp::BAnd
            | jit::NumericBinaryOp::BOr
            | jit::NumericBinaryOp::BXor
            | jit::NumericBinaryOp::Shl
            | jit::NumericBinaryOp::Shr
    ) {
        let lhs_int = lhs.as_integer_strict()?;
        let rhs_int = rhs.as_integer_strict()?;
        let value = match op {
            jit::NumericBinaryOp::BAnd => lhs_int & rhs_int,
            jit::NumericBinaryOp::BOr => lhs_int | rhs_int,
            jit::NumericBinaryOp::BXor => lhs_int ^ rhs_int,
            jit::NumericBinaryOp::Shl => lua_shiftl(lhs_int, rhs_int),
            jit::NumericBinaryOp::Shr => lua_shiftr(lhs_int, rhs_int),
            _ => unreachable!(),
        };
        return Some(LuaValue::integer(value));
    }

    match (lhs.as_integer_strict(), rhs.as_integer_strict()) {
        (Some(lhs_int), Some(rhs_int)) => {
            let value = match op {
                jit::NumericBinaryOp::Add => lhs_int.wrapping_add(rhs_int),
                jit::NumericBinaryOp::Sub => lhs_int.wrapping_sub(rhs_int),
                jit::NumericBinaryOp::Mul => lhs_int.wrapping_mul(rhs_int),
                _ => unreachable!(),
            };
            Some(LuaValue::integer(value))
        }
        _ => {
            let lhs_num = lhs.as_float()?;
            let rhs_num = rhs.as_float()?;
            let value = match op {
                jit::NumericBinaryOp::Add => lhs_num + rhs_num,
                jit::NumericBinaryOp::Sub => lhs_num - rhs_num,
                jit::NumericBinaryOp::Mul => lhs_num * rhs_num,
                _ => unreachable!(),
            };
            Some(LuaValue::float(value))
        }
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn linear_int_compare(lhs: i64, rhs: i64, op: jit::LinearIntGuardOp) -> bool {
    match op {
        jit::LinearIntGuardOp::Eq => lhs == rhs,
        jit::LinearIntGuardOp::Lt => lhs < rhs,
        jit::LinearIntGuardOp::Le => lhs <= rhs,
        jit::LinearIntGuardOp::Gt => lhs > rhs,
        jit::LinearIntGuardOp::Ge => lhs >= rhs,
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn numeric_ifelse_cond_holds(
    sp: *mut LuaValue,
    base: usize,
    cond: jit::NumericIfElseCond,
) -> bool {
    match cond {
        jit::NumericIfElseCond::RegCompare { op, lhs, rhs } => {
            let lhs_ptr = unsafe { sp.add(base + lhs as usize) };
            let rhs_ptr = unsafe { sp.add(base + rhs as usize) };
            let lhs_value = unsafe { &*lhs_ptr };
            let rhs_value = unsafe { &*rhs_ptr };
            if !(ttisinteger(lhs_value) || ttisfloat(lhs_value))
                || !(ttisinteger(rhs_value) || ttisfloat(rhs_value))
            {
                return false;
            }

            match op {
                jit::LinearIntGuardOp::Lt => lt_num(lhs_value, rhs_value),
                jit::LinearIntGuardOp::Le => le_num(lhs_value, rhs_value),
                jit::LinearIntGuardOp::Gt => lt_num(rhs_value, lhs_value),
                jit::LinearIntGuardOp::Ge => le_num(rhs_value, lhs_value),
                jit::LinearIntGuardOp::Eq => {
                    if ttisinteger(lhs_value) && ttisinteger(rhs_value) {
                        lhs_value.ivalue() == rhs_value.ivalue()
                    } else if ttisfloat(lhs_value) && ttisfloat(rhs_value) {
                        lhs_value.fltvalue() == rhs_value.fltvalue()
                    } else if ttisinteger(lhs_value) && ttisfloat(rhs_value) {
                        lhs_value.ivalue() as f64 == rhs_value.fltvalue()
                    } else if ttisfloat(lhs_value) && ttisinteger(rhs_value) {
                        lhs_value.fltvalue() == rhs_value.ivalue() as f64
                    } else {
                        false
                    }
                }
            }
        }
        jit::NumericIfElseCond::IntCompare { op, reg, imm } => {
            let cond_ptr = unsafe { sp.add(base + reg as usize) };
            if unsafe { !pttisinteger(cond_ptr as *const LuaValue) } {
                return false;
            }
            let cond_value = unsafe { pivalue(cond_ptr as *const LuaValue) };
            linear_int_compare(cond_value, imm as i64, op)
        }
        jit::NumericIfElseCond::Truthy { reg } => {
            let cond_ptr = unsafe { sp.add(base + reg as usize) };
            let cond_value = unsafe { *cond_ptr };
            !cond_value.is_nil() && !cond_value.ttisfalse()
        }
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn execute_numeric_steps(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    sp: *mut LuaValue,
    base: usize,
    constants: &[LuaValue],
    steps: &[jit::NumericStep],
) -> bool {
    for step in steps {
        match *step {
            jit::NumericStep::Move { dst, src } => {
                let src_ptr = unsafe { sp.add(base + src as usize) };
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                unsafe { *dst_ptr = *src_ptr };
            }
            jit::NumericStep::GetUpval { dst, upvalue } => {
                if ci.upvalue_ptrs.is_null() {
                    return false;
                }
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(upvalue as usize) };
                let src = upvalue_ptr.as_ref().data.get_value_ref();
                unsafe {
                    (*dst_ptr).value = src.value;
                    (*dst_ptr).tt = src.tt;
                }
            }
            jit::NumericStep::SetUpval { src, upvalue } => {
                if ci.upvalue_ptrs.is_null() {
                    return false;
                }
                let src_ptr = unsafe { sp.add(base + src as usize) };
                let value = unsafe { *src_ptr };
                let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(upvalue as usize) };
                upvalue_ptr
                    .as_mut_ref()
                    .data
                    .set_value_parts(value.value, value.tt);
                if value.tt & 0x40 != 0
                    && let Some(gc_ptr) = value.as_gc_ptr()
                {
                    lua_state.gc_barrier(upvalue_ptr, gc_ptr);
                }
            }
            jit::NumericStep::GetTableInt { dst, table, index } => {
                let table_ptr = unsafe { sp.add(base + table as usize) };
                let index_ptr = unsafe { sp.add(base + index as usize) };
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                if unsafe { !(*table_ptr).is_table() || !pttisinteger(index_ptr as *const LuaValue) } {
                    return false;
                }

                let table = unsafe { (*table_ptr).hvalue() };
                let idx = unsafe { pivalue(index_ptr as *const LuaValue) };
                let loaded = unsafe { table.impl_table.fast_geti_into(idx, dst_ptr) }
                    || unsafe { table.impl_table.get_int_from_hash_into(idx, dst_ptr) };
                if !loaded {
                    return false;
                }
                let loaded_value = unsafe { &*dst_ptr };
                if !(ttisinteger(loaded_value) || ttisfloat(loaded_value)) {
                    return false;
                }
            }
            jit::NumericStep::SetTableInt { table, index, value } => {
                let table_ptr = unsafe { sp.add(base + table as usize) };
                let index_ptr = unsafe { sp.add(base + index as usize) };
                let value_ptr = unsafe { sp.add(base + value as usize) };
                if unsafe { !(*table_ptr).is_table() || !pttisinteger(index_ptr as *const LuaValue) } {
                    return false;
                }

                let table = unsafe { (*table_ptr).hvalue_mut() };
                let idx = unsafe { pivalue(index_ptr as *const LuaValue) };
                let value = unsafe { *value_ptr };
                if !table.impl_table.fast_seti_parts(idx, value.value, value.tt) {
                    return false;
                }
                if value.tt & 0x40 != 0 {
                    unsafe { lua_state.gc_barrier_back((*table_ptr).as_gc_ptr_table_unchecked()) };
                }
            }
            jit::NumericStep::LoadBool { dst, value } => {
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                unsafe {
                    if value {
                        setbtvalue(&mut *dst_ptr);
                    } else {
                        setbfvalue(&mut *dst_ptr);
                    }
                };
            }
            jit::NumericStep::LoadI { dst, imm } => {
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                unsafe { psetivalue(dst_ptr, imm as i64) };
            }
            jit::NumericStep::LoadF { dst, imm } => {
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                unsafe { psetfltvalue(dst_ptr, imm as f64) };
            }
            jit::NumericStep::Binary { dst, lhs, rhs, op } => {
                let Some(lhs_value) = (unsafe { read_numeric_operand(sp, base, constants, lhs) }) else {
                    return false;
                };
                let Some(rhs_value) = (unsafe { read_numeric_operand(sp, base, constants, rhs) }) else {
                    return false;
                };
                let Some(result) = numeric_binary_result(lhs_value, rhs_value, op) else {
                    return false;
                };

                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                unsafe { *dst_ptr = result };
            }
        }
    }

    true
}

#[cfg(feature = "jit")]
#[inline(always)]
fn linear_int_guard_holds(sp: *mut LuaValue, base: usize, guard: jit::LinearIntLoopGuard) -> bool {
    let (op, lhs_val, rhs_val, continue_when) = match guard {
        jit::LinearIntLoopGuard::HeadRegReg { op, lhs, rhs, continue_when, .. }
        | jit::LinearIntLoopGuard::TailRegReg { op, lhs, rhs, continue_when, .. } => {
            let lhs_ptr = unsafe { sp.add(base + lhs as usize) };
            let rhs_ptr = unsafe { sp.add(base + rhs as usize) };
            if unsafe {
                !pttisinteger(lhs_ptr as *const LuaValue) || !pttisinteger(rhs_ptr as *const LuaValue)
            } {
                return false;
            }
            (
                op,
                unsafe { pivalue(lhs_ptr as *const LuaValue) },
                unsafe { pivalue(rhs_ptr as *const LuaValue) },
                continue_when,
            )
        }
        jit::LinearIntLoopGuard::HeadRegImm { op, reg, imm, continue_when, .. }
        | jit::LinearIntLoopGuard::TailRegImm { op, reg, imm, continue_when, .. } => {
            let reg_ptr = unsafe { sp.add(base + reg as usize) };
            if unsafe { !pttisinteger(reg_ptr as *const LuaValue) } {
                return false;
            }
            (
                op,
                unsafe { pivalue(reg_ptr as *const LuaValue) },
                imm as i64,
                continue_when,
            )
        }
    };
    (match op {
        jit::LinearIntGuardOp::Eq => lhs_val == rhs_val,
        jit::LinearIntGuardOp::Lt => lhs_val < rhs_val,
        jit::LinearIntGuardOp::Le => lhs_val <= rhs_val,
        jit::LinearIntGuardOp::Gt => lhs_val > rhs_val,
        jit::LinearIntGuardOp::Ge => lhs_val >= rhs_val,
    }) == continue_when
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_linear_int_jmp_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    steps: &[jit::LinearIntStep],
    guard: jit::LinearIntLoopGuard,
    summary: jit::HelperPlanDispatchSummary,
) -> Option<JitTraceAction> {
    let exit_pc = guard.exit_pc() as usize;
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        if guard.is_head() && !linear_int_guard_holds(sp, base, guard)
        {
            return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
        }

        if !unsafe { execute_linear_int_steps(sp, base, steps) } {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        trace_hits = trace_hits.saturating_add(1);

        if guard.is_tail() && !linear_int_guard_holds(sp, base, guard)
        {
            return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
        }
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) fn jit_execute_return0(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    summary: jit::HelperPlanDispatchSummary,
) -> JitTraceAction {
    jit::record_batched_trace_execution(lua_state, 1, 1, summary);
    jit_finish_return_results(lua_state, ci, ci.base, 0)
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) fn jit_execute_return1(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    src_reg: u32,
    summary: jit::HelperPlanDispatchSummary,
) -> JitTraceAction {
    jit::record_batched_trace_execution(lua_state, 1, 1, summary);
    jit_finish_return_results(lua_state, ci, base + src_reg as usize, 1)
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) fn jit_execute_return(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    start_reg: u32,
    result_count: u8,
    summary: jit::HelperPlanDispatchSummary,
) -> JitTraceAction {
    jit::record_batched_trace_execution(lua_state, 1, 1, summary);
    jit_finish_return_results(
        lua_state,
        ci,
        base + start_reg as usize,
        result_count as usize,
    )
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_numeric_jmp_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    pre_steps: &[jit::NumericStep],
    steps: &[jit::NumericStep],
    guard: jit::NumericJmpLoopGuard,
    summary: jit::HelperPlanDispatchSummary,
) -> Option<JitTraceAction> {
    let exit_pc = guard.exit_pc() as usize;
    let (cond, continue_when, continue_preset, exit_preset) = match guard {
        jit::NumericJmpLoopGuard::Head { cond, continue_when, continue_preset, exit_preset, exit_pc } => {
            let _ = exit_pc;
            (cond, continue_when, continue_preset, exit_preset)
        }
        jit::NumericJmpLoopGuard::Tail { cond, continue_when, continue_preset, exit_preset, exit_pc } => {
            let _ = exit_pc;
            (cond, continue_when, continue_preset, exit_preset)
        }
    };
    let tail_guard = guard.is_tail();
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        if !execute_numeric_steps(lua_state, ci, sp, base, constants, pre_steps) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }
        if !tail_guard {
            let guard_holds = numeric_ifelse_cond_holds(sp, base, cond) == continue_when;
            if !guard_holds {
                if let Some(step) = exit_preset.as_ref()
                    && !jit_execute_single_numeric_step(lua_state, ci, sp, base, constants, step)
                {
                    return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
                }
                return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
            }
            if let Some(step) = continue_preset.as_ref()
                && !jit_execute_single_numeric_step(lua_state, ci, sp, base, constants, step)
            {
                return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
            }
        }

        if !execute_numeric_steps(lua_state, ci, sp, base, constants, steps) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        trace_hits = trace_hits.saturating_add(1);

        if tail_guard {
            let sp = lua_state.stack_mut().as_mut_ptr();
            let guard_holds = numeric_ifelse_cond_holds(sp, base, cond) == continue_when;
            if !guard_holds {
                if let Some(step) = exit_preset.as_ref()
                    && !jit_execute_single_numeric_step(lua_state, ci, sp, base, constants, step)
                {
                    return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
                }
                return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
            }
            if let Some(step) = continue_preset.as_ref()
                && !jit_execute_single_numeric_step(lua_state, ci, sp, base, constants, step)
            {
                return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
            }
        }
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_numeric_table_scan_jmp_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    table_reg: u32,
    index_reg: u32,
    limit_reg: u32,
    step_imm: i32,
    compare_op: jit::LinearIntGuardOp,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<JitTraceAction> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let table_ptr = sp.add(base + table_reg as usize);
        let index_ptr = sp.add(base + index_reg as usize);
        let limit_ptr = sp.add(base + limit_reg as usize);

        if !(*table_ptr).is_table() || !pttisinteger(index_ptr as *const LuaValue) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let limit_value = &*limit_ptr;
        if !(ttisinteger(limit_value) || ttisfloat(limit_value)) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let table = (*table_ptr).hvalue();
        let idx = pivalue(index_ptr as *const LuaValue);
        let mut loaded_value = LuaValue::nil();
        let loaded = table.impl_table.fast_geti_into(idx, &mut loaded_value);
        if !loaded || !(ttisinteger(&loaded_value) || ttisfloat(&loaded_value)) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let guard_holds = match compare_op {
            jit::LinearIntGuardOp::Lt => lt_num(&loaded_value, limit_value),
            jit::LinearIntGuardOp::Le => le_num(&loaded_value, limit_value),
            jit::LinearIntGuardOp::Gt => lt_num(limit_value, &loaded_value),
            jit::LinearIntGuardOp::Ge => le_num(limit_value, &loaded_value),
            jit::LinearIntGuardOp::Eq => {
                if ttisinteger(&loaded_value) && ttisinteger(limit_value) {
                    loaded_value.ivalue() == limit_value.ivalue()
                } else if ttisfloat(&loaded_value) && ttisfloat(limit_value) {
                    loaded_value.fltvalue() == limit_value.fltvalue()
                } else if ttisinteger(&loaded_value) && ttisfloat(limit_value) {
                    loaded_value.ivalue() as f64 == limit_value.fltvalue()
                } else if ttisfloat(&loaded_value) && ttisinteger(limit_value) {
                    loaded_value.fltvalue() == limit_value.ivalue() as f64
                } else {
                    false
                }
            }
        };

        if !guard_holds {
            return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
        }

        psetivalue(index_ptr, idx.wrapping_add(step_imm as i64));
        trace_hits = trace_hits.saturating_add(1);
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_numeric_table_shift_jmp_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    table_reg: u32,
    index_reg: u32,
    left_bound_reg: u32,
    value_reg: u32,
    temp_reg: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<JitTraceAction> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let table_ptr = sp.add(base + table_reg as usize);
        let index_ptr = sp.add(base + index_reg as usize);
        let left_bound_ptr = sp.add(base + left_bound_reg as usize);
        let value_ptr = sp.add(base + value_reg as usize);
        let temp_ptr = sp.add(base + temp_reg as usize);

        if !(*table_ptr).is_table()
            || !pttisinteger(index_ptr as *const LuaValue)
            || !pttisinteger(left_bound_ptr as *const LuaValue)
        {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let index = pivalue(index_ptr as *const LuaValue);
        let left_bound = pivalue(left_bound_ptr as *const LuaValue);
        if left_bound > index {
            return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
        }

        let value = *value_ptr;
        let table = (*table_ptr).hvalue_mut();
        let meta = table.meta_ptr();
        if !(meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into())) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        if !table.impl_table.fast_geti_into(index, temp_ptr) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let current = *temp_ptr;
        if !lt_num(&value, &current) {
            return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
        }

        let next_index = index.wrapping_add(1);
        if !table.impl_table.fast_seti_parts(next_index, current.value, current.tt) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        if current.tt & 0x40 != 0 {
            lua_state.gc_barrier_back((*table_ptr).as_gc_ptr_table_unchecked());
        }

        psetivalue(index_ptr, index.wrapping_sub(1));
        trace_hits = trace_hits.saturating_add(1);
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_next_while_builtin_add(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    key_reg: u32,
    value_reg: u32,
    acc_reg: u32,
    table_reg: u32,
    env_upvalue: u32,
    key_const: u32,
    summary: jit::HelperPlanDispatchSummary,
) -> Option<JitTraceAction> {
    let exit_pc = target_pc + 11;
    let mut trace_hits = 0u32;
    let key_name = unsafe { constants.get_unchecked(key_const as usize) };

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let key_ptr = sp.add(base + key_reg as usize);
        let value_ptr = sp.add(base + value_reg as usize);
        let acc_ptr = sp.add(base + acc_reg as usize);
        let table_ptr = sp.add(base + table_reg as usize);

        if (*key_ptr).is_nil()
            || !(*table_ptr).is_table()
            || !pttisinteger(acc_ptr as *const LuaValue)
            || !pttisinteger(value_ptr as *const LuaValue)
        {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let upvalue_ptr = *ci.upvalue_ptrs.add(env_upvalue as usize);
        let env_value = upvalue_ptr.as_ref().data.get_value_ref();
        let Some(env_table) = env_value.as_table() else {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        };
        let Some(next_value) = env_table.raw_get(key_name) else {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        };
        let Some(next_fn) = next_value.as_cfunction() else {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        };
        if !std::ptr::fn_addr_eq(next_fn, lua_next as crate::lua_vm::CFunction) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        psetivalue(
            acc_ptr,
            pivalue(acc_ptr as *const LuaValue)
                .wrapping_add(pivalue(value_ptr as *const LuaValue)),
        );
        trace_hits = trace_hits.saturating_add(1);

        let table = (*table_ptr).hvalue();
        let current_key = *key_ptr;
        match lua_next_into(table, &current_key, key_ptr, value_ptr) {
            Ok(true) => {
                if !pttisinteger(value_ptr as *const LuaValue) {
                    return jit_trace_complete_without_exit(lua_state, trace_hits, summary);
                }
            }
            Ok(false) => {
                setnilvalue(&mut *key_ptr);
                setnilvalue(&mut *value_ptr);
                return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
            }
            Err(()) => {
                return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
            }
        }
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_linear_int_for_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_reg: u32,
    steps: &[jit::LinearIntStep],
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<JitTraceAction> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        if !execute_linear_int_steps(sp, base, steps) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_numeric_for_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    steps: &[jit::NumericStep],
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<JitTraceAction> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);

        if !execute_numeric_steps(lua_state, ci, sp, base, constants, steps) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_guarded_numeric_for_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    steps: &[jit::NumericStep],
    guard: jit::NumericJmpLoopGuard,
    summary: jit::HelperPlanDispatchSummary,
    loop_exit_pc: usize,
) -> Option<JitTraceAction> {
    let (cond, continue_when, continue_preset, exit_preset, exit_pc) = match guard {
        jit::NumericJmpLoopGuard::Tail {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            exit_pc,
        } => (cond, continue_when, continue_preset, exit_preset, exit_pc as usize),
        _ => return None,
    };

    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);

        if !execute_numeric_steps(lua_state, ci, sp, base, constants, steps) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let guard_holds = numeric_ifelse_cond_holds(sp, base, cond) == continue_when;
        if !guard_holds {
            if let Some(step) = exit_preset.as_ref()
                && !jit_execute_single_numeric_step(lua_state, ci, sp, base, constants, step)
            {
                return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
            }
            return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
        }
        if let Some(step) = continue_preset.as_ref()
            && !jit_execute_single_numeric_step(lua_state, ci, sp, base, constants, step)
        {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, loop_exit_pc);
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_numeric_ifelse_for_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    pre_steps: &[jit::NumericStep],
    cond: jit::NumericIfElseCond,
    then_preset: Option<jit::NumericStep>,
    else_preset: Option<jit::NumericStep>,
    then_steps: &[jit::NumericStep],
    else_steps: &[jit::NumericStep],
    then_on_true: bool,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<JitTraceAction> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);

        if !execute_numeric_steps(lua_state, ci, sp, base, constants, pre_steps) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let cond_holds = numeric_ifelse_cond_holds(sp, base, cond);
        let take_then = cond_holds == then_on_true;
        let branch_preset = if take_then {
            then_preset.as_ref()
        } else {
            else_preset.as_ref()
        };
        if let Some(step) = branch_preset
            && !jit_execute_single_numeric_step(lua_state, ci, sp, base, constants, step)
        {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let branch_steps = if take_then { then_steps } else { else_steps };
        if !execute_numeric_steps(lua_state, ci, sp, base, constants, branch_steps) {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
pub(crate) unsafe fn jit_execute_generic_for_builtin_add(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    tfor_reg: u32,
    value_reg: u32,
    acc_reg: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<JitTraceAction> {
    let mut trace_hits = 0u32;
    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let tfor_base = base + tfor_reg as usize;
        let iter_ptr = sp.add(tfor_base);
        let state_ptr = sp.add(tfor_base + 1);
        let control_ptr = sp.add(tfor_base + 3);
        let value_ptr = sp.add(base + value_reg as usize);
        let acc_ptr = sp.add(base + acc_reg as usize);

        let iter_kind = if let Some(iter_fn) = (*iter_ptr).as_cfunction() {
            if std::ptr::fn_addr_eq(iter_fn, lua_next as crate::lua_vm::CFunction) {
                Some(false)
            } else if std::ptr::fn_addr_eq(iter_fn, ipairs_next as crate::lua_vm::CFunction)
                || ((*state_ptr).is_table()
                    && !(*state_ptr).hvalue().has_metatable()
                    && pttisinteger(control_ptr as *const LuaValue))
            {
                Some(true)
            } else {
                None
            }
        } else {
            None
        };

        let Some(is_ipairs) = iter_kind else {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        };

        if !pttisinteger(acc_ptr as *const LuaValue) || !pttisinteger(value_ptr as *const LuaValue)
        {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        psetivalue(
            acc_ptr,
            pivalue(acc_ptr as *const LuaValue).wrapping_add(pivalue(value_ptr as *const LuaValue)),
        );
        trace_hits = trace_hits.saturating_add(1);

        if is_ipairs {
            if !(*state_ptr).is_table() || !pttisinteger(control_ptr as *const LuaValue) {
                return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
            }

            let table = (*state_ptr).hvalue();
            if table.has_metatable() {
                return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
            }

            let next_index = pivalue(control_ptr as *const LuaValue).wrapping_add(1);
            let loaded = table.impl_table.fast_geti_into(next_index, value_ptr)
                || table.impl_table.get_int_from_hash_into(next_index, value_ptr);
            if !loaded {
                setnilvalue(&mut *control_ptr);
                setnilvalue(&mut *value_ptr);
                return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
            }

            psetivalue(control_ptr, next_index);
            if !pttisinteger(value_ptr as *const LuaValue) {
                return jit_trace_complete_without_exit(lua_state, trace_hits, summary);
            }
            continue;
        }

        if !(*state_ptr).is_table() {
            return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
        }

        let table = (*state_ptr).hvalue();
        let current_key = *control_ptr;
        match lua_next_into(table, &current_key, control_ptr, value_ptr) {
            Ok(true) => {
                if !pttisinteger(value_ptr as *const LuaValue) {
                    return jit_trace_complete_without_exit(lua_state, trace_hits, summary);
                }
            }
            Ok(false) => {
                setnilvalue(&mut *control_ptr);
                setnilvalue(&mut *value_ptr);
                return jit_trace_exit(lua_state, ci, base, target_pc, trace_hits, summary, exit_pc);
            }
            Err(()) => {
                return jit_trace_fallback(lua_state, ci, target_pc, trace_hits, summary);
            }
        }
    }
}

