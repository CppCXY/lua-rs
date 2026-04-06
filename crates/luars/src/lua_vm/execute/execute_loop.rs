/*----------------------------------------------------------------------
  Lua 5.5 VM Execution Engine

  Design Philosophy:
  1. **Slice-Based**: Code and constants accessed via `&[T]` slices with
     `noalias` guarantees — LLVM keeps slice base pointers in registers
     across function calls (raw pointers must be reloaded after `&mut` calls)
  2. **Minimal Indirection**: Use get_unchecked for stack access (no bounds checks)
  4. **CPU Register Optimization**: code, constants, pc, base, trap in CPU registers
  5. **Unsafe but Sound**: Use raw pointers with invariant guarantees for stack

  Key Invariants (maintained by caller):
  - Stack pointer valid throughout execution (no reallocation)
  - CallInfo valid and matches current frame
  - Chunk lifetime extends through execution
  - base + register < stack.len() (validated at call time)

  This leverages Rust's type system for LLVM optimization opportunities
----------------------------------------------------------------------*/

use crate::{
    CallInfo, Instruction, LUA_MASKCALL, LUA_MASKCOUNT, LUA_MASKLINE, LUA_MASKRET, LuaResult,
    LuaState, LuaValue, OpCode,
    gc::TablePtr,
    lua_value::LUA_VNUMINT,
    lua_vm::{
        LuaError, TmKind,
        call_info::call_status::{CIST_C, CIST_CLSRET, CIST_PENDING_FINISH},
        execute::{
            call::{poscall, precall, pretailcall},
            closure::push_closure,
            concat::{concat, try_concat_pair_utf8},
            helper::{
                bin_tm_fallback, eq_fallback, error_div_by_zero, error_global, error_mod_by_zero,
                finishget_fallback, finishset_fallback, finishset_fallback_known_miss,
                float_for_loop, fltvalue, forprep, handle_pending_ops, ivalue, lua_fmod, lua_idiv,
                lua_imod, lua_shiftl, lua_shiftr, luai_numpow, objlen, order_tm_fallback,
                pfltvalue, pivalue, psetfltvalue, psetivalue, ptonumberns, pttisfloat,
                pttisinteger, return0_with_hook, return1_with_hook, self_shortstr_index_chain_fast,
                setbfvalue, setbtvalue, setfltvalue, setivalue, setnilvalue, setobj2s, setobjs2s,
                tointeger, tointegerns, tonumberns, ttisfloat, ttisinteger, ttisstring,
                unary_tm_fallback,
            },
            hook::{hook_check_instruction, hook_on_call},
            metamethod::call_newindex_tm_fast,
            number::{le_num, lt_num},
            vararg::{exec_varargprep, get_vararg, get_varargs},
        },
        lua_limits::EXTRA_STACK,
    },
};

#[cfg(feature = "jit")]
use crate::lua_vm::jit;
#[cfg(feature = "jit")]
use crate::stdlib::basic::{ipairs_next, lua_next};

#[cfg(feature = "jit")]
#[inline(always)]
unsafe fn jit_lua_next_into(
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
unsafe fn jit_execute_linear_int_steps(
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
unsafe fn jit_read_numeric_operand(
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
fn jit_numeric_binary_result(
    lhs: LuaValue,
    rhs: LuaValue,
    op: jit::NumericBinaryOp,
) -> Option<LuaValue> {
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
                jit::NumericBinaryOp::Div
                | jit::NumericBinaryOp::IDiv
                | jit::NumericBinaryOp::Mod
                | jit::NumericBinaryOp::Pow
                | jit::NumericBinaryOp::BAnd
                | jit::NumericBinaryOp::BOr
                | jit::NumericBinaryOp::BXor
                | jit::NumericBinaryOp::Shl
                | jit::NumericBinaryOp::Shr => unreachable!(),
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
                jit::NumericBinaryOp::Div
                | jit::NumericBinaryOp::IDiv
                | jit::NumericBinaryOp::Mod
                | jit::NumericBinaryOp::Pow
                | jit::NumericBinaryOp::BAnd
                | jit::NumericBinaryOp::BOr
                | jit::NumericBinaryOp::BXor
                | jit::NumericBinaryOp::Shl
                | jit::NumericBinaryOp::Shr => unreachable!(),
            };
            Some(LuaValue::float(value))
        }
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_linear_int_compare(lhs: i64, rhs: i64, op: jit::LinearIntGuardOp) -> bool {
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
unsafe fn jit_numeric_ifelse_cond_holds(
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
            jit_linear_int_compare(cond_value, imm as i64, op)
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
unsafe fn jit_execute_numeric_steps(
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
            jit::NumericStep::GetTableInt { dst, table, index } => {
                let table_ptr = unsafe { sp.add(base + table as usize) };
                let index_ptr = unsafe { sp.add(base + index as usize) };
                let dst_ptr = unsafe { sp.add(base + dst as usize) };
                if unsafe {
                    !(*table_ptr).is_table() || !pttisinteger(index_ptr as *const LuaValue)
                } {
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
                let Some(lhs_value) = (unsafe { jit_read_numeric_operand(sp, base, constants, lhs) }) else {
                    return false;
                };
                let Some(rhs_value) = (unsafe { jit_read_numeric_operand(sp, base, constants, rhs) }) else {
                    return false;
                };
                let Some(result) = jit_numeric_binary_result(lhs_value, rhs_value, op) else {
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
unsafe fn jit_linear_int_guard_holds(
    sp: *mut LuaValue,
    base: usize,
    guard: jit::LinearIntLoopGuard,
) -> bool {
    let (op, lhs_val, rhs_val, continue_when) = match guard {
        jit::LinearIntLoopGuard::HeadRegReg {
            op,
            lhs,
            rhs,
            continue_when,
            ..
        }
        | jit::LinearIntLoopGuard::TailRegReg {
            op,
            lhs,
            rhs,
            continue_when,
            ..
        } => {
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
        jit::LinearIntLoopGuard::HeadRegImm {
            op,
            reg,
            imm,
            continue_when,
            ..
        }
        | jit::LinearIntLoopGuard::TailRegImm {
            op,
            reg,
            imm,
            continue_when,
            ..
        } => {
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
    let cond = match op {
        jit::LinearIntGuardOp::Eq => lhs_val == rhs_val,
        jit::LinearIntGuardOp::Lt => lhs_val < rhs_val,
        jit::LinearIntGuardOp::Le => lhs_val <= rhs_val,
        jit::LinearIntGuardOp::Gt => lhs_val > rhs_val,
        jit::LinearIntGuardOp::Ge => lhs_val >= rhs_val,
    };
    cond == continue_when
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_record_trace_hits_or_fallback(
    lua_state: &mut LuaState,
    chunk_ptr: *const crate::lua_value::LuaProto,
    target_pc: usize,
    trace_hits: u32,
    summary: jit::HelperPlanDispatchSummary,
) {
    if trace_hits > 0 {
        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
    } else {
        jit::record_loop_backedge(lua_state, chunk_ptr, target_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_linear_int_jmp_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    steps: &[jit::LinearIntStep],
    guard: jit::LinearIntLoopGuard,
    summary: jit::HelperPlanDispatchSummary,
) -> Option<usize> {
    let exit_pc = match guard {
        jit::LinearIntLoopGuard::HeadRegReg { exit_pc, .. }
        | jit::LinearIntLoopGuard::HeadRegImm { exit_pc, .. }
        | jit::LinearIntLoopGuard::TailRegReg { exit_pc, .. }
        | jit::LinearIntLoopGuard::TailRegImm { exit_pc, .. } => exit_pc as usize,
    };
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        if let jit::LinearIntLoopGuard::HeadRegReg { .. } = guard
            && !jit_linear_int_guard_holds(sp, base, guard)
        {
            jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
            return Some(exit_pc);
        }

        if !jit_execute_linear_int_steps(sp, base, steps) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        trace_hits = trace_hits.saturating_add(1);

        if let jit::LinearIntLoopGuard::TailRegReg { .. } = guard
            && !jit_linear_int_guard_holds(sp, base, guard)
        {
            jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
            return Some(exit_pc);
        }
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_jmp_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    pre_steps: &[jit::NumericStep],
    steps: &[jit::NumericStep],
    guard: jit::NumericJmpLoopGuard,
    summary: jit::HelperPlanDispatchSummary,
) -> Option<usize> {
    let (cond, continue_when, continue_preset, exit_preset, exit_pc, tail_guard) = match guard {
        jit::NumericJmpLoopGuard::Head {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            exit_pc,
        } => (cond, continue_when, continue_preset, exit_preset, exit_pc as usize, false),
        jit::NumericJmpLoopGuard::Tail {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            exit_pc,
        } => (cond, continue_when, continue_preset, exit_preset, exit_pc as usize, true),
    };
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        if !jit_execute_numeric_steps(sp, base, constants, pre_steps) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }
        if !tail_guard {
            let guard_holds = jit_numeric_ifelse_cond_holds(sp, base, cond) == continue_when;
            if !guard_holds {
                if let Some(step) = exit_preset.as_ref()
                    && !jit_execute_numeric_steps(sp, base, constants, std::slice::from_ref(step))
                {
                    jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                    return None;
                }
                jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
                return Some(exit_pc);
            }
            if let Some(step) = continue_preset.as_ref()
                && !jit_execute_numeric_steps(sp, base, constants, std::slice::from_ref(step))
            {
                jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                return None;
            }
        }

        if !jit_execute_numeric_steps(sp, base, constants, steps) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        trace_hits = trace_hits.saturating_add(1);

        if tail_guard {
            let sp = lua_state.stack_mut().as_mut_ptr();
            let guard_holds = jit_numeric_ifelse_cond_holds(sp, base, cond) == continue_when;
            if !guard_holds {
                if let Some(step) = exit_preset.as_ref()
                    && !jit_execute_numeric_steps(sp, base, constants, std::slice::from_ref(step))
                {
                    jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                    return None;
                }
                jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
                return Some(exit_pc);
            }
            if let Some(step) = continue_preset.as_ref()
                && !jit_execute_numeric_steps(sp, base, constants, std::slice::from_ref(step))
            {
                jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                return None;
            }
        }
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_table_scan_jmp_loop(
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
) -> Option<usize> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let table_ptr = sp.add(base + table_reg as usize);
        let index_ptr = sp.add(base + index_reg as usize);
        let limit_ptr = sp.add(base + limit_reg as usize);

        if !(*table_ptr).is_table() || !pttisinteger(index_ptr as *const LuaValue) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let limit_value = &*limit_ptr;
        if !(ttisinteger(limit_value) || ttisfloat(limit_value)) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let table = (*table_ptr).hvalue();
        let idx = pivalue(index_ptr as *const LuaValue);
        let mut loaded_value = LuaValue::nil();
        let loaded = table.impl_table.fast_geti_into(idx, &mut loaded_value);
        if !loaded || !(ttisinteger(&loaded_value) || ttisfloat(&loaded_value)) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
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
            jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
            return Some(exit_pc);
        }

        psetivalue(index_ptr, idx.wrapping_add(step_imm as i64));
        trace_hits = trace_hits.saturating_add(1);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_table_shift_jmp_loop(
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
) -> Option<usize> {
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
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let index = pivalue(index_ptr as *const LuaValue);
        let left_bound = pivalue(left_bound_ptr as *const LuaValue);
        if left_bound > index {
            jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
            return Some(exit_pc);
        }

        let value = *value_ptr;
        let table = (*table_ptr).hvalue_mut();
        let meta = table.meta_ptr();
        if !(meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into())) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        if !table.impl_table.fast_geti_into(index, temp_ptr) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let current = *temp_ptr;
        if !lt_num(&value, &current) {
            jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
            return Some(exit_pc);
        }

        let next_index = index.wrapping_add(1);
        if !table
            .impl_table
            .fast_seti_parts(next_index, current.value, current.tt)
        {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        if current.tt & 0x40 != 0 {
            lua_state.gc_barrier_back((*table_ptr).as_gc_ptr_table_unchecked());
        }

        psetivalue(index_ptr, index.wrapping_sub(1));
        trace_hits = trace_hits.saturating_add(1);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_next_while_builtin_add(
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
) -> Option<usize> {
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
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let upvalue_ptr = *ci.upvalue_ptrs.add(env_upvalue as usize);
        let env_value = upvalue_ptr.as_ref().data.get_value_ref();
        let Some(env_table) = env_value.as_table() else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };
        let Some(next_value) = env_table.raw_get(key_name) else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };
        let Some(next_fn) = next_value.as_cfunction() else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };
        if !std::ptr::fn_addr_eq(next_fn, lua_next as crate::lua_vm::CFunction) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        psetivalue(
            acc_ptr,
            pivalue(acc_ptr as *const LuaValue).wrapping_add(pivalue(value_ptr as *const LuaValue)),
        );
        trace_hits = trace_hits.saturating_add(1);

        let table = (*table_ptr).hvalue();
        let current_key = *key_ptr;
        match jit_lua_next_into(table, &current_key, key_ptr, value_ptr) {
            Ok(true) => {
                if !pttisinteger(value_ptr as *const LuaValue) {
                    jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
                    return None;
                }
            }
            Ok(false) => {
                setnilvalue(&mut *key_ptr);
                setnilvalue(&mut *value_ptr);
                jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
                return Some(exit_pc);
            }
            Err(()) => {
                jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                return None;
            }
        }
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_try_handle_jmp_backedge(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
) -> Option<usize> {
    let Some((executor, summary)) = jit::compiled_trace_executor_or_record(lua_state, ci.chunk_ptr, target_pc) else {
        return None;
    };

    match executor {
        jit::CompiledTraceExecutor::LinearIntJmpLoop { steps, guard } => unsafe {
            jit_execute_linear_int_jmp_loop(lua_state, ci, base, target_pc, &steps, guard, summary)
        },
        jit::CompiledTraceExecutor::NumericTableScanJmpLoop {
            table_reg,
            index_reg,
            limit_reg,
            step_imm,
            compare_op,
            exit_pc,
        } => unsafe {
            jit_execute_numeric_table_scan_jmp_loop(
                lua_state,
                ci,
                base,
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
        jit::CompiledTraceExecutor::NumericTableShiftJmpLoop {
            table_reg,
            index_reg,
            left_bound_reg,
            value_reg,
            temp_reg,
            exit_pc,
        } => unsafe {
            jit_execute_numeric_table_shift_jmp_loop(
                lua_state,
                ci,
                base,
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
        jit::CompiledTraceExecutor::NumericJmpLoop {
            pre_steps,
            steps,
            guard,
        } => unsafe {
            jit_execute_numeric_jmp_loop(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                &pre_steps,
                &steps,
                guard,
                summary,
            )
        },
        jit::CompiledTraceExecutor::NextWhileBuiltinAdd {
            key_reg,
            value_reg,
            acc_reg,
            table_reg,
            env_upvalue,
            key_const,
        } => unsafe {
            jit_execute_next_while_builtin_add(
                lua_state,
                ci,
                base,
                constants,
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
        _ => {
            jit::record_loop_backedge(lua_state, ci.chunk_ptr, target_pc);
            None
        }
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_gettable_add(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_reg: u32,
    table_reg: u32,
    index_reg: u32,
    value_reg: u32,
    acc_reg: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let table_ptr = sp.add(base + table_reg as usize);
        let index_ptr = sp.add(base + index_reg as usize);
        let value_ptr = sp.add(base + value_reg as usize);
        let acc_ptr = sp.add(base + acc_reg as usize);

        if !(*table_ptr).is_table()
            || !pttisinteger(index_ptr as *const LuaValue)
            || !pttisinteger(acc_ptr as *const LuaValue)
        {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let table = (*table_ptr).hvalue();
        let idx = pivalue(index_ptr as *const LuaValue);
        let loaded = table.impl_table.fast_geti_into(idx, value_ptr)
            || table.impl_table.get_int_from_hash_into(idx, value_ptr);
        if !loaded || !pttisinteger(value_ptr as *const LuaValue) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        psetivalue(
            acc_ptr,
            pivalue(acc_ptr as *const LuaValue).wrapping_add(pivalue(value_ptr as *const LuaValue)),
        );
        trace_hits = trace_hits.saturating_add(1);

        let remaining = pivalue(loop_ptr) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_upvalue_addi(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_reg: u32,
    upvalue: u32,
    value_reg: u32,
    imm: i32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let value_ptr = sp.add(base + value_reg as usize);
        let upvalue_ptr = *ci.upvalue_ptrs.add(upvalue as usize);
        let current = upvalue_ptr.as_ref().data.get_value_ref();

        if !pttisinteger(current as *const LuaValue) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let next = pivalue(current as *const LuaValue).wrapping_add(imm as i64);
        psetivalue(value_ptr, next);
        upvalue_ptr
            .as_mut_ref()
            .data
            .set_value_parts((*value_ptr).value, (*value_ptr).tt);

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_table_mul_add_mod(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    table_reg: u32,
    index_reg: u32,
    acc_reg: u32,
    modulo_const: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let modulo_value = constants.get(modulo_const as usize)?;
    let Some(modulo) = modulo_value.as_integer() else {
        return None;
    };
    if modulo == 0 {
        return None;
    }

    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = sp.add(base + index_reg as usize);
        let table_ptr = sp.add(base + table_reg as usize);
        let acc_ptr = sp.add(base + acc_reg as usize);

        if !(*table_ptr).is_table()
            || !pttisinteger(index_ptr as *const LuaValue)
            || !pttisinteger(acc_ptr as *const LuaValue)
        {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let table = (*table_ptr).hvalue();
        let idx = pivalue(index_ptr as *const LuaValue);
        let mut loaded_value = LuaValue::nil();
        let loaded = table.impl_table.fast_geti_into(idx, &mut loaded_value);
        if !loaded || !ttisinteger(&loaded_value) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let product = loaded_value.ivalue().wrapping_mul(idx);
        let sum = pivalue(acc_ptr as *const LuaValue).wrapping_add(product);
        psetivalue(acc_ptr, sum.rem_euclid(modulo));

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_table_copy(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_reg: u32,
    src_table_reg: u32,
    dst_table_reg: u32,
    index_reg: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = sp.add(base + index_reg as usize);
        let src_table_ptr = sp.add(base + src_table_reg as usize);
        let dst_table_ptr = sp.add(base + dst_table_reg as usize);

        if !(*src_table_ptr).is_table()
            || !(*dst_table_ptr).is_table()
            || !pttisinteger(index_ptr as *const LuaValue)
        {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let dst_table = (*dst_table_ptr).hvalue_mut();
        let meta = dst_table.meta_ptr();
        if !(meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into())) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let idx = pivalue(index_ptr as *const LuaValue);
        let src_table = (*src_table_ptr).hvalue();
        let mut loaded_value = LuaValue::nil();
        let loaded = src_table.impl_table.fast_geti_into(idx, &mut loaded_value)
            || src_table.impl_table.get_int_from_hash_into(idx, &mut loaded_value);
        if !loaded {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        dst_table.impl_table.set_int(idx, loaded_value);
        if loaded_value.tt & 0x40 != 0 {
            lua_state.gc_barrier_back((*dst_table_ptr).as_gc_ptr_table_unchecked());
        }

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_table_is_sorted(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_reg: u32,
    table_reg: u32,
    index_reg: u32,
    false_exit_pc: usize,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = sp.add(base + index_reg as usize);
        let table_ptr = sp.add(base + table_reg as usize);

        if !(*table_ptr).is_table() || !pttisinteger(index_ptr as *const LuaValue) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let table = (*table_ptr).hvalue();
        if !jit_plain_numeric_table_guard(&table) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let current_index = pivalue(index_ptr as *const LuaValue);
        let previous_index = current_index.wrapping_sub(1);
        let mut previous_value = LuaValue::nil();
        let mut current_value = LuaValue::nil();

        let loaded_previous = table.impl_table.fast_geti_into(previous_index, &mut previous_value)
            || table.impl_table.get_int_from_hash_into(previous_index, &mut previous_value);
        let loaded_current = table.impl_table.fast_geti_into(current_index, &mut current_value)
            || table.impl_table.get_int_from_hash_into(current_index, &mut current_value);

        let Some(previous_number) = loaded_previous.then(|| previous_value.as_float()).flatten() else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };
        let Some(current_number) = loaded_current.then(|| current_value.as_float()).flatten() else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };

        trace_hits = trace_hits.saturating_add(1);
        if current_number < previous_number {
            jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
            return Some(false_exit_pc);
        }

        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = current_index.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_field_addi(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    table_reg: u32,
    value_reg: u32,
    key_const: u32,
    imm: i32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let key = constants.get(key_const as usize)?;

    if !key.is_short_string() {
        return None;
    }

    let sp = lua_state.stack_mut().as_mut_ptr();
    let table_ptr = sp.add(base + table_reg as usize);
    let value_ptr = sp.add(base + value_reg as usize);

    if !(*table_ptr).is_table() {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let table = (*table_ptr).hvalue_mut();
    let meta = table.meta_ptr();
    if !(meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into())) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let Some(field_slot) = table.impl_table.find_existing_shortstr_slot(key) else {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    };
    if !table.impl_table.shortstr_slot_into(field_slot, value_ptr) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }
    if !pttisinteger(value_ptr as *const LuaValue) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let value_ptr = sp.add(base + value_reg as usize);

        psetivalue(value_ptr, pivalue(value_ptr as *const LuaValue).wrapping_add(imm as i64));
        if !table.impl_table.set_shortstr_slot_parts(field_slot, (*value_ptr).value, (*value_ptr).tt) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
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

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_tabup_addi(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    env_upvalue: u32,
    value_reg: u32,
    key_const: u32,
    imm: i32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let key = constants.get(key_const as usize)?;

    if !key.is_short_string() {
        return None;
    }

    let sp = lua_state.stack_mut().as_mut_ptr();
    let value_ptr = sp.add(base + value_reg as usize);
    let upvalue_ptr = *ci.upvalue_ptrs.add(env_upvalue as usize);
    let env_value = upvalue_ptr.as_ref().data.get_value_ref();

    if !env_value.is_table() {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let table = env_value.hvalue_mut();
    let meta = table.meta_ptr();
    if !(meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into())) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let Some(slot) = table.impl_table.find_existing_shortstr_slot(key) else {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    };
    if !table.impl_table.shortstr_slot_into(slot, value_ptr) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }
    if !pttisinteger(value_ptr as *const LuaValue) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let value_ptr = sp.add(base + value_reg as usize);

        psetivalue(value_ptr, pivalue(value_ptr as *const LuaValue).wrapping_add(imm as i64));
        if !table.impl_table.set_shortstr_slot_parts(slot, (*value_ptr).value, (*value_ptr).tt) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
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

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_tabup_field_addi(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    env_upvalue: u32,
    table_reg: u32,
    value_reg: u32,
    table_key_const: u32,
    field_key_const: u32,
    imm: i32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let table_key = constants.get(table_key_const as usize)?;
    let field_key = constants.get(field_key_const as usize)?;

    if !table_key.is_short_string() || !field_key.is_short_string() {
        return None;
    }

    let sp = lua_state.stack_mut().as_mut_ptr();
    let table_ptr = sp.add(base + table_reg as usize);
    let value_ptr = sp.add(base + value_reg as usize);
    let upvalue_ptr = *ci.upvalue_ptrs.add(env_upvalue as usize);
    let env_value = upvalue_ptr.as_ref().data.get_value_ref();

    if !env_value.is_table() {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let env_table = env_value.hvalue();
    if !env_table.impl_table.has_hash() || !env_table.impl_table.get_shortstr_into(table_key, table_ptr) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }
    if !(*table_ptr).is_table() {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let table = (*table_ptr).hvalue_mut();
    let meta = table.meta_ptr();
    if !(meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into())) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let Some(field_slot) = table.impl_table.find_existing_shortstr_slot(field_key) else {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    };
    if !table.impl_table.shortstr_slot_into(field_slot, value_ptr) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }
    if !pttisinteger(value_ptr as *const LuaValue) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let value_ptr = sp.add(base + value_reg as usize);
        psetivalue(value_ptr, pivalue(value_ptr as *const LuaValue).wrapping_add(imm as i64));
        if !table.impl_table.set_shortstr_slot_parts(field_slot, (*value_ptr).value, (*value_ptr).tt) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
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

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_tabup_field_load(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    env_upvalue: u32,
    value_reg: u32,
    table_key_const: u32,
    field_key_const: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let table_key = constants.get(table_key_const as usize)?;
    let field_key = constants.get(field_key_const as usize)?;

    if !table_key.is_short_string() || !field_key.is_short_string() {
        return None;
    }

    let sp = lua_state.stack_mut().as_mut_ptr();
    let value_ptr = sp.add(base + value_reg as usize);
    let upvalue_ptr = *ci.upvalue_ptrs.add(env_upvalue as usize);
    let env_value = upvalue_ptr.as_ref().data.get_value_ref();

    if !env_value.is_table() {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let env_table = env_value.hvalue();
    if !env_table.impl_table.has_hash() || !env_table.impl_table.get_shortstr_into(table_key, value_ptr) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }
    if !(*value_ptr).is_table() {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let table = (*value_ptr).hvalue();
    let Some(field_slot) = table.impl_table.find_existing_shortstr_slot(field_key) else {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    };
    if !table.impl_table.shortstr_slot_into(field_slot, value_ptr) {
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let constant_value = *value_ptr;
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let value_ptr = sp.add(base + value_reg as usize);
        *value_ptr = constant_value;

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_try_eval_math_floor_const(arg: &LuaValue) -> Option<LuaValue> {
    if let Some(i) = arg.as_integer() {
        return Some(LuaValue::integer(i));
    }

    let f = arg.as_float()?;
    let floored = f.floor();
    if floored >= (i64::MIN as f64) && floored < -(i64::MIN as f64) {
        Some(LuaValue::integer(floored as i64))
    } else {
        Some(LuaValue::float(floored))
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_builtin_unary_const_call(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    func_reg: u32,
    result_reg: u32,
    arg_const: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let arg = constants.get(arg_const as usize)?;
    let result = jit_try_eval_math_floor_const(arg)?;
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let func_ptr = sp.add(base + func_reg as usize);
        let result_ptr = sp.add(base + result_reg as usize);

        let Some(c_func) = (*func_ptr).as_cfunction() else {
            jit::blacklist_trace(
                lua_state,
                ci.chunk_ptr,
                target_pc,
                jit::TraceAbortReason::RuntimeGuardRejected,
            );
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };

        if !std::ptr::fn_addr_eq(c_func, crate::stdlib::math::math_floor as crate::lua_vm::CFunction) {
            jit::blacklist_trace(
                lua_state,
                ci.chunk_ptr,
                target_pc,
                jit::TraceAbortReason::RuntimeGuardRejected,
            );
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        *result_ptr = result;

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_try_eval_string_lower(lua_state: &mut LuaState, arg: &LuaValue) -> Option<LuaValue> {
    let s = arg.as_str()?;
    let bytes = s.as_bytes();
    let len = bytes.len();

    if len <= 256 {
        let mut buf = [0u8; 256];
        buf[..len].copy_from_slice(bytes);
        buf[..len].make_ascii_lowercase();
        let result_str = unsafe { std::str::from_utf8_unchecked(&buf[..len]) };
        lua_state.create_string(result_str).ok()
    } else {
        let mut buf = bytes.to_vec();
        buf.make_ascii_lowercase();
        let result_str = unsafe { String::from_utf8_unchecked(buf) };
        lua_state.create_string_owned(result_str).ok()
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_try_eval_string_upper(lua_state: &mut LuaState, arg: &LuaValue) -> Option<LuaValue> {
    let s = arg.as_str()?;
    let bytes = s.as_bytes();
    let len = bytes.len();

    if len <= 256 {
        let mut buf = [0u8; 256];
        buf[..len].copy_from_slice(bytes);
        buf[..len].make_ascii_uppercase();
        let result_str = unsafe { std::str::from_utf8_unchecked(&buf[..len]) };
        lua_state.create_string(result_str).ok()
    } else {
        let mut buf = bytes.to_vec();
        buf.make_ascii_uppercase();
        let result_str = unsafe { String::from_utf8_unchecked(buf) };
        lua_state.create_string_owned(result_str).ok()
    }
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_try_eval_string_reverse(lua_state: &mut LuaState, arg: &LuaValue) -> Option<LuaValue> {
    let s_bytes = arg.as_bytes()?;
    let len = s_bytes.len();
    let is_ascii = s_bytes.is_ascii();

    if len <= 256 {
        let mut buf = [0u8; 256];
        buf[..len].copy_from_slice(s_bytes);
        buf[..len].reverse();
        if is_ascii {
            lua_state
                .create_string(unsafe { std::str::from_utf8_unchecked(&buf[..len]) })
                .ok()
        } else {
            lua_state.create_bytes(&buf[..len]).ok()
        }
    } else {
        let mut reversed = s_bytes.to_vec();
        reversed.reverse();
        if is_ascii {
            lua_state
                .create_string_owned(unsafe { String::from_utf8_unchecked(reversed) })
                .ok()
        } else {
            lua_state.create_bytes(&reversed).ok()
        }
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_tabup_field_string_unary_call(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    env_upvalue: u32,
    result_reg: u32,
    arg_reg: u32,
    table_key_const: u32,
    field_key_const: u32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let table_key = constants.get(table_key_const as usize)?;
    let field_key = constants.get(field_key_const as usize)?;

    if !table_key.is_short_string() || !field_key.is_short_string() {
        return None;
    }

    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let result_ptr = sp.add(base + result_reg as usize);
        let arg_value = *sp.add(base + arg_reg as usize);
        let upvalue_ptr = *ci.upvalue_ptrs.add(env_upvalue as usize);
        let env_value = upvalue_ptr.as_ref().data.get_value_ref();

        if !env_value.is_table() {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let env_table = env_value.hvalue();
        if !env_table.impl_table.has_hash()
            || !env_table.impl_table.get_shortstr_into(table_key, result_ptr)
        {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }
        if !(*result_ptr).is_table() {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let table = (*result_ptr).hvalue();
        let Some(field_slot) = table.impl_table.find_existing_shortstr_slot(field_key) else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };
        if !table.impl_table.shortstr_slot_into(field_slot, result_ptr) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let Some(c_func) = (*result_ptr).as_cfunction() else {
            jit::blacklist_trace(
                lua_state,
                ci.chunk_ptr,
                target_pc,
                jit::TraceAbortReason::RuntimeGuardRejected,
            );
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };

        let result = if std::ptr::fn_addr_eq(
            c_func,
            crate::stdlib::string::string_upper as crate::lua_vm::CFunction,
        ) {
            jit_try_eval_string_upper(lua_state, &arg_value)
        } else if std::ptr::fn_addr_eq(
            c_func,
            crate::stdlib::string::string_lower as crate::lua_vm::CFunction,
        ) {
            jit_try_eval_string_lower(lua_state, &arg_value)
        } else if std::ptr::fn_addr_eq(
            c_func,
            crate::stdlib::string::string_reverse as crate::lua_vm::CFunction,
        ) {
            jit_try_eval_string_reverse(lua_state, &arg_value)
        } else {
            jit::blacklist_trace(
                lua_state,
                ci.chunk_ptr,
                target_pc,
                jit::TraceAbortReason::RuntimeGuardRejected,
            );
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };

        let Some(result) = result else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };

        *result_ptr = result;

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_lua_closure_addi(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_reg: u32,
    func_reg: u32,
    arg_reg: u32,
    dst_reg: u32,
    imm: i32,
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let sp = lua_state.stack_mut().as_mut_ptr();
    let func_ptr = sp.add(base + func_reg as usize);
    let Some(function) = (*func_ptr).as_lua_function() else {
        jit::blacklist_trace(
            lua_state,
            ci.chunk_ptr,
            target_pc,
            jit::TraceAbortReason::RuntimeGuardRejected,
        );
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    };

    let chunk = function.chunk();
    let code = &chunk.code;
    if function.upvalues().len() != 0
        || code.len() < 3
        || code[0].get_opcode() != OpCode::Add
        || code[1].get_opcode() != OpCode::MmBin
        || code[2].get_opcode() != OpCode::Return1
    {
        jit::blacklist_trace(
            lua_state,
            ci.chunk_ptr,
            target_pc,
            jit::TraceAbortReason::RuntimeGuardRejected,
        );
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let add_inst = code[0];
    if add_inst.get_a() != 2 || add_inst.get_b() != 0 || add_inst.get_c() != 1 {
        jit::blacklist_trace(
            lua_state,
            ci.chunk_ptr,
            target_pc,
            jit::TraceAbortReason::RuntimeGuardRejected,
        );
        jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, 0, summary);
        return None;
    }

    let mut trace_hits = 0u32;
    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let arg_ptr = sp.add(base + arg_reg as usize);
        let dst_ptr = sp.add(base + dst_reg as usize);

        if !pttisinteger(arg_ptr as *const LuaValue) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        psetivalue(dst_ptr, pivalue(arg_ptr as *const LuaValue).wrapping_add(imm as i64));

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
const JIT_NUMERIC_ARRAY_COPY_OPS: [OpCode; 11] = [
    OpCode::NewTable,
    OpCode::ExtraArg,
    OpCode::LoadI,
    OpCode::Len,
    OpCode::LoadI,
    OpCode::ForPrep,
    OpCode::GetTable,
    OpCode::SetTable,
    OpCode::ForLoop,
    OpCode::Return1,
    OpCode::Return0,
];

#[cfg(feature = "jit")]
const JIT_NUMERIC_ARRAY_CHECKSUM_OPS: [OpCode; 15] = [
    OpCode::LoadI,
    OpCode::LoadI,
    OpCode::Len,
    OpCode::LoadI,
    OpCode::ForPrep,
    OpCode::GetTable,
    OpCode::Mul,
    OpCode::MmBin,
    OpCode::Add,
    OpCode::MmBin,
    OpCode::ModK,
    OpCode::MmBinK,
    OpCode::ForLoop,
    OpCode::Return1,
    OpCode::Return0,
];

#[cfg(feature = "jit")]
const JIT_NUMERIC_ARRAY_VALIDATE_SORTED_OPS: [OpCode; 16] = [
    OpCode::LoadI,
    OpCode::Len,
    OpCode::LoadI,
    OpCode::ForPrep,
    OpCode::AddI,
    OpCode::MmBinI,
    OpCode::GetTable,
    OpCode::GetTable,
    OpCode::Lt,
    OpCode::Jmp,
    OpCode::LoadFalse,
    OpCode::Return1,
    OpCode::ForLoop,
    OpCode::LoadTrue,
    OpCode::Return1,
    OpCode::Return0,
];

#[cfg(feature = "jit")]
const JIT_NUMERIC_ARRAY_RECURSIVE_SORT_OPS: [OpCode; 42] = [
    OpCode::Lt,
    OpCode::Jmp,
    OpCode::Sub,
    OpCode::MmBin,
    OpCode::LeI,
    OpCode::Jmp,
    OpCode::GetUpval,
    OpCode::Move,
    OpCode::Move,
    OpCode::Move,
    OpCode::Call,
    OpCode::Return0,
    OpCode::GetUpval,
    OpCode::Move,
    OpCode::Move,
    OpCode::Move,
    OpCode::Call,
    OpCode::Sub,
    OpCode::MmBin,
    OpCode::Sub,
    OpCode::MmBin,
    OpCode::Lt,
    OpCode::Jmp,
    OpCode::Lt,
    OpCode::Jmp,
    OpCode::GetUpval,
    OpCode::Move,
    OpCode::Move,
    OpCode::Move,
    OpCode::Call,
    OpCode::Move,
    OpCode::Jmp,
    OpCode::Lt,
    OpCode::Jmp,
    OpCode::GetUpval,
    OpCode::Move,
    OpCode::Move,
    OpCode::Move,
    OpCode::Call,
    OpCode::Move,
    OpCode::Jmp,
    OpCode::Return0,
];

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_proto_has_opcodes(
    function: &crate::lua_value::LuaFunction,
    expected_upvalues: usize,
    expected: &[OpCode],
) -> bool {
    if function.upvalues().len() != expected_upvalues {
        return false;
    }

    let chunk = function.chunk();
    chunk.code.len() == expected.len()
        && chunk
            .code
            .iter()
            .zip(expected.iter())
            .all(|(inst, opcode)| inst.get_opcode() == *opcode)
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_plain_numeric_table_guard(table: &crate::LuaTable) -> bool {
    let meta = table.meta_ptr();
    meta.is_null()
        || (meta.as_ref().data.no_tm(TmKind::Index.into())
            && meta.as_ref().data.no_tm(TmKind::NewIndex.into()))
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn jit_collect_numeric_table_values(table: &crate::LuaTable) -> Option<Vec<i64>> {
    if !jit_plain_numeric_table_guard(table) {
        return None;
    }

    let len = table.len();
    let mut values = Vec::with_capacity(len);
    for index in 1..=len {
        let mut loaded = LuaValue::nil();
        let key = index as i64;
        let found = table.impl_table.fast_geti_into(key, &mut loaded)
            || table.impl_table.get_int_from_hash_into(key, &mut loaded);
        if !found {
            return None;
        }
        values.push(loaded.as_integer_strict()?);
    }
    Some(values)
}

#[cfg(feature = "jit")]
#[inline(always)]
fn jit_is_strictly_sorted(values: &[i64]) -> bool {
    values.windows(2).all(|pair| pair[0] <= pair[1])
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn jit_store_numeric_table_values(
    lua_state: &mut LuaState,
    values: &[i64],
) -> Option<LuaValue> {
    let table_value = lua_state.create_table(values.len(), 0).ok()?;
    for (index, value) in values.iter().enumerate() {
        if !lua_state.raw_seti(&table_value, (index + 1) as i64, LuaValue::integer(*value)) {
            return None;
        }
    }
    Some(table_value)
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_array_sort_validate_checksum_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_reg: u32,
    source_reg: u32,
    work_reg: u32,
    sum_reg: u32,
    copy_func_reg: u32,
    sort_func_reg: u32,
    check_func_reg: u32,
    checksum_func_reg: u32,
    modulo_const: u32,
    constants: &[LuaValue],
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let modulo = constants.get(modulo_const as usize)?.as_integer_strict()?;
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        let source_ptr = sp.add(base + source_reg as usize);
        let work_ptr = sp.add(base + work_reg as usize);
        let sum_ptr = sp.add(base + sum_reg as usize);
        let copy_func_ptr = sp.add(base + copy_func_reg as usize);
        let sort_func_ptr = sp.add(base + sort_func_reg as usize);
        let check_func_ptr = sp.add(base + check_func_reg as usize);
        let checksum_func_ptr = sp.add(base + checksum_func_reg as usize);

        let Some(copy_func) = (*copy_func_ptr).as_lua_function() else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };
        let Some(sort_func) = (*sort_func_ptr).as_lua_function() else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };
        let Some(check_func) = (*check_func_ptr).as_lua_function() else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };
        let Some(checksum_func) = (*checksum_func_ptr).as_lua_function() else {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };

        if !jit_proto_has_opcodes(copy_func, 0, &JIT_NUMERIC_ARRAY_COPY_OPS)
            || !jit_proto_has_opcodes(sort_func, 3, &JIT_NUMERIC_ARRAY_RECURSIVE_SORT_OPS)
            || !jit_proto_has_opcodes(check_func, 0, &JIT_NUMERIC_ARRAY_VALIDATE_SORTED_OPS)
            || !jit_proto_has_opcodes(checksum_func, 0, &JIT_NUMERIC_ARRAY_CHECKSUM_OPS)
            || !(*source_ptr).is_table()
            || !pttisinteger(sum_ptr as *const LuaValue)
        {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let source_table = (*source_ptr).hvalue();
        let mut values = match jit_collect_numeric_table_values(&source_table) {
            Some(values) => values,
            None => {
                jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                return None;
            }
        };

        values.sort_unstable();
        if !jit_is_strictly_sorted(&values) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let work_value = match jit_store_numeric_table_values(lua_state, &values) {
            Some(value) => value,
            None => {
                jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                return None;
            }
        };

        let mut checksum = 0i64;
        for (offset, value) in values.iter().enumerate() {
            let index = (offset + 1) as i64;
            checksum = lua_imod(checksum.wrapping_add(value.wrapping_mul(index)), modulo);
        }
        let sum = pivalue(sum_ptr as *const LuaValue);

        *work_ptr = work_value;
        psetivalue(sum_ptr, lua_imod(sum.wrapping_add(checksum), modulo));

        trace_hits = trace_hits.saturating_add(1);
        let remaining = pivalue(loop_ptr as *const LuaValue) as u64;
        if remaining > 0 {
            let step_val = pivalue(loop_ptr.add(1) as *const LuaValue);
            let idx = pivalue(index_ptr as *const LuaValue);
            (*loop_ptr).value.i = remaining as i64 - 1;
            (*index_ptr).value.i = idx.wrapping_add(step_val);
            continue;
        }

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_linear_int_for_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    loop_reg: u32,
    steps: &[jit::LinearIntStep],
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        if !jit_execute_linear_int_steps(sp, base, steps) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
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

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_for_loop(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    loop_reg: u32,
    steps: &[jit::NumericStep],
    summary: jit::HelperPlanDispatchSummary,
    exit_pc: usize,
) -> Option<usize> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);
        if !jit_execute_numeric_steps(sp, base, constants, steps) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
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

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_execute_numeric_ifelse_for_loop(
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
) -> Option<usize> {
    let mut trace_hits = 0u32;

    loop {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let loop_ptr = sp.add(base + loop_reg as usize);
        let index_ptr = loop_ptr.add(2);

        if !jit_execute_numeric_steps(sp, base, constants, pre_steps) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let cond_holds = jit_numeric_ifelse_cond_holds(sp, base, cond);
        let take_then = cond_holds == then_on_true;
        let branch_preset = if take_then { then_preset.as_ref() } else { else_preset.as_ref() };
        if let Some(step) = branch_preset
            && !jit_execute_numeric_steps(sp, base, constants, std::slice::from_ref(step))
        {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let branch_steps = if take_then { then_steps } else { else_steps };
        if !jit_execute_numeric_steps(sp, base, constants, branch_steps) {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
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

        jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
        return Some(exit_pc);
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_try_handle_forloop_backedge(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    constants: &[LuaValue],
    target_pc: usize,
    exit_pc: usize,
) -> Option<usize> {
    let Some((executor, summary)) = jit::compiled_trace_executor_or_record(lua_state, ci.chunk_ptr, target_pc) else {
        return None;
    };

    match executor {
        jit::CompiledTraceExecutor::NumericForGetTableAdd {
            loop_reg,
            table_reg,
            index_reg,
            value_reg,
            acc_reg,
        } => unsafe {
            jit_execute_numeric_for_gettable_add(
                lua_state,
                ci,
                base,
                target_pc,
                loop_reg,
                table_reg,
                index_reg,
                value_reg,
                acc_reg,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForTableMulAddMod {
            loop_reg,
            table_reg,
            index_reg,
            acc_reg,
            modulo_const,
        } => unsafe {
            jit_execute_numeric_for_table_mul_add_mod(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                loop_reg,
                table_reg,
                index_reg,
                acc_reg,
                modulo_const,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForTableCopy {
            loop_reg,
            src_table_reg,
            dst_table_reg,
            index_reg,
        } => unsafe {
            jit_execute_numeric_for_table_copy(
                lua_state,
                ci,
                base,
                target_pc,
                loop_reg,
                src_table_reg,
                dst_table_reg,
                index_reg,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForTableIsSorted {
            loop_reg,
            table_reg,
            index_reg,
            false_exit_pc,
        } => unsafe {
            jit_execute_numeric_for_table_is_sorted(
                lua_state,
                ci,
                base,
                target_pc,
                loop_reg,
                table_reg,
                index_reg,
                false_exit_pc as usize,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForArraySortValidateChecksumLoop {
            loop_reg,
            source_reg,
            work_reg,
            sum_reg,
            copy_func_reg,
            sort_func_reg,
            check_func_reg,
            checksum_func_reg,
            modulo_const,
        } => unsafe {
            jit_execute_numeric_for_array_sort_validate_checksum_loop(
                lua_state,
                ci,
                base,
                target_pc,
                loop_reg,
                source_reg,
                work_reg,
                sum_reg,
                copy_func_reg,
                sort_func_reg,
                check_func_reg,
                checksum_func_reg,
                modulo_const,
                constants,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForUpvalueAddI {
            loop_reg,
            upvalue,
            value_reg,
            imm,
        } => unsafe {
            jit_execute_numeric_for_upvalue_addi(
                lua_state,
                ci,
                base,
                target_pc,
                loop_reg,
                upvalue,
                value_reg,
                imm,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForFieldAddI {
            loop_reg,
            table_reg,
            value_reg,
            key_const,
            imm,
        } => unsafe {
            jit_execute_numeric_for_field_addi(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                loop_reg,
                table_reg,
                value_reg,
                key_const,
                imm,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForTabUpAddI {
            loop_reg,
            env_upvalue,
            value_reg,
            key_const,
            imm,
        } => unsafe {
            jit_execute_numeric_for_tabup_addi(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                loop_reg,
                env_upvalue,
                value_reg,
                key_const,
                imm,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForTabUpFieldAddI {
            loop_reg,
            env_upvalue,
            table_reg,
            value_reg,
            table_key_const,
            field_key_const,
            imm,
        } => unsafe {
            jit_execute_numeric_for_tabup_field_addi(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                loop_reg,
                env_upvalue,
                table_reg,
                value_reg,
                table_key_const,
                field_key_const,
                imm,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForTabUpFieldLoad {
            loop_reg,
            env_upvalue,
            value_reg,
            table_key_const,
            field_key_const,
        } => unsafe {
            jit_execute_numeric_for_tabup_field_load(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                loop_reg,
                env_upvalue,
                value_reg,
                table_key_const,
                field_key_const,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForBuiltinUnaryConstCall {
            loop_reg,
            func_reg,
            result_reg,
            arg_const,
        } => unsafe {
            jit_execute_numeric_for_builtin_unary_const_call(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                loop_reg,
                func_reg,
                result_reg,
                arg_const,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForTabUpFieldStringUnaryCall {
            loop_reg,
            env_upvalue,
            result_reg,
            arg_reg,
            table_key_const,
            field_key_const,
        } => unsafe {
            jit_execute_numeric_for_tabup_field_string_unary_call(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                loop_reg,
                env_upvalue,
                result_reg,
                arg_reg,
                table_key_const,
                field_key_const,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForLuaClosureAddI {
            loop_reg,
            func_reg,
            arg_reg,
            dst_reg,
            imm,
        } => unsafe {
            jit_execute_numeric_for_lua_closure_addi(
                lua_state,
                ci,
                base,
                target_pc,
                loop_reg,
                func_reg,
                arg_reg,
                dst_reg,
                imm,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::LinearIntForLoop { loop_reg, steps } => unsafe {
            jit_execute_linear_int_for_loop(
                lua_state,
                ci,
                base,
                target_pc,
                loop_reg,
                &steps,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericForLoop { loop_reg, steps } => unsafe {
            jit_execute_numeric_for_loop(
                lua_state,
                ci,
                base,
                constants,
                target_pc,
                loop_reg,
                &steps,
                summary,
                exit_pc,
            )
        },
        jit::CompiledTraceExecutor::NumericIfElseForLoop {
            loop_reg,
            pre_steps,
            cond,
            then_preset,
            else_preset,
            then_steps,
            else_steps,
            then_on_true,
        } => unsafe {
            jit_execute_numeric_ifelse_for_loop(
                lua_state,
                ci,
                base,
                constants,
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
        _ => {
            jit::record_loop_backedge(lua_state, ci.chunk_ptr, target_pc);
            None
        }
    }
}

#[cfg(feature = "jit")]
#[allow(unsafe_op_in_unsafe_fn)]
#[inline(always)]
unsafe fn jit_try_handle_tforloop_backedge(
    lua_state: &mut LuaState,
    ci: &CallInfo,
    base: usize,
    target_pc: usize,
    exit_pc: usize,
) -> Option<usize> {
    let Some((executor, summary)) = jit::compiled_trace_executor_or_record(lua_state, ci.chunk_ptr, target_pc) else {
        return None;
    };

    let jit::CompiledTraceExecutor::GenericForBuiltinAdd {
        tfor_reg,
        value_reg,
        acc_reg,
    } = executor else {
        jit::record_loop_backedge(lua_state, ci.chunk_ptr, target_pc);
        return None;
    };

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
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        };

        if !pttisinteger(acc_ptr as *const LuaValue) || !pttisinteger(value_ptr as *const LuaValue)
        {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        psetivalue(
            acc_ptr,
            pivalue(acc_ptr as *const LuaValue).wrapping_add(pivalue(value_ptr as *const LuaValue)),
        );
        trace_hits = trace_hits.saturating_add(1);

        if is_ipairs {
            if !(*state_ptr).is_table() || !pttisinteger(control_ptr as *const LuaValue) {
                jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                return None;
            }

            let table = (*state_ptr).hvalue();
            if table.has_metatable() {
                jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                return None;
            }

            let next_index = pivalue(control_ptr as *const LuaValue).wrapping_add(1);
            let loaded = table.impl_table.fast_geti_into(next_index, value_ptr)
                || table.impl_table.get_int_from_hash_into(next_index, value_ptr);
            if !loaded {
                setnilvalue(&mut *control_ptr);
                setnilvalue(&mut *value_ptr);
                jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
                return Some(exit_pc);
            }

            psetivalue(control_ptr, next_index);
            if !pttisinteger(value_ptr as *const LuaValue) {
                jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
                return None;
            }
            continue;
        }

        if !(*state_ptr).is_table() {
            jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
            return None;
        }

        let table = (*state_ptr).hvalue();
        let current_key = *control_ptr;
        match jit_lua_next_into(table, &current_key, control_ptr, value_ptr) {
            Ok(true) => {
                if !pttisinteger(value_ptr as *const LuaValue) {
                    jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
                    return None;
                }
            }
            Ok(false) => {
                setnilvalue(&mut *control_ptr);
                setnilvalue(&mut *value_ptr);
                jit::record_batched_trace_execution(lua_state, trace_hits, trace_hits, summary);
                return Some(exit_pc);
            }
            Err(()) => {
                jit_record_trace_hits_or_fallback(lua_state, ci.chunk_ptr, target_pc, trace_hits, summary);
                return None;
            }
        }
    }
}

/// Execute until call depth reaches target_depth
/// Used for protected calls (pcall) to execute only the called function
/// without affecting caller frames
///
/// NOTE: n_ccalls tracking is NOT done here (unlike the wrapper approach).
/// Instead, each recursive CALL SITE (metamethods, pcall, resume, __close)
/// increments/decrements n_ccalls around its call to lua_execute, mirroring
/// Lua 5.5's luaD_call pattern.
pub fn lua_execute(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    // STARTFUNC: Function context switching point (like Lua C's startfunc label)
    'startfunc: loop {
        // Check if we've returned past target depth.
        let current_depth = lua_state.call_depth();
        if current_depth <= target_depth {
            return Ok(());
        }

        let frame_idx = current_depth - 1;
        let ci_ptr = unsafe { lua_state.get_call_info_ptr(frame_idx) } as *mut CallInfo;
        let mut ci = unsafe { &mut *ci_ptr };
        let call_status = ci.call_status;
        if call_status & (CIST_C | CIST_PENDING_FINISH) != 0 && handle_pending_ops(lua_state, ci)? {
            continue 'startfunc;
        }

        let mut base = ci.base;
        let pc_init = ci.pc as usize;
        let mut chunk = unsafe { &*ci.chunk_ptr };
        debug_assert!(lua_state.stack_len() >= base + chunk.max_stack_size + EXTRA_STACK);

        let mut code: &[Instruction] = &chunk.code;
        let mut constants: &[LuaValue] = &chunk.constants;
        let mut pc: usize = pc_init;

        if lua_state.hook_mask & LUA_MASKLINE != 0 {
            lua_state.oldpc = if pc_init > 0 {
                (pc_init - 1) as u32
            } else if chunk.is_vararg {
                0
            } else {
                u32::MAX
            };
        }

        // CALL HOOK: fire when entering a new Lua function (pc == 0)
        #[cfg(not(feature = "sandbox"))]
        let mut trap = lua_state.hook_mask != 0;

        #[cfg(feature = "sandbox")]
        let mut trap = lua_state.has_active_instruction_watch();
        if pc == 0 && trap {
            let hook_mask = lua_state.hook_mask;
            if hook_mask & LUA_MASKCALL != 0 && lua_state.allow_hook {
                hook_on_call(lua_state, hook_mask, call_status, chunk)?;
            }
            if hook_mask & LUA_MASKCOUNT != 0 {
                lua_state.hook_count = lua_state.base_hook_count;
            }
        }

        macro_rules! stack_id {
            ($a:expr) => {
                base + $a as usize
            };
        }

        macro_rules! stack_val_mut {
            ($a:expr) => {
                unsafe { lua_state.stack_mut().get_unchecked_mut(stack_id!($a)) }
            };
        }

        macro_rules! stack_val {
            ($a:expr) => {
                unsafe { lua_state.stack().get_unchecked(stack_id!($a)) }
            };
        }

        macro_rules! k_val {
            ($a:expr) => {
                unsafe { constants.get_unchecked($a as usize) }
            };
        }

        macro_rules! updatetrap {
            () => {
                #[cfg(not(feature = "sandbox"))]
                {
                    trap = lua_state.hook_mask != 0;
                }

                #[cfg(feature = "sandbox")]
                {
                    trap = lua_state.has_active_instruction_watch();
                }
            };
        }

        macro_rules! updatebase {
            () => {
                base = ci.base;
            };
        }

        macro_rules! savestate {
            () => {
                ci.save_pc(pc);
                lua_state.set_top_raw(ci.top as usize);
            };
        }

        macro_rules! resume_caller_fast {
            () => {{
                let new_depth = lua_state.call_depth();
                if new_depth <= target_depth {
                    return Ok(());
                }
                let new_fi = new_depth - 1;
                let ci_ptr = unsafe { lua_state.get_call_info_ptr(new_fi) } as *mut CallInfo;
                ci = unsafe { &mut *ci_ptr };
                if ci.call_status & (CIST_C | CIST_PENDING_FINISH) != 0 {
                    continue 'startfunc;
                }
                base = ci.base;
                pc = ci.pc as usize;
                chunk = unsafe { &*ci.chunk_ptr };
                code = &chunk.code;
                constants = &chunk.constants;
                trap = lua_state.hook_mask != 0;
                if lua_state.hook_mask & LUA_MASKLINE != 0 {
                    lua_state.oldpc = if pc > 0 {
                        (pc - 1) as u32
                    } else if chunk.is_vararg {
                        0
                    } else {
                        u32::MAX
                    };
                }
                continue;
            }};
        }

        // MAINLOOP: Main instruction dispatch loop
        loop {
            let instr = unsafe { *code.get_unchecked(pc) }; // vmfetch
            pc += 1;

            if trap {
                trap = hook_check_instruction(lua_state, pc, chunk, ci)?;
                updatebase!();
            }

            match instr.get_opcode() {
                OpCode::Move => {
                    // R[A] := R[B]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    setobjs2s(lua_state, stack_id!(a), stack_id!(b));
                }
                OpCode::LoadI => {
                    // R[A] := sBx
                    let a = instr.get_a();
                    let sbx = instr.get_sbx();
                    setivalue(stack_val_mut!(a), sbx as i64);
                }
                OpCode::LoadF => {
                    // R[A] := (float)sBx
                    let a = instr.get_a();
                    let sbx = instr.get_sbx();
                    setfltvalue(stack_val_mut!(a), sbx as f64);
                }
                OpCode::LoadK => {
                    // R[A] := K[Bx]
                    let a = instr.get_a();
                    let bx = instr.get_bx();
                    setobj2s(lua_state, stack_id!(a), k_val!(bx));
                }
                OpCode::LoadKX => {
                    // R[A] := K[extra arg]
                    let a = instr.get_a();
                    let next_instr = unsafe { *code.get_unchecked(pc) };
                    debug_assert_eq!(next_instr.get_opcode(), OpCode::ExtraArg);
                    let rb = next_instr.get_ax();
                    pc += 1;
                    setobj2s(lua_state, stack_id!(a), k_val!(rb));
                }
                OpCode::LoadFalse => {
                    // R[A] := false
                    let a = instr.get_a();
                    setbfvalue(stack_val_mut!(a));
                }
                OpCode::LFalseSkip => {
                    // R[A] := false; pc++
                    let a = instr.get_a();
                    setbfvalue(stack_val_mut!(a));
                    pc += 1; // Skip next instruction
                }
                OpCode::LoadTrue => {
                    // R[A] := true
                    let a = instr.get_a();
                    setbtvalue(stack_val_mut!(a));
                }
                OpCode::LoadNil => {
                    // R[A], R[A+1], ..., R[A+B] := nil
                    let mut a = instr.get_a();
                    let mut b = instr.get_b();
                    loop {
                        setnilvalue(stack_val_mut!(a));
                        if b == 0 {
                            break;
                        }
                        b -= 1;
                        a += 1;
                    }
                }
                OpCode::GetUpval => {
                    // R[A] := UpValue[B]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(b as usize) };
                    let src = upvalue_ptr.as_ref().data.get_value_ref();
                    let dest = unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                    unsafe {
                        (*dest).value = src.value;
                        (*dest).tt = src.tt;
                    }
                }
                OpCode::SetUpval => {
                    // UpValue[B] := R[A]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    unsafe {
                        let upvalue_ptr = *ci.upvalue_ptrs.add(b as usize);
                        let value = lua_state.stack().get_unchecked(base + a as usize);
                        upvalue_ptr
                            .as_mut_ref()
                            .data
                            .set_value_parts(value.value, value.tt);

                        // GC barrier (only for collectable values)
                        if value.tt & 0x40 != 0
                            && let Some(gc_ptr) = value.as_gc_ptr()
                        {
                            lua_state.gc_barrier(upvalue_ptr, gc_ptr);
                        }
                    }
                }
                OpCode::GetTabUp => {
                    // R[A] := UpValue[B][K[C]:shortstring]
                    let a = instr.get_a();
                    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(instr.get_b() as usize) };
                    let upval_value = upvalue_ptr.as_ref().data.get_value_ref();
                    let key = k_val!(instr.get_c());
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    if upval_value.is_table() {
                        let table = upval_value.hvalue();
                        if !trap {
                            let next_instr = unsafe { *code.get_unchecked(pc) };
                            if next_instr.get_opcode() == OpCode::GetField
                                && next_instr.get_b() == a
                            {
                                let next_key = k_val!(next_instr.get_c());
                                debug_assert!(
                                    next_key.is_short_string(),
                                    "GetField key must be short string for fast path"
                                );

                                if let Some(outer) = table.impl_table.get_shortstr_fast(key) {
                                    if outer.is_table() {
                                        let inner_table = outer.hvalue();
                                        if inner_table.impl_table.has_hash() {
                                            let dest = unsafe {
                                                lua_state.stack_mut().as_mut_ptr().add(stack_id!(a))
                                            };
                                            if unsafe {
                                                inner_table
                                                    .impl_table
                                                    .get_shortstr_into(next_key, dest)
                                            } {
                                                pc += 1;
                                                continue;
                                            }
                                        }
                                    }

                                    setobj2s(lua_state, stack_id!(a), &outer);
                                    continue;
                                }
                            }
                        }

                        if table.impl_table.has_hash() {
                            let dest =
                                unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                            if unsafe { table.impl_table.get_shortstr_into(key, dest) } {
                                continue;
                            }
                        }
                    }
                    savestate!();
                    let upval_value = *upval_value;
                    finishget_fallback(lua_state, ci, &upval_value, key, stack_id!(a))?;
                    updatetrap!();
                }
                OpCode::GetTable => {
                    // GETTABLE: R[A] := R[B][R[C]]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();

                    let rb_ptr = unsafe { lua_state.stack().as_ptr().add(stack_id!(b)) };

                    if unsafe { (*rb_ptr).is_table() } {
                        let table = unsafe { (*rb_ptr).hvalue() };
                        let rc_idx = stack_id!(c);
                        let rc_ptr = unsafe { lua_state.stack().as_ptr().add(rc_idx) };
                        let rc_tt = unsafe { (*rc_ptr).tt };
                        let dest = unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                        // Hot path 1: integer key → array fast path
                        if rc_tt == LUA_VNUMINT {
                            let key = unsafe { (*rc_ptr).value.i };
                            if unsafe { table.impl_table.fast_geti_into(key, dest) } {
                                continue;
                            }
                            if unsafe { table.impl_table.get_int_from_hash_into(key, dest) } {
                                continue;
                            }
                        }
                        // Hot path 2: short string key → hash fast path (zero-copy)
                        else if unsafe { (*rc_ptr).is_short_string() }
                            && table.impl_table.has_hash()
                            && unsafe { table.impl_table.get_shortstr_into(&*rc_ptr, dest) }
                        {
                            continue;
                        }
                        let rc = unsafe { *rc_ptr };
                        // Cold path: other key types, hash fallback for integers
                        if let Some(val) = table.impl_table.raw_get(&rc) {
                            setobj2s(lua_state, stack_id!(a), &val);
                            continue;
                        }

                        savestate!();
                        let rb = unsafe { *rb_ptr };
                        finishget_fallback(lua_state, ci, &rb, &rc, stack_id!(a))?;
                        updatetrap!();
                        continue;
                    }

                    let rb = unsafe { *rb_ptr };
                    let rc = *unsafe { lua_state.stack().get_unchecked(stack_id!(c)) };

                    // Metamethod / non-table fallback
                    savestate!();
                    finishget_fallback(lua_state, ci, &rb, &rc, stack_id!(a))?;
                    updatetrap!();
                }
                OpCode::GetI => {
                    // GETI: R[A] := R[B][C] (integer key)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let rc = instr.get_c() as i64;
                    let rb = *stack_val!(b);
                    if rb.is_table() {
                        let table = rb.hvalue();
                        let dest = unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                        // fast_geti: try array part first
                        let found = unsafe { table.impl_table.fast_geti_into(rc, dest) };
                        if found {
                            continue;
                        }
                        // fallback: direct integer hash lookup (no float/array re-check)
                        let found = unsafe { table.impl_table.get_int_from_hash_into(rc, dest) };
                        if found {
                            continue;
                        }
                    }

                    savestate!();
                    finishget_fallback(lua_state, ci, &rb, &LuaValue::integer(rc), stack_id!(a))?;
                    updatetrap!();
                }
                OpCode::GetField => {
                    // GETFIELD: R[A] := R[B][K[C]:string]
                    let rb_ptr =
                        unsafe { lua_state.stack().as_ptr().add(stack_id!(instr.get_b())) };
                    let key = k_val!(instr.get_c());
                    debug_assert!(
                        key.is_short_string(),
                        "GetField key must be short string for fast path"
                    );
                    if unsafe { (*rb_ptr).is_table() } {
                        let table = unsafe { (*rb_ptr).hvalue() };
                        if table.impl_table.has_hash() {
                            let dest = unsafe {
                                lua_state
                                    .stack_mut()
                                    .as_mut_ptr()
                                    .add(stack_id!(instr.get_a()))
                            };
                            if unsafe { table.impl_table.get_shortstr_into(key, dest) } {
                                continue;
                            }
                        }
                    }
                    savestate!();
                    let rb = unsafe { *rb_ptr };
                    finishget_fallback(lua_state, ci, &rb, key, stack_id!(instr.get_a()))?;
                    updatetrap!();
                }
                OpCode::SetTabUp => {
                    // UpValue[A][K[B]:shortstring] := RK(C)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();
                    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(a as usize) };
                    let upval_value = upvalue_ptr.as_ref().data.get_value_ref();
                    let key = k_val!(b);
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    let mut known_newindex_miss = false;
                    let mut meta = TablePtr::null();
                    if upval_value.is_table() {
                        let table = upval_value.hvalue_mut();
                        let table_ptr = unsafe { upval_value.as_table_ptr_unchecked() };
                        let gc_ptr = unsafe { upval_value.as_gc_ptr_table_unchecked() };
                        meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let (new_key, delta, rc_tt) = if instr.get_k() {
                                let rc = *k_val!(c);
                                if table.impl_table.set_existing_shortstr(key, rc) {
                                    if rc.is_collectable() {
                                        lua_state.gc_barrier_back(gc_ptr);
                                    }
                                    continue;
                                }
                                let pset_result = table.impl_table.pset_shortstr(key, rc);
                                let (new_key, delta) =
                                    table.impl_table.finish_shortstr_set(key, rc, pset_result);
                                (new_key, delta, rc.tt)
                            } else {
                                let rc_ptr =
                                    unsafe { lua_state.stack().as_ptr().add(stack_id!(c)) };
                                let rc_tt = unsafe { (*rc_ptr).tt };
                                let rc_value = unsafe { (*rc_ptr).value };
                                if table
                                    .impl_table
                                    .set_existing_shortstr_parts(key, rc_value, rc_tt)
                                {
                                    if rc_tt & 0x40 != 0 {
                                        lua_state.gc_barrier_back(gc_ptr);
                                    }
                                    continue;
                                }
                                let pset_result =
                                    table.impl_table.pset_shortstr_parts(key, rc_value, rc_tt);
                                let (new_key, delta) = table.impl_table.finish_shortstr_set_parts(
                                    key,
                                    rc_value,
                                    rc_tt,
                                    pset_result,
                                );
                                (new_key, delta, rc_tt)
                            };
                            if new_key {
                                table.invalidate_tm_cache();
                            }
                            if delta != 0 {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if rc_tt & 0x40 != 0 {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        } else {
                            let rc = if instr.get_k() {
                                *k_val!(c)
                            } else {
                                *unsafe { lua_state.stack().get_unchecked(stack_id!(c)) }
                            };
                            if table.impl_table.set_existing_shortstr(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            known_newindex_miss = true;
                        }
                    }

                    let upval_value = *upval_value;
                    let rc = if instr.get_k() {
                        *k_val!(c)
                    } else {
                        *unsafe { lua_state.stack().get_unchecked(stack_id!(c)) }
                    };
                    savestate!();
                    if known_newindex_miss {
                        if call_newindex_tm_fast(lua_state, ci, upval_value, meta, *key, rc)? {
                            updatetrap!();
                            continue;
                        }
                        finishset_fallback_known_miss(lua_state, ci, &upval_value, key, rc)?;
                    } else {
                        finishset_fallback(lua_state, ci, &upval_value, key, rc)?;
                    }
                    updatetrap!();
                }
                OpCode::SetTable => {
                    // SETTABLE: R[A][R[B]] := RK(C)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();
                    let ra_ptr = unsafe { lua_state.stack().as_ptr().add(stack_id!(a)) };
                    let rb_ptr = unsafe { lua_state.stack().as_ptr().add(stack_id!(b)) };

                    // Hot path: table + integer key in array range, no __newindex
                    // Deferred computation: table_ptr and gc barrier only when needed
                    if unsafe { (*ra_ptr).is_table() && (*rb_ptr).ttisinteger() } {
                        let table = unsafe { (*ra_ptr).hvalue_mut() };
                        let key = unsafe { (*rb_ptr).ivalue() };
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            if !instr.get_k() {
                                let rc_ptr =
                                    unsafe { lua_state.stack().as_ptr().add(stack_id!(c)) };
                                let rc_tt = unsafe { (*rc_ptr).tt };
                                let rc_value = unsafe { (*rc_ptr).value };
                                if table.impl_table.fast_seti_parts(key, rc_value, rc_tt) {
                                    if rc_tt & 0x40 != 0 {
                                        lua_state.gc_barrier_back(unsafe {
                                            (*ra_ptr).as_gc_ptr_table_unchecked()
                                        });
                                    }
                                    continue;
                                }

                                let rc = unsafe { *rc_ptr };
                                let delta = table.impl_table.set_int_slow(key, rc);
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(
                                        unsafe { (*ra_ptr).as_table_ptr_unchecked() },
                                        delta,
                                    );
                                }
                                if rc_tt & 0x40 != 0 {
                                    lua_state.gc_barrier_back(unsafe {
                                        (*ra_ptr).as_gc_ptr_table_unchecked()
                                    });
                                }
                                continue;
                            }

                            let rc = *k_val!(c);
                            if table.impl_table.fast_seti(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(unsafe {
                                        (*ra_ptr).as_gc_ptr_table_unchecked()
                                    });
                                }
                                continue;
                            }

                            let delta = table.impl_table.set_int_slow(key, rc);
                            if delta != 0 {
                                lua_state.gc_track_table_resize(
                                    unsafe { (*ra_ptr).as_table_ptr_unchecked() },
                                    delta,
                                );
                            }
                            if rc.is_collectable() {
                                lua_state.gc_barrier_back(unsafe {
                                    (*ra_ptr).as_gc_ptr_table_unchecked()
                                });
                            }
                            continue;
                        } else {
                            let rc = if instr.get_k() {
                                *k_val!(c)
                            } else {
                                *stack_val!(c)
                            };
                            if table.impl_table.set_existing_int(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(unsafe {
                                        (*ra_ptr).as_gc_ptr_table_unchecked()
                                    });
                                }
                                continue;
                            }
                            // Fall through to finishset_fallback_known_miss
                            let ra = unsafe { *ra_ptr };
                            let rb = unsafe { *rb_ptr };
                            let rc = if instr.get_k() {
                                *k_val!(c)
                            } else {
                                *stack_val!(c)
                            };
                            savestate!();
                            if call_newindex_tm_fast(lua_state, ci, ra, meta, rb, rc)? {
                                updatetrap!();
                                continue;
                            }
                            finishset_fallback_known_miss(lua_state, ci, &ra, &rb, rc)?;
                            updatetrap!();
                            continue;
                        }
                    }

                    // Slow path: shortstr, generic key, non-table, or metamethod
                    let ra = unsafe { *ra_ptr };
                    let rb = unsafe { *rb_ptr };
                    let rc = if instr.get_k() {
                        *k_val!(c)
                    } else {
                        *stack_val!(c)
                    };
                    if ra.is_table() {
                        let table = ra.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            if rb.is_short_string() {
                                if table.impl_table.set_existing_shortstr(&rb, rc) {
                                    if rc.is_collectable() || rb.is_collectable() {
                                        lua_state.gc_barrier_back(unsafe {
                                            ra.as_gc_ptr_table_unchecked()
                                        });
                                    }
                                    continue;
                                }
                                let pset_result = table.impl_table.pset_shortstr(&rb, rc);
                                let (new_key, delta) =
                                    table.impl_table.finish_shortstr_set(&rb, rc, pset_result);
                                if new_key {
                                    table.invalidate_tm_cache();
                                }
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(
                                        unsafe { ra.as_table_ptr_unchecked() },
                                        delta,
                                    );
                                }
                                if rc.is_collectable() || rb.is_collectable() {
                                    lua_state
                                        .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                                }
                                continue;
                            } else if !rb.is_nil() && !rb.ttisinteger() {
                                let (_new_key, delta) = table.impl_table.raw_set(&rb, rc);
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(
                                        unsafe { ra.as_table_ptr_unchecked() },
                                        delta,
                                    );
                                }
                                if rc.is_collectable() || rb.is_collectable() {
                                    lua_state
                                        .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                                }
                                continue;
                            }
                        } else if rb.is_short_string() {
                            if table.impl_table.set_existing_shortstr(&rb, rc) {
                                if rc.is_collectable() || rb.is_collectable() {
                                    lua_state
                                        .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                                }
                                continue;
                            }
                            savestate!();
                            if call_newindex_tm_fast(lua_state, ci, ra, meta, rb, rc)? {
                                updatetrap!();
                                continue;
                            }
                            finishset_fallback_known_miss(lua_state, ci, &ra, &rb, rc)?;
                            updatetrap!();
                            continue;
                        }
                    }
                    savestate!();
                    finishset_fallback(lua_state, ci, &ra, &rb, rc)?;
                    updatetrap!();
                }
                OpCode::SetI => {
                    // SETI: R[A][B] := RK(C) (integer key)
                    let ra = stack_val!(instr.get_a());
                    let b = instr.get_b() as i64;
                    let c = instr.get_c();

                    // Hot path: table with no __newindex metamethod, key in array range
                    if ra.is_table() {
                        let table = ra.hvalue_mut();
                        // Pre-extract table/gc pointers as Copy values to break borrow chain
                        // (ra is a reference into the stack which borrows lua_state)
                        let table_ptr = unsafe { ra.as_table_ptr_unchecked() };
                        let gc_ptr = unsafe { ra.as_gc_ptr_table_unchecked() };
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            if !instr.get_k() {
                                let rc_ptr =
                                    unsafe { lua_state.stack().as_ptr().add(stack_id!(c)) };
                                let rc_tt = unsafe { (*rc_ptr).tt };
                                let rc_value = unsafe { (*rc_ptr).value };
                                if table.impl_table.fast_seti_parts(b, rc_value, rc_tt) {
                                    if rc_tt & 0x40 != 0 {
                                        lua_state.gc_barrier_back(gc_ptr);
                                    }
                                    continue;
                                }

                                let rc = unsafe { *rc_ptr };
                                let delta = table.impl_table.set_int_slow(b, rc);
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(table_ptr, delta);
                                }
                                if rc_tt & 0x40 != 0 {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }

                            let rc = *k_val!(c);
                            if table.impl_table.fast_seti(b, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }

                            let delta = table.impl_table.set_int_slow(b, rc);
                            if delta != 0 {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if rc.is_collectable() {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        } else {
                            let rc = if instr.get_k() {
                                *k_val!(c)
                            } else {
                                *stack_val!(c)
                            };
                            if table.impl_table.set_existing_int(b, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            // Fall through to finishset_fallback_known_miss
                            let ra = *ra;
                            let rb = LuaValue::integer(b);
                            savestate!();
                            if call_newindex_tm_fast(lua_state, ci, ra, meta, rb, rc)? {
                                updatetrap!();
                                continue;
                            }
                            finishset_fallback_known_miss(lua_state, ci, &ra, &rb, rc)?;
                            updatetrap!();
                            continue;
                        }
                    }
                    let rc = if instr.get_k() {
                        *k_val!(c)
                    } else {
                        *stack_val!(c)
                    };
                    let ra = *ra;
                    let rb = LuaValue::integer(b);
                    savestate!();
                    finishset_fallback(lua_state, ci, &ra, &rb, rc)?;
                    updatetrap!();
                }
                OpCode::SetField => {
                    // SETFIELD: R[A][K[B]:string] := RK(C)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();
                    let ra_ptr = unsafe { lua_state.stack().as_ptr().add(stack_id!(a)) };
                    let key = k_val!(b);
                    debug_assert!(
                        key.is_short_string(),
                        "SetField key must be short string for fast path"
                    );
                    let mut known_newindex_miss = false;
                    let mut meta = TablePtr::null();
                    if unsafe { (*ra_ptr).is_table() } {
                        let table = unsafe { (*ra_ptr).hvalue_mut() };
                        meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let (new_key, delta, rc_tt) = if instr.get_k() {
                                let rc = *k_val!(c);
                                if table.impl_table.set_existing_shortstr(key, rc) {
                                    if rc.is_collectable() {
                                        lua_state.gc_barrier_back(unsafe {
                                            (*ra_ptr).as_gc_ptr_table_unchecked()
                                        });
                                    }
                                    continue;
                                }
                                let pset_result = table.impl_table.pset_shortstr(key, rc);
                                let (new_key, delta) =
                                    table.impl_table.finish_shortstr_set(key, rc, pset_result);
                                (new_key, delta, rc.tt)
                            } else {
                                let rc_ptr =
                                    unsafe { lua_state.stack().as_ptr().add(stack_id!(c)) };
                                let rc_tt = unsafe { (*rc_ptr).tt };
                                let rc_value = unsafe { (*rc_ptr).value };
                                if table
                                    .impl_table
                                    .set_existing_shortstr_parts(key, rc_value, rc_tt)
                                {
                                    if rc_tt & 0x40 != 0 {
                                        lua_state.gc_barrier_back(unsafe {
                                            (*ra_ptr).as_gc_ptr_table_unchecked()
                                        });
                                    }
                                    continue;
                                }
                                let pset_result =
                                    table.impl_table.pset_shortstr_parts(key, rc_value, rc_tt);
                                let (new_key, delta) = table.impl_table.finish_shortstr_set_parts(
                                    key,
                                    rc_value,
                                    rc_tt,
                                    pset_result,
                                );
                                (new_key, delta, rc_tt)
                            };
                            if new_key {
                                table.invalidate_tm_cache();
                            }
                            if delta != 0 {
                                lua_state.gc_track_table_resize(
                                    unsafe { (*ra_ptr).as_table_ptr_unchecked() },
                                    delta,
                                );
                            }
                            if rc_tt & 0x40 != 0 {
                                lua_state.gc_barrier_back(unsafe {
                                    (*ra_ptr).as_gc_ptr_table_unchecked()
                                });
                            }
                            continue;
                        } else {
                            let rc = if instr.get_k() {
                                *k_val!(c)
                            } else {
                                *stack_val!(c)
                            };
                            if table.impl_table.set_existing_shortstr(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(unsafe {
                                        (*ra_ptr).as_gc_ptr_table_unchecked()
                                    });
                                }
                                continue;
                            }
                            known_newindex_miss = true;
                        }
                    }
                    let ra = unsafe { *ra_ptr };
                    let rc = if instr.get_k() {
                        *k_val!(c)
                    } else {
                        *stack_val!(c)
                    };
                    let rb = *key;
                    savestate!();
                    if known_newindex_miss {
                        if call_newindex_tm_fast(lua_state, ci, ra, meta, rb, rc)? {
                            updatetrap!();
                            continue;
                        }
                        finishset_fallback_known_miss(lua_state, ci, &ra, &rb, rc)?;
                    } else {
                        finishset_fallback(lua_state, ci, &ra, &rb, rc)?;
                    }
                    updatetrap!();
                }
                OpCode::NewTable => {
                    // R[A] := {} (new table) — table ops should be inlined
                    let a = instr.get_a();
                    let mut vb = instr.get_vb();
                    let mut vc = instr.get_vc();
                    let k = instr.get_k();

                    vb = if vb > 0 {
                        if vb > 31 { 0 } else { 1 << (vb - 1) }
                    } else {
                        0
                    };

                    if k {
                        let extra_instr = unsafe { *code.get_unchecked(pc) };
                        if extra_instr.get_opcode() == OpCode::ExtraArg {
                            vc += extra_instr.get_ax() * 1024;
                        }
                    }

                    pc += 1; // skip EXTRAARG

                    let value = lua_state.create_table(vc as usize, vb as usize)?;
                    setobj2s(lua_state, stack_id!(a), &value);

                    let new_top = base + a as usize + 1;
                    // ci.save_pc(pc);
                    // lua_state.set_top_raw(new_top);
                    // lua_state.check_gc()?;
                    // let frame_top = ci.top;
                    // lua_state.set_top_raw(frame_top as usize);
                    lua_state.check_gc_in_loop(ci, pc, new_top, &mut trap);
                }
                OpCode::Self_ => {
                    // SELF: R[A+1] := R[B]; R[A] := R[B][K[C]:string]
                    let a = instr.get_a();
                    let rb = *stack_val!(instr.get_b());
                    let key = k_val!(instr.get_c());

                    debug_assert!(
                        key.is_short_string(),
                        "Self key must be short string for fast path"
                    );
                    setobj2s(lua_state, stack_id!(a + 1), &rb);
                    // Fast path: rb is a table with hash part
                    if rb.ttistable() {
                        let table = rb.hvalue();
                        if table.impl_table.has_hash() {
                            let dest =
                                unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                            if unsafe { table.impl_table.get_shortstr_into(key, dest) } {
                                continue;
                            }
                        }
                        if self_shortstr_index_chain_fast(lua_state, &rb, key, stack_id!(a)) {
                            continue;
                        }
                    }

                    savestate!();
                    finishget_fallback(lua_state, ci, &rb, key, stack_id!(a))?;
                    updatetrap!();
                }
                OpCode::Add => {
                    // op_arith(L, l_addi, luai_numadd)
                    // R[A] := R[B] + R[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_add(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) + pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 + n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::AddI => {
                    // op_arithI(L, l_addi, luai_numadd)
                    // R[A] := R[B] + sC
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let sc = instr.get_sc();

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        // Fast path: integer (most common)
                        if pttisinteger(v1_ptr) {
                            let iv1 = pivalue(v1_ptr);
                            psetivalue(ra_ptr, iv1.wrapping_add(sc as i64));
                            pc += 1; // Skip metamethod on success
                        }
                        // Slow path: float
                        else if pttisfloat(v1_ptr) {
                            let nb = pfltvalue(v1_ptr);
                            psetfltvalue(ra_ptr, nb + (sc as f64));
                            pc += 1; // Skip metamethod on success
                        }
                        // else: fall through to MMBINI (next instruction)
                    }
                }
                OpCode::Sub => {
                    // op_arith(L, l_subi, luai_numsub)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_sub(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) - pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 - n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Mul => {
                    // op_arith(L, l_muli, luai_nummul)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_mul(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) * pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 * n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Div => {
                    // op_arithf(L, luai_numdiv) - 浮点除法
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) / pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 / n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::IDiv => {
                    // op_arith(L, luaV_idiv, luai_numidiv) - 整数除法
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_idiv(i1, i2));
                                pc += 1;
                            } else {
                                ci.save_pc(pc);
                                return Err(error_div_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, (pfltvalue(v1_ptr) / pfltvalue(v2_ptr)).floor());
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, (n1 / n2).floor());
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Mod => {
                    // op_arith(L, luaV_mod, luaV_modf)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_imod(i1, i2));
                                pc += 1;
                            } else {
                                ci.save_pc(pc);
                                return Err(error_mod_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, lua_fmod(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, lua_fmod(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Pow => {
                    // op_arithf(L, luai_numpow)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, luai_numpow(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, luai_numpow(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::AddK => {
                    // op_arithK(L, l_addi, luai_numadd)
                    // R[A] := R[B] + K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_add(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) + pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 + n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::SubK => {
                    // R[A] := R[B] - K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_sub(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) - pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 - n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::MulK => {
                    // R[A] := R[B] * K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_mul(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) * pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 * n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::ModK => {
                    // R[A] := R[B] % K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_imod(i1, i2));
                                pc += 1;
                            } else {
                                ci.save_pc(pc);
                                return Err(error_mod_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, lua_fmod(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, lua_fmod(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::PowK => {
                    // R[A] := R[B] ^ K[C] (always float)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, luai_numpow(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, luai_numpow(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::DivK => {
                    // R[A] := R[B] / K[C] (float division)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) / pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 / n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::IDivK => {
                    // R[A] := R[B] // K[C] (floor division)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_idiv(i1, i2));
                                pc += 1;
                            } else {
                                ci.save_pc(pc);
                                return Err(error_div_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, (pfltvalue(v1_ptr) / pfltvalue(v2_ptr)).floor());
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, (n1 / n2).floor());
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::BAndK => {
                    // R[A] := R[B] & K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = k_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 & i2);
                    }
                }
                OpCode::BOrK => {
                    // R[A] := R[B] | K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = k_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 | i2);
                    }
                }
                OpCode::BXorK => {
                    // R[A] := R[B] ^ K[C] (bitwise xor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = k_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 ^ i2);
                    }
                }
                OpCode::BAnd => {
                    // op_bitwise(L, l_band)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 & i2);
                    }
                }
                OpCode::BOr => {
                    // op_bitwise(L, l_bor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 | i2);
                    }
                }
                OpCode::BXor => {
                    // op_bitwise(L, l_bxor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 ^ i2);
                    }
                }
                OpCode::Shl => {
                    // op_bitwise(L, luaV_shiftl)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), lua_shiftl(i1, i2));
                    }
                }
                OpCode::Shr => {
                    // op_bitwise(L, luaV_shiftr)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), lua_shiftr(i1, i2));
                    }
                }
                OpCode::ShlI => {
                    // R[A] := sC << R[B]
                    // Note: In Lua 5.5, SHLI is immediate << register (not register << immediate)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc(); // shift amount from immediate

                    let rb = stack_val!(b);

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftl(ic, ib): shift ic left by ib
                        setivalue(stack_val_mut!(a), lua_shiftl(ic as i64, ib));
                    }
                    // else: metamethod
                }
                OpCode::ShrI => {
                    // R[A] := R[B] >> sC
                    // Logical right shift (Lua 5.5: luaV_shiftr)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc(); // shift amount

                    let rb = stack_val!(b);

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftr(ib, ic) = luaV_shiftl(ib, -ic)
                        setivalue(stack_val_mut!(a), lua_shiftr(ib, ic as i64));
                    }
                    // else: metamethod
                }
                OpCode::MmBin => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let ra = *stack_val!(a);
                    let rb = *stack_val!(b);
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let result_reg = (base + pi.get_a() as usize) as u32;

                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };

                    savestate!();
                    bin_tm_fallback(lua_state, ci, ra, rb, result_reg, a as u32, b as u32, tm)?;
                    updatetrap!();
                }
                OpCode::MmBinI => {
                    let a = instr.get_a() as usize;
                    let imm = instr.get_sb();
                    let flip = instr.get_k();

                    let ra = stack_val!(a);
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let result_reg = (base + pi.get_a() as usize) as u32;

                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };
                    let rb = LuaValue::integer(imm as i64);
                    let r = if flip { (rb, *ra) } else { (*ra, rb) };
                    savestate!();
                    bin_tm_fallback(lua_state, ci, r.0, r.1, result_reg, a as u32, a as u32, tm)?;
                    updatetrap!();
                }
                OpCode::MmBinK => {
                    let ra = *stack_val!(instr.get_a());
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let imm = *k_val!(instr.get_b());
                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };
                    let flip = instr.get_k();
                    let result_reg = (base + pi.get_a() as usize) as u32;

                    let a_reg = instr.get_a();
                    savestate!();
                    let r = if flip { (imm, ra) } else { (ra, imm) };
                    bin_tm_fallback(lua_state, ci, r.0, r.1, result_reg, a_reg, a_reg, tm)?;
                    updatetrap!();
                }
                OpCode::Unm => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = *stack_val!(b);

                    if ttisinteger(&rb) {
                        let ib = ivalue(&rb);
                        setivalue(stack_val_mut!(a), ib.wrapping_neg());
                    } else {
                        let mut nb = 0.0;
                        if tonumberns(&rb, &mut nb) {
                            setfltvalue(stack_val_mut!(a), -nb);
                        } else {
                            savestate!();
                            let result_reg = stack_id!(a);
                            unary_tm_fallback(lua_state, ci, rb, result_reg, TmKind::Unm)?;
                            updatetrap!();
                        }
                    }
                }
                OpCode::BNot => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = *stack_val!(b);

                    let mut ib = 0i64;
                    if tointegerns(&rb, &mut ib) {
                        setivalue(stack_val_mut!(a), !ib);
                    } else {
                        savestate!();
                        let result_reg = stack_id!(a);
                        unary_tm_fallback(lua_state, ci, rb, result_reg, TmKind::Bnot)?;
                        updatetrap!();
                    }
                }
                OpCode::Not => {
                    // R[A] := not R[B]
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = stack_val!(b);
                    if rb.ttisfalse() || rb.is_nil() {
                        setbtvalue(stack_val_mut!(a));
                    } else {
                        setbfvalue(stack_val_mut!(a));
                    }
                }
                OpCode::Len => {
                    // HOT PATH: inline table length for no-metatable case
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let rb = *stack_val!(b);
                    savestate!();
                    objlen(lua_state, stack_id!(a), rb)?;
                }
                OpCode::Concat => {
                    let a = instr.get_a();
                    let n = instr.get_b();

                    if n == 2 {
                        let left = *stack_val!(a);
                        let right = *stack_val!(a + 1);
                        ci.save_pc(pc);

                        if let Some(result) = try_concat_pair_utf8(lua_state, left, right)? {
                            *stack_val_mut!(a) = result;
                            updatetrap!();

                            let top = lua_state.get_top();
                            lua_state.check_gc_in_loop(ci, pc, top, &mut trap);
                            continue;
                        }
                    }

                    let concat_top = base + (a + n) as usize;
                    lua_state.set_top_raw(concat_top);

                    // ProtectNT
                    ci.save_pc(pc);
                    match concat(lua_state, n as usize) {
                        Ok(()) => {}
                        Err(LuaError::Yield) => {
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                    updatetrap!();

                    let top = lua_state.get_top();
                    lua_state.check_gc_in_loop(ci, pc, top, &mut trap);
                }
                OpCode::Close => {
                    let a = instr.get_a();
                    let close_from = stack_id!(a);

                    ci.save_pc(pc);
                    match lua_state.close_all(close_from) {
                        Ok(()) => {}
                        Err(LuaError::Yield) => {
                            ci.pc -= 1;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::Tbc => {
                    // Mark variable as to-be-closed
                    let a = instr.get_a();
                    ci.save_pc(pc); // save PC so get_local_var_name finds the variable name
                    lua_state.mark_tbc(stack_id!(a))?;
                }
                OpCode::Jmp => {
                    let sj = instr.get_sj();
                    let target_pc = (pc as isize + sj as isize) as usize;
                    #[cfg(feature = "jit")]
                    if sj < 0 {
                        if let Some(exit_pc) = unsafe {
                            jit_try_handle_jmp_backedge(lua_state, ci, base, constants, target_pc)
                        } {
                            pc = exit_pc;
                            updatetrap!();
                            continue;
                        }
                    }
                    pc = target_pc;
                    updatetrap!();
                }
                OpCode::Eq => {
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let ra = *stack_val!(a);
                    let rb = *stack_val!(b);
                    savestate!();
                    let cond = eq_fallback(lua_state, ci, ra, rb)?;
                    updatetrap!();
                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::Lt => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = unsafe { stack.get_unchecked(stack_id!(a)) };
                        let rb = unsafe { stack.get_unchecked(stack_id!(b)) };

                        if ttisinteger(ra) && ttisinteger(rb) {
                            ivalue(ra) < ivalue(rb)
                        } else if ra.is_number() && rb.is_number() {
                            lt_num(ra, rb)
                        } else if ttisstring(ra) && ttisstring(rb) {
                            let sa = ra.as_bytes();
                            let sb = rb.as_bytes();

                            if let (Some(sa), Some(sb)) = (sa, sb) {
                                sa < sb
                            } else {
                                false
                            }
                        } else {
                            let va = *ra;
                            let vb = *rb;
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Lt)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::Le => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = unsafe { stack.get_unchecked(stack_id!(a)) };
                        let rb = unsafe { stack.get_unchecked(stack_id!(b)) };

                        if ttisinteger(ra) && ttisinteger(rb) {
                            ivalue(ra) <= ivalue(rb)
                        } else if ra.is_number() && rb.is_number() {
                            le_num(ra, rb)
                        } else if ttisstring(ra) && ttisstring(rb) {
                            let sa = ra.as_bytes();
                            let sb = rb.as_bytes();

                            if let (Some(sa), Some(sb)) = (sa, sb) {
                                sa <= sb
                            } else {
                                false
                            }
                        } else {
                            let va = *ra;
                            let vb = *rb;
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Le)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::EqK => {
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let k = instr.get_k();

                    let ra = stack_val!(a);
                    let rb = k_val!(b);
                    // Raw equality (no metamethods for constants)
                    let cond = ra == rb;
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::EqI => {
                    let a = instr.get_a();
                    let im = instr.get_sb();
                    let ra = stack_val!(a);
                    let cond = if ttisinteger(ra) {
                        ivalue(ra) == im as i64
                    } else if ttisfloat(ra) {
                        fltvalue(ra) == im as f64
                    } else {
                        false
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::LtI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = unsafe {
                        let ra_ptr = lua_state.stack_mut().as_mut_ptr().add(base + a);

                        if pttisinteger(ra_ptr) {
                            pivalue(ra_ptr) < im as i64
                        } else if pttisfloat(ra_ptr) {
                            pfltvalue(ra_ptr) < im as f64
                        } else {
                            let va = *ra_ptr;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Lt)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::LeI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = unsafe {
                        let ra_ptr = lua_state.stack_mut().as_mut_ptr().add(base + a);

                        if pttisinteger(ra_ptr) {
                            pivalue(ra_ptr) <= im as i64
                        } else if pttisfloat(ra_ptr) {
                            pfltvalue(ra_ptr) <= im as f64
                        } else {
                            let va = *ra_ptr;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Le)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::GtI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = unsafe {
                        let ra_ptr = lua_state.stack_mut().as_mut_ptr().add(base + a);

                        if pttisinteger(ra_ptr) {
                            pivalue(ra_ptr) > im as i64
                        } else if pttisfloat(ra_ptr) {
                            pfltvalue(ra_ptr) > im as f64
                        } else {
                            let va = *ra_ptr;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            // GtI: a > b ≡ b < a → swap args, use Lt
                            let result = order_tm_fallback(lua_state, ci, vb, va, TmKind::Lt)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::GeI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = unsafe {
                        let ra_ptr = lua_state.stack_mut().as_mut_ptr().add(base + a);

                        if pttisinteger(ra_ptr) {
                            pivalue(ra_ptr) >= im as i64
                        } else if pttisfloat(ra_ptr) {
                            pfltvalue(ra_ptr) >= im as f64
                        } else {
                            let va = *ra_ptr;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            // GeI: a >= b ≡ b <= a → swap args, use Le
                            let result = order_tm_fallback(lua_state, ci, vb, va, TmKind::Le)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::Test => {
                    let a = instr.get_a();
                    let ra = stack_val!(a);
                    // l_isfalse: nil or false => truthy = !nil && !false
                    let cond = !ra.is_nil() && !ra.ttisfalse();

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::TestSet => {
                    // if (l_isfalse(R[B]) == k) then pc++ else R[A] := R[B]; donextjump
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let k = instr.get_k();
 
                    let rb = *stack_val!(b);
                    let cond = rb.is_nil() || rb.ttisfalse();
                    if cond == k {
                        pc += 1; // Condition failed - skip next instruction (JMP)
                    } else {
                        // Condition succeeded - copy value and EXECUTE next instruction (must be JMP)
                        setobj2s(lua_state, stack_id!(a), &rb);
                        // donextjump: fetch and execute next JMP instruction
                        let next_instr = unsafe { *code.get_unchecked(pc) };
                        debug_assert!(next_instr.get_opcode() == OpCode::Jmp);
                        pc += 1; // Move past the JMP instruction
                        let sj = next_instr.get_sj();
                        pc = (pc as isize + sj as isize) as usize; // Execute the jump
                        updatetrap!();
                    }
                }
                OpCode::Call => {
                    let a = instr.get_a();
                    let b = instr.get_b() as usize;
                    let nresults = instr.get_c() as i32 - 1;
                    let func_idx = stack_id!(a);
                    let nargs = if b != 0 {
                        lua_state.set_top_raw(func_idx + b);
                        b - 1
                    } else {
                        lua_state.get_top() - func_idx - 1
                    };
                    ci.save_pc(pc);
                    if precall(lua_state, func_idx, nargs, nresults)? {
                        // Lua call: new frame pushed
                        continue 'startfunc;
                    }
                    // C call completed
                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::TailCall => {
                    let a = instr.get_a();
                    let mut b = instr.get_b() as usize;
                    let func_idx = stack_id!(a);
                    // let nparams1 = instr.get_c() as usize;
                    if b != 0 {
                        lua_state.set_top_raw(func_idx + b);
                    } else {
                        b = lua_state.get_top() - func_idx;
                    }
                    ci.save_pc(pc);
                    if instr.get_k() {
                        lua_state.close_upvalues(base);
                    }
                    if pretailcall(lua_state, ci, func_idx, b)? {
                        // Lua tail call: CI reused in place
                        continue 'startfunc;
                    }
                    // C tail call completed
                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::Return => {
                    // return R[A], ..., R[A+B-2]   (lvm.c:1763-1783)
                    let a_pos = stack_id!(instr.get_a());
                    let mut n;

                    // Check if resuming after a yield inside __close during return
                    if ci.call_status & CIST_CLSRET != 0 {
                        // Resuming from yield-in-close: use saved nres and skip close_all
                        // (close_all already ran; remaining TBCs were closed on resume)
                        ci.call_status &= !CIST_CLSRET;
                        n = ci.saved_nres();

                        // Save pc first so re-yield points to RETURN again
                        ci.save_pc(pc);

                        // Continue closing remaining TBC variables (if any)
                        match lua_state.close_all(base) {
                            Ok(()) => {}
                            Err(LuaError::Yield) => {
                                ci.call_status |= CIST_CLSRET;
                                ci.pc -= 1;
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        n = instr.get_b() as i32 - 1;
                        if n < 0 {
                            n = (lua_state.get_top() - a_pos) as i32;
                        }

                        ci.save_pc(pc);
                        if instr.get_k() {
                            // May have open upvalues / TBC variables
                            ci.set_saved_nres(n);
                            if lua_state.get_top() < ci.top as usize {
                                lua_state.set_top_raw(ci.top as usize);
                            }
                            match lua_state.close_all(base) {
                                Ok(()) => {}
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_CLSRET;
                                    ci.pc -= 1;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    }

                    lua_state.set_top_raw(a_pos + n as usize);
                    poscall(lua_state, ci, n as usize, pc)?;
                    resume_caller_fast!();
                }
                OpCode::Return0 => {
                    // return (no values)
                    if lua_state.hook_mask & (LUA_MASKRET | LUA_MASKLINE) != 0 {
                        return0_with_hook(lua_state, ci, stack_id!(instr.get_a()), pc)?;
                        continue 'startfunc;
                    }

                    // Inlined fast path: no hook, no moveresults overhead
                    let nresults = ci.nresults();
                    let res = ci.base - ci.func_offset as usize;
                    lua_state.pop_call_frame();
                    lua_state.set_top_raw(res);
                    // nil-fill if caller wanted results
                    if nresults > 0 {
                        unsafe {
                            let sp = lua_state.stack_mut().as_mut_ptr();
                            for i in 0..nresults as usize {
                                *sp.add(res + i) = LuaValue::nil();
                            }
                        }
                        lua_state.set_top_raw(res + nresults as usize);
                    }

                    resume_caller_fast!();
                }
                OpCode::Return1 => {
                    // return R[A]  (single value)
                    if lua_state.hook_mask & (LUA_MASKRET | LUA_MASKLINE) != 0 {
                        return1_with_hook(lua_state, ci, stack_id!(instr.get_a()), pc)?;
                        continue 'startfunc;
                    }

                    // Inlined fast path — raw pointer for single copy
                    let nresults = ci.nresults();
                    let res = ci.base - ci.func_offset as usize;
                    lua_state.pop_call_frame();
                    if nresults == 0 {
                        // Caller wants no results
                        lua_state.set_top_raw(res);
                    } else {
                        // Copy the single result value using raw pointer
                        unsafe {
                            let sp = lua_state.stack_mut().as_mut_ptr();
                            let val = *sp.add(base + instr.get_a() as usize);
                            *sp.add(res) = val;
                        }
                        lua_state.set_top_raw(res + 1);
                        // nil-fill if caller wanted more than 1
                        if nresults > 1 {
                            unsafe {
                                let sp = lua_state.stack_mut().as_mut_ptr();
                                for i in 1..nresults as usize {
                                    *sp.add(res + i) = LuaValue::nil();
                                }
                            }
                            lua_state.set_top_raw(res + nresults as usize);
                        }
                    }

                    resume_caller_fast!();
                }
                OpCode::ForLoop => {
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    unsafe {
                        let ra = lua_state.stack_mut().as_mut_ptr().add(base + a);
                        // Check if integer loop (tag of step at ra+1)
                        if pttisinteger(ra.add(1)) {
                            // Integer loop (most common for numeric loops)
                            // ra: counter (count of iterations left)
                            // ra+1: step
                            // ra+2: control variable (idx)
                            let count = pivalue(ra) as u64;
                            if count > 0 {
                                // More iterations
                                let step = pivalue(ra.add(1));
                                let idx = pivalue(ra.add(2));

                                // Update counter (decrement) - only write value, tag unchanged
                                (*ra).value.i = count as i64 - 1;
                                // Update control variable: idx += step - only write value
                                (*ra.add(2)).value.i = idx.wrapping_add(step);

                                // Jump back
                                pc -= bx;
                                #[cfg(feature = "jit")]
                                {
                                    if let Some(exit_pc) = jit_try_handle_forloop_backedge(
                                        lua_state,
                                        ci,
                                        base,
                                        constants,
                                        pc,
                                        pc + bx,
                                    )
                                    {
                                        pc = exit_pc;
                                    }
                                }
                            }
                            // else: counter expired, exit loop
                        } else if float_for_loop(lua_state, base + a) {
                            // Float loop with non-integer step
                            // Jump back if loop continues
                            pc -= bx;
                            #[cfg(feature = "jit")]
                            {
                                jit::record_loop_backedge(lua_state, ci.chunk_ptr, pc);
                            }
                        }
                    }

                    updatetrap!();
                }
                OpCode::ForPrep => {
                    let a = instr.get_a();
                    savestate!();
                    if forprep(lua_state, stack_id!(a))? {
                        // Skip the loop body: jump forward past FORLOOP
                        pc += instr.get_bx() as usize + 1;
                    }
                }
                OpCode::TForPrep => {
                    // Prepare generic for loop — inline (for loop related)
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = base + a;

                    // Swap control and closing variables
                    stack.swap(ra + 3, ra + 2);

                    // Mark ra+2 as to-be-closed if not nil
                    lua_state.mark_tbc(ra + 2)?;

                    pc += bx;
                }
                OpCode::TForCall => {
                    // Generic for loop call — matches C Lua's OP_TFORCALL.
                    // Copy iterator+state+control to ra+3..ra+5, then precall.
                    let a = instr.get_a() as usize;
                    let c = instr.get_c() as usize;
                    let ra = base + a;
                    let func_idx = ra + 3;
                    unsafe {
                        let stack = lua_state.stack_mut();
                        *stack.get_unchecked_mut(ra + 5) = *stack.get_unchecked(ra + 3);
                        *stack.get_unchecked_mut(ra + 4) = *stack.get_unchecked(ra + 1);
                        *stack.get_unchecked_mut(ra + 3) = *stack.get_unchecked(ra);
                    }
                    lua_state.set_top_raw(func_idx + 3); // func + 2 args
                    ci.save_pc(pc);
                    if precall(lua_state, func_idx, 2, c as i32)? {
                        // Lua iterator: new frame pushed
                        continue 'startfunc;
                    }
                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::TForLoop => {
                    // Generic for loop test
                    // If ra+3 (control variable) != nil then continue loop (jump back)
                    // After TForPrep swap: ra+2=closing(TBC), ra+3=control
                    // TFORCALL places first result at ra+3, automatically updating control
                    // Check if ra+3 (control value from iterator) is not nil
                    if !stack_val!(instr.get_a() + 3).is_nil() {
                        // Continue loop: jump back
                        pc -= instr.get_bx() as usize;
                        #[cfg(feature = "jit")]
                        {
                            if let Some(exit_pc) = unsafe {
                                jit_try_handle_tforloop_backedge(
                                    lua_state,
                                    ci,
                                    base,
                                    pc,
                                    pc + instr.get_bx() as usize,
                                )
                            } {
                                pc = exit_pc;
                            }
                        }
                    }
                    // else: exit loop (control variable is nil)
                }
                OpCode::SetList => {
                    let a = instr.get_a();
                    let mut n = instr.get_vb() as usize;
                    let stack_idx = instr.get_vc() as usize;
                    let mut last = stack_idx;
                    if n == 0 {
                        n = lua_state.get_top() - stack_id!(a) - 1; // adjust n based on top if vb=0
                    } else {
                        lua_state.set_top_raw(ci.top as usize);
                    }
                    last += n;
                    if instr.get_k() {
                        let next_instr = unsafe { *code.get_unchecked(pc) };
                        debug_assert!(next_instr.get_opcode() == OpCode::ExtraArg);
                        pc += 1; // Consume EXTRAARG
                        let extra = next_instr.get_ax() as usize;
                        // Add extra to starting index
                        last += extra * (1 << Instruction::SIZE_V_C);
                    }
                    let ra = *stack_val!(a);
                    let h = ra.hvalue_mut();
                    if last > h.impl_table.asize as usize {
                        h.impl_table.resize_array(last as u32);
                    }

                    let impl_table = &mut h.impl_table;
                    let stack_ptr = lua_state.stack().as_ptr();
                    let mut is_collectable = false;
                    // Port of C Lua's SETLIST loop (lvm.c):
                    //   for (; n > 0; n--) { val = s2v(ra+n); obj2arr(h, last, val); last--; }
                    // Reads n values from stack[ra+n..ra+1], writes to table[last..last-n+1]
                    let mut write_idx = last;
                    for i in (1..=n).rev() {
                        let val = unsafe { *stack_ptr.add(stack_id!(a) + i) };
                        if val.iscollectable() {
                            is_collectable = true;
                        }
                        unsafe {
                            impl_table.write_array(write_idx as i64, val);
                        }
                        write_idx -= 1;
                    }

                    if is_collectable {
                        lua_state.gc_barrier_back(unsafe { ra.as_gc_ptr_unchecked() });
                    }
                }
                OpCode::Closure => {
                    let a = instr.get_a() as usize;
                    let proto_idx = instr.get_bx() as usize;
                    savestate!();
                    let upvalue_ptrs =
                        unsafe { std::slice::from_raw_parts(ci.upvalue_ptrs, chunk.upvalue_count) };
                    push_closure(lua_state, base, a, proto_idx, chunk, upvalue_ptrs)?;

                    lua_state.check_gc_in_loop(ci, pc, base + a + 1, &mut trap);
                }
                OpCode::Vararg => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let n = instr.get_c() as i32 - 1;
                    let vatab = if instr.get_k() { b as i32 } else { -1 };

                    savestate!();
                    match get_varargs(lua_state, ci, base, a, b, vatab, n, chunk) {
                        Ok(()) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::GetVarg => {
                    let a = stack_id!(instr.get_a());
                    let c = stack_id!(instr.get_c());
                    get_vararg(lua_state, ci, base, a, c)?;
                }
                OpCode::ErrNNil => {
                    let a = instr.get_a();
                    let ra = stack_val!(a);

                    if !ra.is_nil() {
                        let bx = instr.get_bx() as usize;
                        let global_name = if bx > 0 && bx - 1 < constants.len() {
                            if let Some(s) = constants[bx - 1].as_str() {
                                s.to_string()
                            } else {
                                "?".to_string()
                            }
                        } else {
                            "?".to_string()
                        };

                        savestate!();
                        return Err(error_global(lua_state, &global_name));
                    }
                }
                OpCode::VarargPrep => {
                    exec_varargprep(lua_state, ci, chunk, &mut base)?;
                    // After varargprep, hook call if hooks are active
                    let hook_mask = lua_state.hook_mask;
                    if hook_mask != 0 {
                        let call_status = ci.call_status;
                        hook_on_call(lua_state, hook_mask, call_status, chunk)?;
                        if hook_mask & LUA_MASKLINE != 0 {
                            lua_state.oldpc = u32::MAX; // force line event on next instruction
                        }
                    }
                }
                OpCode::ExtraArg => {
                    // Extra argument for previous opcode
                    // This instruction should never be executed directly
                    // It's always consumed by the previous instruction (NEWTABLE, SETLIST, etc.)
                    // If we reach here, it's a compiler error
                    debug_assert!(false, "ExtraArg should never be executed directly");
                }
            }
        }
    }
}
