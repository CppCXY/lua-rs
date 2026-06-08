use crate::lua_value::{LUA_VNIL, LUA_VNUMINT, LuaInnerValue};
use crate::stdlib::basic::parse_number::parse_lua_number;
use crate::stdlib::debug::{objtypename, ordererror, typeerror};
use crate::{
    Instruction, LuaProto, LuaResult, LuaValue, OpCode,
    gc::TablePtr,
    lua_value::{lua_value_to_udvalue, udvalue_to_lua_value, udvalue_to_lua_value_with_token},
    lua_vm::{
        LuaError, LuaState, StkId, TmKind,
        call_info::{
            CallInfo,
            call_status::{
                CIST_C, CIST_PENDING_FINISH, CIST_RECST, CIST_XPCALL, CIST_YCALL, CIST_YPCALL,
            },
        },
        execute::{
            call::poscall,
            call_tm_res,
            concat::concat,
            metamethod::{self, call_tm_res_into},
        },
        lua_limits::{EXTRA_STACK, MAXTAGLOOP},
    },
};

/// Build hidden arguments for vararg functions
///
/// Initial stack:  func arg1 ... argn extra1 ...
///                 ^ ci->func                    ^ L->top
/// Final stack: func nil ... nil extra1 ... func arg1 ... argn
///                                          ^ ci->func
pub fn buildhiddenargs(
    lua_state: &mut LuaState,
    chunk: &LuaProto,
    totalargs: usize,
    nfixparams: usize,
    _nextra: usize,
) -> LuaResult<usize> {
    let old_base = lua_state
        .current_frame()
        .expect("base lookup requires an active call frame")
        .base;
    let func_pos = if old_base > 0 { old_base - 1 } else { 0 };

    // The new function position is right after all the original arguments.
    // This way the extra (vararg) arguments are "hidden" between the old and new func positions.
    let new_func_pos = func_pos + totalargs + 1;
    let new_base = new_func_pos + 1;

    // Ensure enough stack space for new base + registers + EXTRA_STACK
    let new_needed_size = new_base + chunk.max_stack_size + EXTRA_STACK;
    if new_needed_size > lua_state.stack_len() {
        lua_state.grow_stack(new_needed_size)?;
    }

    let stack = lua_state.stack_mut();

    // Step 1: Copy function to new_func_pos
    // grow_stack above ensures stack is large enough for new_func_pos and new_base.
    stack[new_func_pos] = stack[func_pos];

    // Step 2: Copy fixed parameters to after new function position
    for i in 0..nfixparams {
        stack[new_base + i] = stack[func_pos + 1 + i];
        // Erase original parameter with nil (for GC)
        setnilvalue(&mut stack[func_pos + 1 + i]);
    }

    {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let ci = lua_state
            .current_frame_mut()
            .expect("stack frame update requires an active call frame");
        ci.base = new_base;
        ci.base_stk = StkId::from_stack(sp, new_base);
        ci.top = (new_base + chunk.max_stack_size) as u32;
        ci.func_offset = (new_base - func_pos) as u32; // Distance from new_base to original func
    }

    // Update lua_state.top to match call_info.top
    let new_call_info_top = new_base + chunk.max_stack_size;
    lua_state.set_top(new_call_info_top)?;

    Ok(new_base)
}

/// ttisinteger_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn ttisinteger(v: &LuaValue) -> bool {
    v.ttisinteger()
}

/// ttisfloat_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn ttisfloat(v: &LuaValue) -> bool {
    v.ttisfloat()
}

/// ttisstring_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn ttisstring(v: &LuaValue) -> bool {
    v.is_string()
}

// ============ 值访问宏 (对应 Lua 的 ivalue/fltvalue) ============

/// ivalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn ivalue(v: &LuaValue) -> i64 {
    v.ivalue()
}

/// setivalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn setivalue(v: &mut LuaValue, i: i64) {
    v.tt = LUA_VNUMINT;
    v.value.i = i;
}

/// setnilvalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn setnilvalue(v: &mut LuaValue) {
    // *v = LuaValue::nil();
    v.tt = LUA_VNIL;
    v.value = LuaInnerValue::NIL;
}

#[inline(always)]
pub fn setobjs2s(l: &mut LuaState, a: usize, b: usize) {
    let stack = l.stack_mut();
    unsafe {
        *stack.get_unchecked_mut(a) = *stack.get_unchecked(b);
    }
}

/// tointegerns_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn tointegerns(v: &LuaValue, out: &mut i64) -> bool {
    if v.ttisinteger() {
        *out = v.ivalue();
        true
    } else if v.ttisfloat() {
        // Try converting integral-valued floats (e.g. 5.0 -> 5)
        // Range check matches C Lua's lua_numbertointeger:
        //   f >= (i64::MIN as f64) && f < -(i64::MIN as f64)
        // Note: i64::MAX as f64 rounds UP to 2^63, so we must use strict <
        // with -(i64::MIN as f64) = 2^63 (exactly representable).
        let f = v.fltvalue();
        if f >= (i64::MIN as f64) && f < -(i64::MIN as f64) && f == (f as i64 as f64) {
            *out = f as i64;
            true
        } else {
            false
        }
    } else {
        false
    }
}

/// tonumberns - 引用版本
#[inline(always)]
pub fn tonumberns(v: &LuaValue, out: &mut f64) -> bool {
    if v.ttisfloat() {
        *out = v.fltvalue();
        true
    } else if v.ttisinteger() {
        *out = v.ivalue() as f64;
        true
    } else {
        false
    }
}

/// Convert a LuaValue to integer using a rounding mode.
/// Port of C Lua 5.5's luaV_tointeger (lvm.c:157).
/// mode: 0 = exact only, 1 = floor, 2 = ceil
/// Handles integers, floats, and strings.
fn tointeger_mode(v: &LuaValue, mode: i32) -> Option<i64> {
    if v.ttisinteger() {
        return Some(v.ivalue());
    }
    // Get as float (including string conversion)
    let f = if v.ttisfloat() {
        v.fltvalue()
    } else if v.is_string() {
        let result = parse_lua_number(v.as_str().unwrap_or(""));
        if result.is_float() {
            result.fltvalue()
        } else if result.is_integer() {
            return Some(result.ivalue());
        } else {
            return None;
        }
    } else {
        return None;
    };
    // Convert float to integer using mode
    let rounded = match mode {
        1 => f.floor(),
        2 => f.ceil(),
        _ => f, // mode 0: exact
    };
    if rounded.is_nan() {
        return None;
    }
    if mode == 0 && rounded != f {
        return None;
    }
    // Check range: must fit in i64
    if rounded >= (i64::MIN as f64) && rounded < -(i64::MIN as f64) {
        Some(rounded as i64)
    } else {
        None
    }
}

/// Lookup value from object's metatable __index
/// Returns Ok(Some(value)) if found, Ok(None) if not found in table chain,
/// or Err if attempting to index a non-table value without __index metamethod.
///
/// Optimized hot path: inline fasttm check for __index to avoid function call overhead.
/// Matches Lua 5.5's luaV_finishget pattern.
fn finishget_inner(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    skip_first_raw_lookup: bool,
) -> LuaResult<Option<LuaValue>> {
    let mut t = *obj;
    let mut skip_raw_lookup = skip_first_raw_lookup;

    for _ in 0..MAXTAGLOOP {
        // Inline fasttm for __index on tables (hot path optimization)
        let tm = if let Some(table) = t.as_table_mut() {
            // Try raw_get first — handles key types the caller's fast paths didn't cover
            // (float→int normalization, long strings, etc.)
            if !skip_raw_lookup {
                if let Some(val) = table.raw_get(key) {
                    return Ok(Some(val));
                }
            } else {
                skip_raw_lookup = false;
            }

            match get_metamethod_from_meta_ptr(lua_state, table.meta_ptr(), TmKind::Index) {
                Some(v) => v,
                None => return Ok(None),
            }
        } else {
            // Non-table (string, userdata): check trait-based field access first
            if t.ttisfulluserdata()
                && let Some(ud) = t.as_userdata_mut()
            {
                let token = ud.sub_guard_token();
                let trait_obj = ud.get_trait()?;
                // Try trait-based get_field (key must be a string)
                if let Some(key_str) = key.as_str()
                    && let Some(udv) = trait_obj.get_field(key_str)
                {
                    let result = udvalue_to_lua_value_with_token(lua_state, udv, token)?;
                    return Ok(Some(result));
                }
            }
            // Fall back to general metamethod path
            match get_metamethod_event(lua_state, &t, TmKind::Index) {
                Some(tm) => tm,
                None => {
                    // No __index metamethod on non-table value → error
                    // Use typeerror for enhanced error message with varinfo
                    return Err(typeerror(lua_state, &t, "index"));
                }
            }
        };

        // If __index is a function, call it using call_tm_res
        if tm.is_function() {
            let result = call_tm_res(lua_state, tm, t, *key)?;
            return Ok(Some(result));
        }

        // __index is a table, try to access tm[key] directly
        t = tm;

        if let Some(table) = t.as_table() {
            // Use fast_geti for integer keys to avoid raw_get's float normalization
            let value = if key.ttisinteger() {
                table.impl_table.fast_geti(key.ivalue())
            } else if key.is_short_string() {
                table.impl_table.get_shortstr_fast(key)
            } else {
                table.raw_get(key)
            };
            if let Some(value) = value {
                return Ok(Some(value));
            }
            skip_raw_lookup = true;
        }

        // If not found, loop again to check if tm has __index
    }

    // Too many iterations - possible loop
    Err(lua_state.error("'__index' chain too long; possible loop".to_string()))
}

pub fn finishget(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
) -> LuaResult<Option<LuaValue>> {
    finishget_inner(lua_state, obj, key, false)
}

/// Get a metamethod from a metatable value — implements Lua 5.5's fasttm/luaT_gettm pattern.
/// Uses bit-flag cache (u32, covering all 26 TmKind values) to skip hash lookups
/// when the metamethod is known absent.
#[inline]
fn get_metamethod_from_metatable(
    lua_state: &mut LuaState,
    metatable: LuaValue,
    tm_kind: TmKind,
) -> Option<LuaValue> {
    metatable
        .as_table_ptr()
        .and_then(|meta_ptr| get_metamethod_from_meta_ptr(lua_state, meta_ptr, tm_kind))
}

#[inline]
pub(crate) fn get_metamethod_from_meta_ptr(
    lua_state: &mut LuaState,
    meta_ptr: TablePtr,
    tm_kind: TmKind,
) -> Option<LuaValue> {
    if meta_ptr.is_null() {
        return None;
    }

    let mt = unsafe { &mut (*meta_ptr.as_mut_ptr()).data };
    let tm_idx = tm_kind as u8;
    if mt.no_tm(tm_idx) {
        return None;
    }

    let vm = lua_state.global_state_mut();
    let event_key = vm.const_strings.get_tm_value(tm_kind);
    let result = mt.impl_table.get_shortstr_fast(&event_key);

    if result.is_none() {
        mt.set_tm_absent(tm_idx);
    }

    result
}

/// Port of Lua 5.5's luaV_finishset from lvm.c:334
/// ```c
/// void luaV_finishset (lua_State *L, const TValue *t, TValue *key,
///                       TValue *val, int hres) {
///   int loop;  /* counter to avoid infinite loops */
///   for (loop = 0; loop < MAXTAGLOOP; loop++) {
///     const TValue *tm;  /* '__newindex' metamethod */
///     if (hres != HNOTATABLE) {  /* is 't' a table? */
///       Table *h = hvalue(t);  /* save 't' table */
///       tm = fasttm(L, h->metatable, TM_NEWINDEX);  /* get metamethod */
///       if (tm == NULL) {  /* no metamethod? */
///         sethvalue2s(L, L->top.p, h);  /* anchor 't' */
///         L->top.p++;  /* assume EXTRA_STACK */
///         luaH_finishset(L, h, key, val, hres);  /* set new value */
///         L->top.p--;
///         invalidateTMcache(h);
///         luaC_barrierback(L, obj2gco(h), val);
///         return;
///       }
///       /* else will try the metamethod */
///     }
///     else {  /* not a table; check metamethod */
///       tm = luaT_gettmbyobj(L, t, TM_NEWINDEX);
///       if (l_unlikely(notm(tm)))
///         luaG_typeerror(L, t, "index");
///     }
///     /* try the metamethod */
///     if (ttisfunction(tm)) {
///       luaT_callTM(L, tm, t, key, val);
///       return;
///     }
///     t = tm;  /* else repeat assignment over 'tm' */
///     luaV_fastset(t, key, val, hres, luaH_pset);
///     if (hres == HOK) {
///       luaV_finishfastset(L, t, val);
///       return;  /* done */
///     }
///     /* else 'return luaV_finishset(L, t, key, val, slot)' (loop) */
///   }
///   luaG_runerror(L, "'__newindex' chain too long; possible loop");
/// }
/// ```
pub(crate) fn finishset(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    value: LuaValue,
    skip_existing_check: bool,
) -> LuaResult<bool> {
    // Check for invalid keys (nil or NaN)
    if key.is_nil() {
        return Err(lua_state.error("table index is nil".to_string()));
    }
    if key.ttisfloat()
        && let Some(f) = key.as_float()
        && f.is_nan()
    {
        return Err(lua_state.error("table index is NaN".to_string()));
    }

    let mut t = *obj;
    let mut skip_existing = skip_existing_check;

    for _ in 0..MAXTAGLOOP {
        // Check if t is a table — use inline fasttm for __newindex
        if let Some(table) = t.as_table_mut() {
            let tm_val =
                get_metamethod_from_meta_ptr(lua_state, table.meta_ptr(), TmKind::NewIndex);

            if tm_val.is_none() {
                // No metamethod - set directly
                lua_state.raw_set(&t, *key, value);
                return Ok(true);
            }

            // Check if key already exists in the table.
            // If it does, do a raw set regardless of __newindex.
            if !skip_existing {
                if let Some(existing) = table.raw_get(key)
                    && !existing.is_nil()
                {
                    lua_state.raw_set(&t, *key, value);
                    return Ok(true);
                }
            } else {
                skip_existing = false;
            }

            // Key does not exist - call __newindex metamethod
            if let Some(tm) = tm_val {
                if tm.is_function() {
                    metamethod::call_tm(lua_state, tm, t, *key, value)?;
                    return Ok(true);
                }

                // Metamethod is a table - repeat assignment over 'tm'
                t = tm;
                continue;
            }
        } else {
            // Not a table — try trait-based set_field for userdata first
            if t.ttisfulluserdata()
                && let Some(ud) = t.as_userdata_mut()
                && let Some(key_str) = key.as_str()
            {
                let udv = lua_value_to_udvalue(&value);
                match ud.get_trait_mut() {
                    Ok(trait_obj) => match trait_obj.set_field(key_str, udv) {
                        Some(Ok(())) => return Ok(true),
                        Some(Err(msg)) => {
                            return Err(lua_state.error(msg));
                        }
                        None => {} // Fall through to metatable
                    },
                    Err(e) => return Err(e),
                }
            }
            // Get __newindex metamethod
            if let Some(tm) = get_metamethod_event(lua_state, &t, TmKind::NewIndex) {
                if tm.is_function() {
                    metamethod::call_tm(lua_state, tm, t, *key, value)?;

                    return Ok(true);
                }

                // Metamethod is a table
                t = tm;
                continue;
            }

            // No metamethod found for non-table
            return Err(typeerror(lua_state, &t, "index"));
        }
    }

    // Too many iterations - possible loop
    Err(lua_state.error("'__newindex' chain too long; possible loop".to_string()))
}

pub fn get_metamethod_event(
    lua_state: &mut LuaState,
    value: &LuaValue,
    tm_kind: TmKind,
) -> Option<LuaValue> {
    let mt = get_metatable(lua_state, value)?;
    get_metamethod_from_metatable(lua_state, mt, tm_kind)
}

/// Get binary operation metamethod from either of two values
/// Checks v1's metatable first, then v2's if not found
pub fn get_binop_metamethod(
    lua_state: &mut LuaState,
    v1: &LuaValue,
    v2: &LuaValue,
    tm_kind: TmKind,
) -> Option<LuaValue> {
    // Try v1's metatable first
    if let Some(mt) = get_metatable(lua_state, v1)
        && let Some(mm) = get_metamethod_from_metatable(lua_state, mt, tm_kind)
    {
        return Some(mm);
    }

    // Try v2's metatable
    if let Some(mt) = get_metatable(lua_state, v2)
        && let Some(mm) = get_metamethod_from_metatable(lua_state, mt, tm_kind)
    {
        return Some(mm);
    }

    None
}

/// Get metatable for any value type
pub fn get_metatable(lua_state: &mut LuaState, value: &LuaValue) -> Option<LuaValue> {
    if let Some(table) = value.as_table_mut() {
        return table.get_metatable();
    } else if let Some(ud) = value.as_userdata_mut() {
        return ud.get_metatable();
    }
    // Basic types: use global type metatable
    lua_state
        .global_state_mut()
        .get_basic_metatable(value.kind())
}

/// Finish a C frame left on the call stack after yield-resume.
/// This is the Rust equivalent of Lua 5.5's finishCcall.
#[cold]
#[inline(never)]
fn finish_c_frame(lua_state: &mut LuaState, ci: &CallInfo) -> LuaResult<()> {
    let pcall_func_pos = ci.base - ci.func_offset as usize;
    let nresults = ci.nresults();
    let call_status = ci.call_status;
    let has_recst = call_status & CIST_RECST != 0;
    let is_xpcall = call_status & CIST_XPCALL != 0;

    if call_status & CIST_YPCALL != 0 {
        if has_recst {
            // Save handler before it gets overwritten (only for xpcall)
            let handler = if is_xpcall {
                lua_state.stack_get(pcall_func_pos).unwrap_or_default()
            } else {
                LuaValue::nil()
            };

            // Error recovery completed (or continuing) after yield.
            // Retrieve the saved error value and try to close remaining TBC entries.
            let error_val = lua_state.take_error_object();
            lua_state.clear_error();
            let close_level = pcall_func_pos + 1; // body's base position

            // Try to close remaining TBC entries
            let close_result = lua_state.close_tbc_with_error(close_level, error_val);

            match close_result {
                Ok(()) => {
                    // All TBC entries closed. Set up (false, error) result.
                    let final_err = lua_state.take_error_object();
                    let result_err = if !final_err.is_nil() {
                        final_err
                    } else {
                        error_val
                    };
                    lua_state.clear_error();

                    // If xpcall, call error handler to transform the error
                    let result_err = if is_xpcall {
                        lua_state.nny += 1;
                        let handler_result = lua_state.pcall(handler, vec![result_err]);
                        lua_state.nny -= 1;
                        match handler_result {
                            Ok((true, results)) => {
                                results.into_iter().next().unwrap_or(LuaValue::nil())
                            }
                            _ => lua_state.create_string("error in error handling")?,
                        }
                    } else {
                        result_err
                    };

                    lua_state.stack_set(pcall_func_pos, LuaValue::boolean(false))?;
                    lua_state.stack_set(pcall_func_pos + 1, result_err)?;
                    let n = 2;

                    // Pop pcall C frame
                    lua_state.pop_frame();

                    // Handle nresults adjustment
                    let final_n = if nresults == -1 { n } else { nresults as usize };
                    let new_top = pcall_func_pos + final_n;

                    if nresults >= 0 {
                        let wanted = nresults as usize;
                        for i in n..wanted {
                            lua_state.stack_set(pcall_func_pos + i, LuaValue::nil())?;
                        }
                    }

                    lua_state.set_top_raw(new_top);

                    // Restore caller frame top
                    if lua_state.call_depth() > 0 {
                        let ci_idx = lua_state.call_depth() - 1;
                        if nresults == -1 {
                            let ci_top = lua_state.get_call_info(ci_idx).top as usize;
                            if ci_top < new_top {
                                lua_state.get_call_info_mut(ci_idx).top = new_top as u32;
                            }
                        } else {
                            let frame_top = lua_state.get_call_info(ci_idx).top as usize;
                            lua_state.set_top_raw(frame_top);
                        }
                    }

                    Ok(())
                }
                Err(LuaError::Yield) => {
                    // Another TBC close method yielded. Save cascaded error and yield.
                    let had_cascaded = lua_state.has_error_object();
                    let cascaded = lua_state.take_error_object();
                    lua_state.set_error_object(if had_cascaded { cascaded } else { error_val });
                    Err(LuaError::Yield)
                }
                Err(e) => {
                    // TBC close threw — propagate as error
                    Err(e)
                }
            }
        } else {
            // pcall body completed successfully after yield.
            // Body's return values are at pcall_func_pos + 1 … top-1.
            // We need: [true, res1, res2, ...] starting at pcall_func_pos.
            let stack_top = lua_state.get_top();
            let body_results_start = pcall_func_pos + 1;
            let body_nres = stack_top.saturating_sub(body_results_start);

            // Place true at pcall_func_pos (body results already at +1)
            lua_state.stack_set(pcall_func_pos, LuaValue::boolean(true))?;

            let n = 1 + body_nres; // total results: true + body results

            // Pop pcall C frame
            lua_state.pop_frame();

            // Handle nresults adjustment (same as call_c_function post-processing)
            let final_n = if nresults == -1 { n } else { nresults as usize };
            let new_top = pcall_func_pos + final_n;

            if nresults >= 0 {
                let wanted = nresults as usize;
                // Pad with nil if needed
                for i in n..wanted {
                    lua_state.stack_set(pcall_func_pos + i, LuaValue::nil())?;
                }
            }

            lua_state.set_top_raw(new_top);

            // Restore caller frame top
            if lua_state.call_depth() > 0 {
                let ci_idx = lua_state.call_depth() - 1;
                if nresults == -1 {
                    let ci_top = lua_state.get_call_info(ci_idx).top as usize;
                    if ci_top < new_top {
                        lua_state.get_call_info_mut(ci_idx).top = new_top as u32;
                    }
                } else {
                    let frame_top = lua_state.get_call_info(ci_idx).top as usize;
                    lua_state.set_top_raw(frame_top);
                }
            }

            Ok(())
        }
    } else if call_status & CIST_YCALL != 0 {
        // Unprotected call (e.g. dofile) completed after yield.
        // Move results from body_start to func_pos (no true/false prefix).
        let stack_top = lua_state.get_top();
        let body_results_start = pcall_func_pos + 1;
        let body_nres = stack_top.saturating_sub(body_results_start);

        // Move results down to func_pos
        for i in 0..body_nres {
            let val = lua_state
                .stack_get(body_results_start + i)
                .unwrap_or_default();
            lua_state.stack_set(pcall_func_pos + i, val)?;
        }

        let n = body_nres;

        lua_state.pop_frame();

        let final_n = if nresults == -1 { n } else { nresults as usize };
        let new_top = pcall_func_pos + final_n;

        if nresults >= 0 {
            let wanted = nresults as usize;
            for i in n..wanted {
                lua_state.stack_set(pcall_func_pos + i, LuaValue::nil())?;
            }
        }

        lua_state.set_top_raw(new_top);

        if lua_state.call_depth() > 0 {
            let ci_idx = lua_state.call_depth() - 1;
            if nresults == -1 {
                let ci_top = lua_state.get_call_info(ci_idx).top as usize;
                if ci_top < new_top {
                    lua_state.get_call_info_mut(ci_idx).top = new_top as u32;
                }
            } else {
                let frame_top = lua_state.get_call_info(ci_idx).top as usize;
                lua_state.set_top_raw(frame_top);
            }
        }

        Ok(())
    } else {
        // Generic C frame after yield — just pop it.
        // This shouldn't normally happen, but be safe.
        lua_state.pop_frame();
        Ok(())
    }
}

#[inline(always)]
fn mark_pending_finish(ci: &mut CallInfo, aux: i32) {
    ci.set_pending_finish_get(aux);
    ci.call_status |= CIST_PENDING_FINISH;
}

/// Handle pending metamethod finish (cold path, extracted from main loop).
/// Returns true if a C frame was finished and execution should restart.
/// This is the equivalent of C Lua's luaV_finishOp.
#[cold]
#[inline(never)]
pub fn handle_pending_ops(lua_state: &mut LuaState, ci: &mut CallInfo) -> LuaResult<bool> {
    if ci.call_status & CIST_C != 0 {
        finish_c_frame(lua_state, ci)?;
        return Ok(true); // restart startfunc
    }
    // === luaV_finishOp equivalent ===
    // Extract needed CI fields upfront, then drop the borrow.
    let (saved_pc, base_tmp, pending_finish, _nresults, ci_chunk_ptr, ci_top) = {
        (
            ci.pc as usize,
            ci.base,
            ci.pending_finish_get(),
            ci.nresults(),
            ci.chunk_ptr,
            ci.top as usize,
        )
    };

    // Get the chunk to read the interrupted instruction
    if !ci_chunk_ptr.is_null() {
        let chunk = unsafe { &*ci_chunk_ptr };
        let code = &chunk.code;

        if saved_pc > 0 && saved_pc <= code.len() {
            let interrupted_instr = code[saved_pc - 1];
            let op = interrupted_instr.get_opcode();
            match op {
                OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK => {
                    // Arithmetic metamethod: result at stack[top-1],
                    // destination from the instruction at savedpc - 2
                    let top = lua_state.get_top();
                    if top > 0 && saved_pc >= 2 {
                        let arith_instr = code[saved_pc - 2];
                        let dest = if pending_finish >= 0 {
                            pending_finish as usize
                        } else {
                            base_tmp + arith_instr.get_a() as usize
                        };
                        let result = lua_state.stack_mut()[top - 1];
                        lua_state.stack_mut()[dest] = result;
                        lua_state.set_top_raw(top - 1);
                    }
                }
                OpCode::Unm
                | OpCode::BNot
                | OpCode::Len
                | OpCode::GetTabUp
                | OpCode::GetTable
                | OpCode::GetI
                | OpCode::GetField
                | OpCode::Self_ => {
                    // Unary/table get ops: result at stack[top-1],
                    // destination at base + A of the interrupted instruction
                    let top = lua_state.get_top();
                    if top > 0 {
                        let dest = if pending_finish >= 0 {
                            pending_finish as usize
                        } else {
                            base_tmp + interrupted_instr.get_a() as usize
                        };
                        let result = lua_state.stack_mut()[top - 1];
                        lua_state.stack_mut()[dest] = result;
                        lua_state.set_top_raw(top - 1);
                    }
                }
                OpCode::Lt
                | OpCode::Le
                | OpCode::LtI
                | OpCode::LeI
                | OpCode::GtI
                | OpCode::GeI
                | OpCode::Eq => {
                    // Comparison ops: truthiness of stack[top-1] is the result.
                    // Next instruction should be JMP.
                    // If result != k, skip the JMP.
                    let top = lua_state.get_top();
                    if top > 0 {
                        let res_val = lua_state.stack_mut()[top - 1];
                        let res = !res_val.is_nil() && !(res_val == LuaValue::boolean(false));
                        lua_state.set_top_raw(top - 1);
                        let k = interrupted_instr.get_k();
                        if res != k {
                            // Skip the JMP instruction
                            ci.pc += 1;
                        }
                    }
                }
                OpCode::Concat => {
                    // Port of C Lua 5.5's finishOp for OP_CONCAT (lvm.c:882-893)
                    // After yield in __concat metamethod, the result is at top-1.
                    // We must copy it to concat_top - 2 and continue if elements remain.
                    let top = lua_state.get_top();
                    if top > 0 {
                        let a = interrupted_instr.get_a() as usize;
                        let n = interrupted_instr.get_b() as usize;
                        let concat_top = base_tmp + a + n;
                        let result = lua_state.stack_mut()[top - 1];
                        lua_state.stack_mut()[concat_top - 2] = result;
                        let total = concat_top - 1 - (base_tmp + a);
                        if total > 1 {
                            lua_state.set_top_raw(concat_top - 1);
                            concat(lua_state, total)?;
                        }
                    }
                }
                _ => {
                    // CALL, TAILCALL, TFORCALL, SETTAB*, SETFIELD, SETI — no special action needed
                }
            }
        }
    }

    // Restore ci_top
    let current_top = lua_state.get_top();
    if current_top < ci_top {
        lua_state.set_top_raw(ci_top);
    }

    ci.set_pending_finish_get(-1);
    ci.call_status &= !CIST_PENDING_FINISH;
    Ok(false) // continue to hot path
}

pub fn objlen(
    l: &mut LuaState,
    ci: &mut CallInfo,
    dest_stk_id: StkId,
    value: LuaValue,
) -> LuaResult<()> {
    if let Some(bytes) = value.as_bytes() {
        let len = bytes.len();
        dest_stk_id.set_integer(len as i64);
        return Ok(());
    } else if value.ttistable() {
        if let Some(tm) = get_metamethod_event(l, &value, TmKind::Len) {
            return match call_tm_res_into(l, tm, value, value, dest_stk_id) {
                Ok(()) => Ok(()),
                Err(LuaError::Yield) => {
                    let aux = l.offset_of_stk_id(dest_stk_id);
                    mark_pending_finish(ci, aux);
                    Err(LuaError::Yield)
                }
                Err(e) => Err(e),
            };
        }

        let len = value.hvalue().len();
        dest_stk_id.set_integer(len as i64);
        return Ok(());
    } else {
        // Try trait-based __len for userdata first
        if value.ttisfulluserdata()
            && let Some(ud) = value.as_userdata_mut()
        {
            let trait_obj = ud.get_trait()?;
            if let Some(udv) = trait_obj.lua_len() {
                let result = udvalue_to_lua_value(l, udv)?;
                dest_stk_id.set_integer(result.as_integer().unwrap_or(0));
                return Ok(());
            }
        }
    }

    let tm = get_metamethod_event(l, &value, TmKind::Len);
    if let Some(tm) = tm {
        match call_tm_res_into(l, tm, value, value, dest_stk_id) {
            Ok(()) => {}
            Err(LuaError::Yield) => {
                let aux = l.offset_of_stk_id(dest_stk_id);
                mark_pending_finish(ci, aux);
                return Err(LuaError::Yield);
            }
            Err(e) => return Err(e),
        }
    } else {
        return Err(typeerror(l, &value, "get length of"));
    }
    Ok(())
}

/// Equality comparison - direct port of Lua 5.5's luaV_equalobj
/// Returns true if values are equal, false otherwise
/// Handles metamethods for tables and userdata
pub fn equalobj(lua_state: &mut LuaState, t1: LuaValue, t2: LuaValue) -> LuaResult<bool> {
    // Direct port of lvm.c:582 luaV_equalobj
    if t1 == t2 {
        return Ok(true);
    }

    if t1.tt() != t2.tt() {
        return Ok(false);
    }

    if t1.ttisfulluserdata() {
        // Userdata: first check identity
        if let (Some(u_ptr1), Some(u_ptr2)) = (t1.as_userdata_ptr(), t2.as_userdata_ptr())
            && u_ptr1 == u_ptr2
        {
            return Ok(true);
        }
        // Try trait-based lua_eq before metatable
        if let Some(ud1) = t1.as_userdata_mut()
            && let Some(ud2) = t2.as_userdata_mut()
            && let Some(result) = {
                let t1 = ud1.get_trait()?;
                let t2 = ud2.get_trait()?;
                t1.lua_eq(t2)
            }
        {
            return Ok(result);
        }
        // Different userdata - try __eq metamethod
        let tm = get_binop_metamethod(lua_state, &t1, &t2, TmKind::Eq);

        if let Some(metamethod) = tm {
            let result = call_tm_res(lua_state, metamethod, t1, t2)?;
            return Ok(!result.is_falsy());
        } else {
            return Ok(false);
        }
    }

    if t1.ttistable() {
        // Tables: first check identity
        if let (Some(t_ptr1), Some(t_ptr2)) = (t1.as_table_ptr(), t2.as_table_ptr())
            && t_ptr1 == t_ptr2
        {
            return Ok(true);
        }
        // Different tables - try __eq metamethod
        let tm = get_binop_metamethod(lua_state, &t1, &t2, TmKind::Eq);
        if let Some(metamethod) = tm {
            let result = call_tm_res(lua_state, metamethod, t1, t2)?;
            return Ok(!result.is_falsy());
        } else {
            return Ok(false);
        }
    }

    if t1.ttiscfunction() {
        // C functions: compare function pointers
        return Ok(unsafe { std::ptr::fn_addr_eq(t1.value.f, t2.value.f) });
    }

    // Lua functions, threads, etc.: compare GC pointers
    if let (Some(f_ptr1), Some(f_ptr2)) = (t1.as_function_ptr(), t2.as_function_ptr()) {
        return Ok(f_ptr1 == f_ptr2);
    }

    Ok(false)
}

pub fn forprep(lua_state: &mut LuaState, ra: StkId) -> LuaResult<bool> {
    let r_init = ra;
    let r_limit = ra.offset(1);
    let r_step = ra.offset(2);
    if r_init.is_integer() && r_step.is_integer() {
        // Integer loop (init and step are integers)
        let init = r_init.ivalue();
        let step = r_step.ivalue();

        if step == 0 {
            return Err(lua_state.error("'for' step is zero".to_string()));
        }
        // forlimit: convert limit to integer per C Lua 5.5 logic
        let (limit, should_skip) = for_limit(lua_state, r_limit, init, step)?;

        if should_skip {
            return Ok(true);
        }

        // Check if loop should be skipped based on direction
        if step > 0 {
            if init > limit {
                return Ok(true); // skip: init already past limit
            }
        } else if limit > init {
            return Ok(true); // skip: init already past limit (counting down)
        }

        {
            let count = if step > 0 {
                ((limit as u64).wrapping_sub(init as u64)) / (step as u64)
            } else {
                let step_abs = if step == i64::MIN {
                    i64::MAX as u64 + 1
                } else {
                    (-step) as u64
                };
                ((init as u64).wrapping_sub(limit as u64)) / step_abs
            };

            ra.change_i(count as i64);
            r_limit.set_integer(step);
            r_step.change_i(init);
        }
    } else {
        // Float loop — delegate to existing handler
        let mut init = 0.0;
        let mut limit = 0.0;
        let mut step = 0.0;

        if !for_tonumber(r_limit, &mut limit) {
            let t = objtypename(lua_state, r_limit.get_ref());
            return Err(lua_state.error(format!("bad 'for' limit (number expected, got {})", t)));
        }
        if !for_tonumber(r_step, &mut step) {
            let t = objtypename(lua_state, r_step.get_ref());
            return Err(lua_state.error(format!("bad 'for' step (number expected, got {})", t)));
        }
        if !for_tonumber(ra, &mut init) {
            let t = objtypename(lua_state, ra.get_ref());
            return Err(lua_state.error(format!(
                "bad 'for' initial value (number expected, got {})",
                t
            )));
        }

        if step == 0.0 {
            return Err(lua_state.error("'for' step is zero".to_string()));
        }

        let should_skip = if step > 0.0 {
            limit < init
        } else {
            init < limit
        };

        if should_skip {
            return Ok(true);
        } else {
            ra.set_float(limit);
            r_limit.set_float(step);
            r_step.set_float(init);
        }
    }

    Ok(false)
}

/// Port of C Lua 5.5's luaV_tonumber_ (lvm.c).
/// Full conversion: handles strings in addition to integers/floats.
/// Used by forprep where operands may be strings.
/// Marked cold to keep the arithmetic hot path (ptonumberns) compact.
fn for_tonumber(v: StkId, out: &mut f64) -> bool {
    if v.is_float() {
        *out = v.fltvalue();
        true
    } else if v.is_integer() {
        *out = v.ivalue() as f64;
        true
    } else if v.get_ref().is_string() {
        if let Some(s) = v.get_ref().as_str() {
            let result = parse_lua_number(s);
            if result.is_float() {
                *out = result.fltvalue();
                true
            } else if result.is_integer() {
                *out = result.ivalue() as f64;
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    }
}

fn for_limit(
    lua_state: &mut LuaState,
    r_limit: StkId,
    init: i64,
    step: i64,
) -> LuaResult<(i64, bool)> {
    // Port of C Lua 5.5's forlimit (lvm.c:181-198)
    // Try converting the limit to integer (with floor or ceil depending on step direction)
    let mode = if step < 0 { 2 } else { 1 }; // 1=floor, 2=ceil
    if let Some(lim) = tointeger_mode(r_limit.get_ref(), mode) {
        // Successfully converted to integer
        let skip = if step > 0 { init > lim } else { init < lim };
        Ok((lim, skip))
    } else {
        // Not coercible to integer. Try converting to float to check bounds.
        let mut flimit = 0.0;
        if !for_tonumber(r_limit, &mut flimit) {
            return Err(error_for_bad_limit(lua_state, r_limit.get_ref()));
        }
        // flim is a float out of integer bounds
        if 0.0 < flimit {
            // Limit is above max integer
            if step < 0 {
                return Ok((i64::MAX, true)); // skip
            }
            Ok((i64::MAX, false)) // truncate, caller checks init > limit
        } else {
            // Limit is below min integer
            if step > 0 {
                return Ok((i64::MIN, true)); // skip
            }
            Ok((i64::MIN, false)) // truncate, caller checks init < limit
        }
    }
}

#[cold]
#[inline(never)]
pub fn float_for_loop(ra: StkId) -> bool {
    let step = ra.offset(1).fltvalue();
    let limit = ra.fltvalue();
    let mut idx = ra.offset(2).fltvalue();
    idx += step;
    if (step > 0.0 && idx <= limit) || (step <= 0.0 && idx >= limit) {
        ra.offset(2).change_f(idx);
        return true;
    }

    false
}

#[cold]
#[inline(never)]
fn error_for_bad_limit(lua_state: &mut LuaState, limit_val: &LuaValue) -> LuaError {
    let t = objtypename(lua_state, limit_val);
    lua_state.error(format!("bad 'for' limit (number expected, got {})", t))
}

/// Cold error: attempt to divide by zero (IDIV)
#[cold]
#[inline(never)]
pub fn error_div_by_zero(lua_state: &mut LuaState) -> LuaError {
    lua_state.error("attempt to divide by zero".to_string())
}

/// Cold error: attempt to perform 'n%0' (MOD)
#[cold]
#[inline(never)]
pub fn error_mod_by_zero(lua_state: &mut LuaState) -> LuaError {
    lua_state.error("attempt to perform 'n%0'".to_string())
}

#[cold]
#[inline(never)]
pub fn error_global(lua_state: &mut LuaState, global_name: &str) -> LuaError {
    lua_state.error(format!("global '{}' already defined", global_name))
}

/// Cold path: comparison metamethod fallback for LtI/LeI/GtI/GeI/Lt/Le
/// Extraced from execute_loop to reduce main function size and improve register allocation.
#[cold]
#[inline(never)]
pub fn order_tm_fallback(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    va: LuaValue,
    vb: LuaValue,
    tm: TmKind,
) -> LuaResult<bool> {
    use crate::lua_vm::execute::metamethod::try_comp_tm;
    match try_comp_tm(lua_state, va, vb, tm) {
        Ok(Some(result)) => Ok(result),
        Ok(None) => Err(ordererror(lua_state, &va, &vb)),
        Err(LuaError::Yield) => {
            mark_pending_finish(ci, -1);
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Cold path: binary metamethod fallback for MmBin/MmBinI/MmBinK
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub fn bin_tm_fallback(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    ra: LuaValue,
    rb: LuaValue,
    result_reg: u32,
    a_reg: u32,
    b_reg: u32,
    tm: TmKind,
) -> LuaResult<()> {
    use crate::lua_vm::execute::metamethod::try_bin_tm;
    match try_bin_tm(lua_state, ra, rb, result_reg, a_reg, b_reg, tm) {
        Ok(_) => Ok(()),
        Err(LuaError::Yield) => {
            mark_pending_finish(ci, result_reg as i32);
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Cold path: unary metamethod fallback for Unm/BNot/Len
#[cold]
#[inline(never)]
pub fn unary_tm_fallback(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    rb: LuaValue,
    result_reg: usize,
    tm: TmKind,
) -> LuaResult<()> {
    use crate::lua_vm::execute::metamethod::try_unary_tm;
    match try_unary_tm(lua_state, rb, result_reg, tm) {
        Ok(_) => Ok(()),
        Err(LuaError::Yield) => {
            mark_pending_finish(ci, result_reg as i32);
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// finishget wrapper for GetTabUp/GetTable/GetI/GetField/Self_
/// Handles __index metamethod chain + yield propagation.
/// NOT #[cold]: __index is a common OOP operation.
#[inline(never)]
pub fn finishget_fallback(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    obj: &LuaValue,
    key: &LuaValue,
    dest_stk_id: StkId,
) -> LuaResult<()> {
    match finishget_to_reg_known_miss(lua_state, obj, key, dest_stk_id) {
        Ok(()) => Ok(()),
        Err(LuaError::Yield) => {
            let aux = lua_state.offset_of_stk_id(dest_stk_id);
            mark_pending_finish(ci, aux);
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Fast path for OOP-style `SELF_` lookups where the miss is resolved by a
/// table-only `__index` chain and the key is a short string.
/// Returns true if a value was found and written directly to `dest_stk_id`.
#[inline(never)]
pub fn self_shortstr_index_chain_fast(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    dest_stk_id: StkId,
) -> bool {
    const TM_INDEX_BIT: u8 = TmKind::Index as u8;

    debug_assert!(key.is_short_string());

    let event_key = lua_state
        .global_state_mut()
        .const_strings
        .get_tm_value(TmKind::Index);
    let mut current = *obj;

    for _ in 0..MAXTAGLOOP {
        let Some(table) = current.as_table_mut() else {
            return false;
        };

        if let Some(value) = table.impl_table.get_shortstr_fast(key) {
            dest_stk_id.write(&value);
            return true;
        }

        let meta = table.meta_ptr();
        if meta.is_null() {
            return false;
        }

        let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
        if mt.no_tm(TM_INDEX_BIT) {
            return false;
        }

        let Some(tm) = mt.impl_table.get_shortstr_fast(&event_key) else {
            mt.set_tm_absent(TM_INDEX_BIT);
            return false;
        };

        if tm.is_function() || !tm.is_table() {
            return false;
        }

        current = tm;
    }

    false
}

fn finishget_to_reg_inner(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    dest_stk_id: StkId,
    skip_first_raw_lookup: bool,
) -> LuaResult<()> {
    const TM_INDEX_BIT: u8 = TmKind::Index as u8;

    let mut t = *obj;
    let mut skip_raw_lookup = skip_first_raw_lookup;

    for _ in 0..MAXTAGLOOP {
        let tm = if let Some(table) = t.as_table_mut() {
            if !skip_raw_lookup {
                if let Some(val) = table.raw_get(key) {
                    dest_stk_id.write(&val);
                    return Ok(());
                }
            } else {
                skip_raw_lookup = false;
            }

            let meta = table.meta_ptr();
            if meta.is_null() {
                dest_stk_id.set_nil();
                return Ok(());
            }
            let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
            if mt.no_tm(TM_INDEX_BIT) {
                dest_stk_id.set_nil();
                return Ok(());
            }
            let vm = lua_state.global_state_mut();
            let event_key = vm.const_strings.get_tm_value(TmKind::Index);
            match mt.impl_table.get_shortstr_fast(&event_key) {
                Some(v) => v,
                None => {
                    mt.set_tm_absent(TM_INDEX_BIT);
                    dest_stk_id.set_nil();
                    return Ok(());
                }
            }
        } else {
            if t.ttisfulluserdata()
                && let Some(ud) = t.as_userdata_mut()
            {
                let token = ud.sub_guard_token();
                let trait_obj = ud.get_trait()?;
                if let Some(key_str) = key.as_str()
                    && let Some(udv) = trait_obj.get_field(key_str)
                {
                    let result = udvalue_to_lua_value_with_token(lua_state, udv, token)?;
                    dest_stk_id.write(&result);
                    return Ok(());
                }
            }

            match get_metamethod_event(lua_state, &t, TmKind::Index) {
                Some(tm) => tm,
                None => {
                    return Err(typeerror(lua_state, &t, "index"));
                }
            }
        };

        if tm.is_function() {
            return call_tm_res_into(lua_state, tm, t, *key, dest_stk_id);
        }

        t = tm;
        if let Some(table) = t.as_table() {
            let value = if key.ttisinteger() {
                table.impl_table.fast_geti(key.ivalue())
            } else if key.is_short_string() {
                table.impl_table.get_shortstr_fast(key)
            } else {
                table.raw_get(key)
            };
            if let Some(value) = value {
                dest_stk_id.write(&value);
                return Ok(());
            }
            skip_raw_lookup = true;
        }
    }

    Err(lua_state.error("'__index' chain too long; possible loop".to_string()))
}

fn finishget_to_reg_known_miss(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    dest_stk_id: StkId,
) -> LuaResult<()> {
    finishget_to_reg_inner(lua_state, obj, key, dest_stk_id, true)
}

/// finishset wrapper for SetTabUp/SetTable/SetI/SetField
/// Handles __newindex metamethod chain + yield propagation.
/// NOT #[cold]: __newindex is a common OOP operation.
#[inline(never)]
pub fn finishset_fallback(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    obj: &LuaValue,
    key: &LuaValue,
    value: LuaValue,
    known_miss: bool,
) -> LuaResult<()> {
    match finishset(lua_state, obj, key, value, known_miss) {
        Ok(_) => Ok(()),
        Err(LuaError::Yield) => {
            mark_pending_finish(ci, -2);
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Cold path: equality metamethod fallback for Eq
#[inline(never)]
pub fn eq_fallback(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    ra: LuaValue,
    rb: LuaValue,
) -> LuaResult<bool> {
    match equalobj(lua_state, ra, rb) {
        Ok(eq) => Ok(eq),
        Err(LuaError::Yield) => {
            mark_pending_finish(ci, -1);
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Cold path: Return0 with active hooks — delegates to generic poscall
#[inline(never)]
pub fn return0_with_hook(lua_state: &mut LuaState, a_pos: usize, pc: usize) -> LuaResult<()> {
    lua_state.set_top_raw(a_pos);
    lua_state
        .current_frame_mut()
        .expect("saving pc requires an active call frame")
        .save_pc(pc);
    poscall(lua_state, 0, pc)
}

/// Cold path: Return1 with active hooks — delegates to generic poscall
#[inline(never)]
pub fn return1_with_hook(lua_state: &mut LuaState, a_pos: usize, pc: usize) -> LuaResult<()> {
    lua_state.set_top_raw(a_pos + 1);
    lua_state
        .current_frame_mut()
        .expect("saving pc requires an active call frame")
        .save_pc(pc);
    poscall(lua_state, 1, pc)
}

pub trait VmIndex {
    fn to_usize(self) -> usize;
}

impl VmIndex for usize {
    #[inline(always)]
    fn to_usize(self) -> usize {
        self
    }
}

impl VmIndex for u8 {
    #[inline(always)]
    fn to_usize(self) -> usize {
        self as usize
    }
}

impl VmIndex for u16 {
    #[inline(always)]
    fn to_usize(self) -> usize {
        self as usize
    }
}

impl VmIndex for u32 {
    #[inline(always)]
    fn to_usize(self) -> usize {
        self as usize
    }
}

impl VmIndex for i32 {
    #[inline(always)]
    fn to_usize(self) -> usize {
        self as usize
    }
}

#[inline(always)]
pub fn instr_at(code: &[Instruction], pc: usize) -> Instruction {
    unsafe { *code.get_unchecked(pc) }
}

#[inline(always)]
pub fn const_ref<I: VmIndex>(constants: &[LuaValue], index: I) -> &LuaValue {
    unsafe { constants.get_unchecked(index.to_usize()) }
}

#[inline(always)]
pub fn k_val<I: VmIndex>(constants: &[LuaValue], index: I) -> &LuaValue {
    const_ref(constants, index)
}

pub fn pk_val(constants: &[LuaValue], index: usize) -> StkId {
    StkId::from_const_ptr(const_ref(constants, index) as *const LuaValue)
}
