use crate::{
    CallInfo, LuaProto, LuaResult, LuaValue,
    gc::TablePtr,
    lua_value::{LUA_VNUMFLT, LUA_VNUMINT, udvalue_to_lua_value},
    lua_vm::{
        LuaError, LuaState, TmKind,
        call_info::call_status::{
            CIST_C, CIST_PENDING_FINISH, CIST_RECST, CIST_XPCALL, CIST_YCALL, CIST_YPCALL,
        },
        execute::{
            call::poscall, call_tm_res, call_tm_res1, concat::concat, metamethod::call_tm_res_into,
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
    ci: &mut CallInfo,
    chunk: &LuaProto,
    totalargs: usize,
    nfixparams: usize,
    _nextra: usize,
) -> LuaResult<usize> {
    let old_base = ci.base;
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
    // Safety: grow_stack above ensures stack is large enough for new_func_pos and new_base
    unsafe { *stack.get_unchecked_mut(new_func_pos) = *stack.get_unchecked(func_pos) };

    // Step 2: Copy fixed parameters to after new function position
    for i in 0..nfixparams {
        unsafe { *stack.get_unchecked_mut(new_base + i) = *stack.get_unchecked(func_pos + 1 + i) };
        // Erase original parameter with nil (for GC)
        unsafe {
            psetnilvalue(&mut stack[func_pos + 1 + i] as *mut LuaValue);
        }
    }

    ci.base = new_base;
    ci.top = (new_base + chunk.max_stack_size) as u32;
    ci.func_offset = (new_base - func_pos) as u32; // Distance from new_base to original func

    // Update lua_state.top to match call_info.top
    let new_call_info_top = new_base + chunk.max_stack_size;
    lua_state.set_top(new_call_info_top)?;

    Ok(new_base)
}

/// ttisinteger - 检查是否是整数 (最快的类型检查)
#[inline(always)]
pub unsafe fn pttisinteger(v: *const LuaValue) -> bool {
    unsafe { (*v).ttisinteger() }
}

/// ttisinteger_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn ttisinteger(v: &LuaValue) -> bool {
    v.ttisinteger()
}

/// ttisfloat - 检查是否是浮点数
#[inline(always)]
pub unsafe fn pttisfloat(v: *const LuaValue) -> bool {
    unsafe { (*v).ttisfloat() }
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
// OPTIMIZED: 指针版本，避免引用/解引用开销

/// ivalue - 直接获取整数值 (调用前必须用 ttisinteger 检查)
#[inline(always)]
pub unsafe fn pivalue(v: *const LuaValue) -> i64 {
    unsafe { (*v).ivalue() }
}

/// ivalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn ivalue(v: &LuaValue) -> i64 {
    v.ivalue()
}

#[inline(always)]
pub fn chgivalue(v: &mut LuaValue, i: i64) {
    v.value.i = i;
}

/// fltvalue - 直接获取浮点值 (调用前必须用 ttisfloat 检查)
#[inline(always)]
pub unsafe fn pfltvalue(v: *const LuaValue) -> f64 {
    unsafe { (*v).fltvalue() }
}

/// fltvalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn fltvalue(v: &LuaValue) -> f64 {
    v.fltvalue()
}

#[inline(always)]
pub fn chgfltvalue(v: &mut LuaValue, n: f64) {
    v.value.n = n;
}

/// setivalue - 设置整数值
/// OPTIMIZATION: Direct field access matching Lua 5.5's setivalue macro
#[inline(always)]
pub unsafe fn psetivalue(v: *mut LuaValue, i: i64) {
    unsafe {
        *v = LuaValue::integer(i);
    }
}

/// setivalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn setivalue(v: &mut LuaValue, i: i64) {
    *v = LuaValue::integer(i);
}

/// luai_numpow - Power operation matching Lua 5.5's luai_numpow macro
/// Special-cases b==2 to a*a (single multiply instead of costly pow())
#[inline(always)]
pub fn luai_numpow(a: f64, b: f64) -> f64 {
    if b == 2.0 { a * a } else { a.powf(b) }
}

/// setfltvalue - 设置浮点值  
/// OPTIMIZATION: Direct field access matching Lua 5.5's setfltvalue macro
#[inline(always)]
pub unsafe fn psetfltvalue(v: *mut LuaValue, n: f64) {
    unsafe {
        *v = LuaValue::float(n);
    }
}

/// setfltvalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn setfltvalue(v: &mut LuaValue, n: f64) {
    *v = LuaValue::float(n);
}

/// setbfvalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn setbfvalue(v: &mut LuaValue) {
    *v = LuaValue::boolean(false);
}

/// setbtvalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn setbtvalue(v: &mut LuaValue) {
    *v = LuaValue::boolean(true);
}

/// setnilvalue - 设置nil
#[inline(always)]
pub unsafe fn psetnilvalue(v: *mut LuaValue) {
    unsafe {
        *v = LuaValue::nil();
    }
}

/// setnilvalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn setnilvalue(v: &mut LuaValue) {
    *v = LuaValue::nil();
}

#[inline(always)]
pub fn setobjs2s(l: &mut LuaState, a: usize, b: usize) {
    let stack = l.stack_mut();
    unsafe {
        *stack.get_unchecked_mut(a) = *stack.get_unchecked(b);
    }
}

#[inline(always)]
pub fn setobj2s(l: &mut LuaState, a: usize, b: &LuaValue) {
    let stack = l.stack_mut();
    unsafe {
        *stack.get_unchecked_mut(a) = *b;
    }
}

/// luaV_shiftl - Shift integer x left by y positions.
/// If y is negative, shifts right (LOGICAL/unsigned shift).
/// Matches Lua 5.5's luaV_shiftl from lvm.c.
#[inline(always)]
pub fn lua_shiftl(x: i64, y: i64) -> i64 {
    if y < 0 {
        // Right shift (logical/unsigned)
        if y <= -64 {
            0
        } else {
            ((x as u64) >> ((-y) as u32)) as i64
        }
    } else {
        // Left shift
        if y >= 64 {
            0
        } else {
            ((x as u64) << (y as u32)) as i64
        }
    }
}

/// luaV_shiftr - Shift integer x right by y positions.
/// luaV_shiftr(x, y) = luaV_shiftl(x, -y)
#[inline(always)]
pub fn lua_shiftr(x: i64, y: i64) -> i64 {
    lua_shiftl(x, y.wrapping_neg())
}

/// Lua floor division for integers: a // b
/// Equivalent to luaV_idiv in Lua 5.5
#[inline(always)]
pub fn lua_idiv(a: i64, b: i64) -> i64 {
    // Handle overflow case: MIN_INT / -1 would overflow, wrapping gives MIN_INT (floor division same result)
    if b == -1 {
        return a.wrapping_neg();
    }
    let q = a / b;
    // If the signs of a and b differ and there is a remainder,
    // subtract 1 to achieve floor division (toward -infinity)
    if (a ^ b) < 0 && a % b != 0 {
        q.wrapping_sub(1)
    } else {
        q
    }
}

/// Lua modulo for integers: a % b
/// Equivalent to luaV_mod in Lua 5.5: m = a % b; if m != 0 && (m ^ b) < 0 then m += b
#[inline(always)]
pub fn lua_imod(a: i64, b: i64) -> i64 {
    // Handle overflow case: MIN_INT % -1 = 0
    if b == -1 {
        return 0;
    }
    let m = a % b;
    if m != 0 && (m ^ b) < 0 {
        m.wrapping_add(b)
    } else {
        m
    }
}

/// Float modulo matching C Lua's `luai_nummod`.
/// Uses hardware fmod (Rust's `%` operator on f64) then adjusts sign.
#[inline(always)]
pub fn lua_fmod(a: f64, b: f64) -> f64 {
    let mut m = a % b; // C fmod
    if m != 0.0 && ((m > 0.0) != (b > 0.0)) {
        m += b;
    }
    m
}

// ============ 类型转换辅助函数 ============

/// tointegerns - 尝试转换为整数 (不抛出错误)
/// 对应 Lua 的 tointegerns 宏
#[inline(always)]
pub unsafe fn ptointegerns(v: *const LuaValue, out: *mut i64) -> bool {
    unsafe {
        if pttisinteger(v) {
            *out = pivalue(v);
            true
        } else if pttisfloat(v) {
            // Try converting integral-valued floats (e.g. 5.0 -> 5)
            // Range check matches C Lua's lua_numbertointeger:
            //   f >= (i64::MIN as f64) && f < -(i64::MIN as f64)
            // Note: i64::MAX as f64 rounds UP to 2^63, so we must use strict <
            // with -(i64::MIN as f64) = 2^63 (exactly representable).
            let f = pfltvalue(v);
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
}

/// tointegerns_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn tointegerns(v: &LuaValue, out: &mut i64) -> bool {
    unsafe { ptointegerns(v as *const LuaValue, out as *mut i64) }
}

/// tonumber - convert LuaValue to f64, including string coercion.
/// Port of C Lua's luaV_tonumber_.
pub fn tonumber(v: &LuaValue, out: &mut f64) -> bool {
    if tonumberns(v, out) {
        return true;
    }
    if v.is_string() {
        let result = crate::stdlib::basic::parse_number::parse_lua_number(v.as_str().unwrap_or(""));
        if result.is_float() {
            *out = unsafe { result.value.n };
            return true;
        } else if result.is_integer() {
            *out = unsafe { result.value.i } as f64;
            return true;
        }
    }
    false
}

/// ptonumberns - 尝试转换为浮点数 (不抛出错误)
/// Only handles float and integer (NO string coercion).
#[inline(always)]
pub unsafe fn ptonumberns(v: *const LuaValue, out: *mut f64) -> bool {
    unsafe {
        if pttisfloat(v) {
            *out = pfltvalue(v);
            true
        } else if pttisinteger(v) {
            *out = pivalue(v) as f64;
            true
        } else {
            false
        }
    }
}

/// tonumberns - 引用版本
#[inline(always)]
pub fn tonumberns(v: &LuaValue, out: &mut f64) -> bool {
    unsafe { ptonumberns(v as *const LuaValue, out as *mut f64) }
}

/// tointeger - 从LuaValue引用获取整数 (用于常量)
#[inline(always)]
pub fn tointeger(v: &LuaValue, out: &mut i64) -> bool {
    if v.tt() == LUA_VNUMINT {
        unsafe {
            *out = v.value.i;
        }
        true
    } else if v.tt() == LUA_VNUMFLT {
        // Try converting integral-valued floats (e.g. 5.0 -> 5)
        // Range check: f must be in [i64::MIN, 2^63) — see ptointegerns for details.
        let f = unsafe { v.value.n };
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

/// Convert a LuaValue to integer using a rounding mode.
/// Port of C Lua 5.5's luaV_tointeger (lvm.c:157).
/// mode: 0 = exact only, 1 = floor, 2 = ceil
/// Handles integers, floats, and strings.
fn tointeger_mode(v: &LuaValue, mode: i32) -> Option<i64> {
    if v.tt() == LUA_VNUMINT {
        return Some(unsafe { v.value.i });
    }
    // Get as float (including string conversion)
    let f = if v.tt() == LUA_VNUMFLT {
        unsafe { v.value.n }
    } else if v.is_string() {
        let result = crate::stdlib::basic::parse_number::parse_lua_number(v.as_str().unwrap_or(""));
        if result.is_float() {
            unsafe { result.value.n }
        } else if result.is_integer() {
            return Some(unsafe { result.value.i });
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
                // Try trait-based get_field (key must be a string)
                // This handles both field access AND method lookup
                // (methods return UdValue::Function(cfunction))
                if let Some(key_str) = key.as_str()
                    && let Some(udv) = ud.get_trait().get_field(key_str)
                {
                    let result = crate::lua_value::udvalue_to_lua_value(lua_state, udv)?;
                    return Ok(Some(result));
                }
            }
            // Fall back to general metamethod path
            match get_metamethod_event(lua_state, &t, TmKind::Index) {
                Some(tm) => tm,
                None => {
                    // No __index metamethod on non-table value → error
                    // Use typeerror for enhanced error message with varinfo
                    return Err(crate::stdlib::debug::typeerror(lua_state, &t, "index"));
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

#[cfg(feature = "jit")]
pub fn finishget_known_miss(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
) -> LuaResult<Option<LuaValue>> {
    finishget_inner(lua_state, obj, key, true)
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

    let vm = lua_state.vm_mut();
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
fn finishset_inner(
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
                    use crate::lua_vm::execute;
                    execute::metamethod::call_tm(lua_state, tm, t, *key, value)?;
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
                let udv = crate::lua_value::lua_value_to_udvalue(&value);
                match ud.get_trait_mut().set_field(key_str, udv) {
                    Some(Ok(())) => return Ok(true),
                    Some(Err(msg)) => {
                        return Err(lua_state.error(msg));
                    }
                    None => {} // Fall through to metatable
                }
            }
            // Get __newindex metamethod
            if let Some(tm) = get_metamethod_event(lua_state, &t, TmKind::NewIndex) {
                if tm.is_function() {
                    // Call metamethod
                    use crate::lua_vm::execute;

                    execute::metamethod::call_tm(lua_state, tm, t, *key, value)?;

                    return Ok(true);
                }

                // Metamethod is a table
                t = tm;
                continue;
            }

            // No metamethod found for non-table
            return Err(crate::stdlib::debug::typeerror(lua_state, &t, "index"));
        }
    }

    // Too many iterations - possible loop
    Err(lua_state.error("'__newindex' chain too long; possible loop".to_string()))
}

pub fn finishset(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    value: LuaValue,
) -> LuaResult<bool> {
    finishset_inner(lua_state, obj, key, value, false)
}

pub fn finishset_known_miss(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    value: LuaValue,
) -> LuaResult<bool> {
    finishset_inner(lua_state, obj, key, value, true)
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
    lua_state.vm_mut().get_basic_metatable(value.kind())
}

/// Finish a C frame left on the call stack after yield-resume.
/// This is the Rust equivalent of Lua 5.5's finishCcall.
#[cold]
#[inline(never)]
fn finish_c_frame(lua_state: &mut LuaState, ci: &mut CallInfo) -> LuaResult<()> {
    let pcall_func_pos = ci.base - ci.func_offset as usize;
    let nresults = ci.nresults();
    let has_recst = ci.call_status & CIST_RECST != 0;
    let is_xpcall = ci.call_status & CIST_XPCALL != 0;

    if ci.call_status & CIST_YPCALL != 0 {
        if has_recst {
            // Save handler before it gets overwritten (only for xpcall)
            let handler = if is_xpcall {
                lua_state.stack_get(pcall_func_pos).unwrap_or_default()
            } else {
                LuaValue::nil()
            };

            // Error recovery completed (or continuing) after yield.
            // Retrieve the saved error value and try to close remaining TBC entries.
            let error_val = std::mem::take(&mut lua_state.error_object);
            lua_state.clear_error();
            let close_level = pcall_func_pos + 1; // body's base position

            // Try to close remaining TBC entries
            let close_result = lua_state.close_tbc_with_error(close_level, error_val);

            match close_result {
                Ok(()) => {
                    // All TBC entries closed. Set up (false, error) result.
                    let final_err = std::mem::take(&mut lua_state.error_object);
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
                    let cascaded = std::mem::take(&mut lua_state.error_object);
                    lua_state.error_object = if !cascaded.is_nil() {
                        cascaded
                    } else {
                        error_val
                    };
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
    } else if ci.call_status & CIST_YCALL != 0 {
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
    // The interrupted instruction is at savedpc - 1.
    // We need to check what opcode was interrupted and handle accordingly.
    let saved_pc = ci.pc as usize;
    let base_tmp = ci.base;
    let _nresults = ci.nresults();

    // Get the chunk to read the interrupted instruction
    if !ci.chunk_ptr.is_null() {
        let chunk = unsafe { &*ci.chunk_ptr };
        let code = &chunk.code;

        if saved_pc > 0 && saved_pc <= code.len() {
            let interrupted_instr = code[saved_pc - 1];
            let op = interrupted_instr.get_opcode();

            use crate::lua_vm::OpCode;
            match op {
                OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK => {
                    // Arithmetic metamethod: result at stack[top-1],
                    // destination from the instruction at savedpc - 2
                    let top = lua_state.get_top();
                    if top > 0 && saved_pc >= 2 {
                        let arith_instr = code[saved_pc - 2];
                        let dest = base_tmp + arith_instr.get_a() as usize;
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
                        let dest = base_tmp + interrupted_instr.get_a() as usize;
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
    let ci_top = ci.top as usize;
    let current_top = lua_state.get_top();
    if current_top < ci_top {
        lua_state.set_top_raw(ci_top);
    }

    ci.set_pending_finish_get(-1);
    ci.call_status &= !CIST_PENDING_FINISH;
    Ok(false) // continue to hot path
}

pub fn objlen(l: &mut LuaState, result_reg: usize, value: LuaValue) -> LuaResult<()> {
    let result = objlen_value(l, value)?;
    setobj2s(l, result_reg, &result);
    Ok(())
}

pub fn objlen_value(l: &mut LuaState, value: LuaValue) -> LuaResult<LuaValue> {
    if let Some(bytes) = value.as_bytes() {
        let len = bytes.len();
        return Ok(LuaValue::integer(len as i64));
    } else if value.ttistable() {
        if let Some(tm) = value
            .as_table()
            .and_then(|table| get_metamethod_from_meta_ptr(l, table.meta_ptr(), TmKind::Len))
        {
            return call_tm_res1(l, tm, value);
        }

        let len = value.as_table().unwrap().len();
        return Ok(LuaValue::integer(len as i64));
    } else {
        // Try trait-based __len for userdata first
        if value.ttisfulluserdata()
            && let Some(ud) = value.as_userdata_mut()
            && let Some(udv) = ud.get_trait().lua_len()
        {
            let result = udvalue_to_lua_value(l, udv)?;
            return Ok(LuaValue::integer(result.as_integer().unwrap_or(0)));
        }
    }

    let tm = get_metamethod_event(l, &value, TmKind::Len);
    if let Some(tm) = tm {
        call_tm_res1(l, tm, value)
    } else {
        Err(crate::stdlib::debug::typeerror(l, &value, "get length of"))
    }
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
            && let Some(result) = ud1.get_trait().lua_eq(ud2.get_trait())
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
        return Ok(unsafe { t1.value.f == t2.value.f });
    }

    // Lua functions, threads, etc.: compare GC pointers
    if let (Some(f_ptr1), Some(f_ptr2)) = (t1.as_function_ptr(), t2.as_function_ptr()) {
        return Ok(f_ptr1 == f_ptr2);
    }

    Ok(false)
}

pub fn forprep(lua_state: &mut LuaState, ra_pos: usize) -> LuaResult<bool> {
    let stack = lua_state.stack();
    let init_pos = ra_pos;
    let limit_pos = ra_pos + 1;
    let step_pos = ra_pos + 2;
    if ttisinteger(unsafe { stack.get_unchecked(init_pos) })
        && ttisinteger(unsafe { stack.get_unchecked(step_pos) })
    {
        // Integer loop (init and step are integers)
        let init = ivalue(unsafe { stack.get_unchecked(init_pos) });
        let step = ivalue(unsafe { stack.get_unchecked(step_pos) });

        if step == 0 {
            return Err(lua_state.error("'for' step is zero".to_string()));
        }
        // forlimit: convert limit to integer per C Lua 5.5 logic
        let limit_val = *unsafe { stack.get_unchecked(limit_pos) };
        let (limit, should_skip) = for_limit(lua_state, limit_val, init, step)?;

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

            let stack = lua_state.stack_mut();
            chgivalue(unsafe { stack.get_unchecked_mut(ra_pos) }, count as i64);
            setivalue(unsafe { stack.get_unchecked_mut(ra_pos + 1) }, step);
            chgivalue(unsafe { stack.get_unchecked_mut(ra_pos + 2) }, init);
        }
    } else {
        // Float loop — delegate to existing handler
        let mut init = 0.0;
        let mut limit = 0.0;
        let mut step = 0.0;

        // Copy values for potential error messages (avoids borrow conflict)
        let init_val = stack[init_pos];
        let limit_val = stack[limit_pos];
        let step_val = stack[step_pos];

        if !tonumber(&limit_val, &mut limit) {
            let t = crate::stdlib::debug::objtypename(lua_state, &limit_val);
            return Err(lua_state.error(format!("bad 'for' limit (number expected, got {})", t)));
        }
        if !tonumber(&step_val, &mut step) {
            let t = crate::stdlib::debug::objtypename(lua_state, &step_val);
            return Err(lua_state.error(format!("bad 'for' step (number expected, got {})", t)));
        }
        if !tonumber(&init_val, &mut init) {
            let t = crate::stdlib::debug::objtypename(lua_state, &init_val);
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

        let stack = lua_state.stack_mut();
        if should_skip {
            return Ok(true);
        } else {
            setfltvalue(&mut stack[ra_pos], limit);
            setfltvalue(&mut stack[ra_pos + 1], step);
            setfltvalue(&mut stack[ra_pos + 2], init);
        }
    }

    Ok(false)
}

fn for_limit(
    lua_state: &mut LuaState,
    limit_val: LuaValue,
    init: i64,
    step: i64,
) -> LuaResult<(i64, bool)> {
    // Port of C Lua 5.5's forlimit (lvm.c:181-198)
    // Try converting the limit to integer (with floor or ceil depending on step direction)
    let mode = if step < 0 { 2 } else { 1 }; // 1=floor, 2=ceil
    if let Some(lim) = tointeger_mode(&limit_val, mode) {
        // Successfully converted to integer
        let skip = if step > 0 { init > lim } else { init < lim };
        Ok((lim, skip))
    } else {
        // Not coercible to integer. Try converting to float to check bounds.
        let mut flimit = 0.0;
        if !tonumber(&limit_val, &mut flimit) {
            return Err(error_for_bad_limit(lua_state, &limit_val));
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
pub fn float_for_loop(lua_state: &mut LuaState, ra_pos: usize) -> bool {
    let stack = lua_state.stack_mut();
    let step = fltvalue(unsafe { stack.get_unchecked(ra_pos + 1) });
    let limit = fltvalue(unsafe { stack.get_unchecked(ra_pos) });
    let mut idx = fltvalue(unsafe { stack.get_unchecked(ra_pos + 2) });
    idx += step;
    if (step > 0.0 && idx <= limit) || (step <= 0.0 && idx >= limit) {
        chgfltvalue(unsafe { stack.get_unchecked_mut(ra_pos + 2) }, idx);
        return true;
    }

    false
}

#[cold]
#[inline(never)]
fn error_for_bad_limit(lua_state: &mut LuaState, limit_val: &LuaValue) -> LuaError {
    let t = crate::stdlib::debug::objtypename(lua_state, limit_val);
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
        Ok(None) => Err(crate::stdlib::debug::ordererror(lua_state, &va, &vb)),
        Err(LuaError::Yield) => {
            ci.call_status |= CIST_PENDING_FINISH;
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Cold path: binary metamethod fallback for MmBin/MmBinI/MmBinK
#[cold]
#[inline(never)]
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
            ci.set_pending_finish_get(result_reg as i32);
            ci.call_status |= CIST_PENDING_FINISH;
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
            ci.set_pending_finish_get(result_reg as i32);
            ci.call_status |= CIST_PENDING_FINISH;
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
    dest_reg: usize,
) -> LuaResult<()> {
    match finishget_to_reg_known_miss(lua_state, obj, key, dest_reg) {
        Ok(()) => Ok(()),
        Err(LuaError::Yield) => {
            ci.set_pending_finish_get(dest_reg as i32);
            ci.call_status |= CIST_PENDING_FINISH;
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Fast path for OOP-style `SELF_` lookups where the miss is resolved by a
/// table-only `__index` chain and the key is a short string.
/// Returns true if a value was found and written directly to `dest_reg`.
#[inline(never)]
pub fn self_shortstr_index_chain_fast(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    dest_reg: usize,
) -> bool {
    const TM_INDEX_BIT: u8 = TmKind::Index as u8;

    debug_assert!(key.is_short_string());

    let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
    let mut current = *obj;

    for _ in 0..MAXTAGLOOP {
        let Some(table) = current.as_table_mut() else {
            return false;
        };

        if let Some(value) = table.impl_table.get_shortstr_fast(key) {
            setobj2s(lua_state, dest_reg, &value);
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

#[inline(never)]
fn table_index_chain_fast_known_miss(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    dest_reg: usize,
) -> bool {
    const TM_INDEX_BIT: u8 = TmKind::Index as u8;

    let integer_key = key.ttisinteger().then(|| key.ivalue());
    if integer_key.is_none() && !key.is_short_string() {
        return false;
    }

    let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
    let mut current = *obj;
    let mut skip_lookup = true;

    for _ in 0..MAXTAGLOOP {
        let Some(table) = current.as_table_mut() else {
            return false;
        };

        if !skip_lookup {
            let value = if let Some(index) = integer_key {
                table.impl_table.fast_geti(index)
            } else {
                table.impl_table.get_shortstr_fast(key)
            };

            if let Some(value) = value {
                setobj2s(lua_state, dest_reg, &value);
                return true;
            }
        } else {
            skip_lookup = false;
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
    dest_reg: usize,
    skip_first_raw_lookup: bool,
) -> LuaResult<()> {
    const TM_INDEX_BIT: u8 = TmKind::Index as u8;

    let mut t = *obj;
    let mut skip_raw_lookup = skip_first_raw_lookup;

    for _ in 0..MAXTAGLOOP {
        let tm = if let Some(table) = t.as_table_mut() {
            if !skip_raw_lookup {
                if let Some(val) = table.raw_get(key) {
                    setobj2s(lua_state, dest_reg, &val);
                    return Ok(());
                }
            } else {
                skip_raw_lookup = false;
            }

            let meta = table.meta_ptr();
            if meta.is_null() {
                setobj2s(lua_state, dest_reg, &LuaValue::nil());
                return Ok(());
            }
            let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
            if mt.no_tm(TM_INDEX_BIT) {
                setobj2s(lua_state, dest_reg, &LuaValue::nil());
                return Ok(());
            }
            let vm = lua_state.vm_mut();
            let event_key = vm.const_strings.get_tm_value(TmKind::Index);
            match mt.impl_table.get_shortstr_fast(&event_key) {
                Some(v) => v,
                None => {
                    mt.set_tm_absent(TM_INDEX_BIT);
                    setobj2s(lua_state, dest_reg, &LuaValue::nil());
                    return Ok(());
                }
            }
        } else {
            if t.ttisfulluserdata()
                && let Some(ud) = t.as_userdata_mut()
                && let Some(key_str) = key.as_str()
                && let Some(udv) = ud.get_trait().get_field(key_str)
            {
                let result = crate::lua_value::udvalue_to_lua_value(lua_state, udv)?;
                setobj2s(lua_state, dest_reg, &result);
                return Ok(());
            }

            match get_metamethod_event(lua_state, &t, TmKind::Index) {
                Some(tm) => tm,
                None => {
                    return Err(crate::stdlib::debug::typeerror(lua_state, &t, "index"));
                }
            }
        };

        if tm.is_function() {
            return call_tm_res_into(lua_state, tm, t, *key, dest_reg);
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
                setobj2s(lua_state, dest_reg, &value);
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
    dest_reg: usize,
) -> LuaResult<()> {
    if table_index_chain_fast_known_miss(lua_state, obj, key, dest_reg) {
        return Ok(());
    }

    finishget_to_reg_inner(lua_state, obj, key, dest_reg, true)
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
) -> LuaResult<()> {
    match finishset(lua_state, obj, key, value) {
        Ok(_) => Ok(()),
        Err(LuaError::Yield) => {
            ci.set_pending_finish_get(-2);
            ci.call_status |= CIST_PENDING_FINISH;
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

#[inline(never)]
pub fn finishset_fallback_known_miss(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    obj: &LuaValue,
    key: &LuaValue,
    value: LuaValue,
) -> LuaResult<()> {
    match finishset_known_miss(lua_state, obj, key, value) {
        Ok(_) => Ok(()),
        Err(LuaError::Yield) => {
            ci.set_pending_finish_get(-2);
            ci.call_status |= CIST_PENDING_FINISH;
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
            ci.call_status |= CIST_PENDING_FINISH;
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Cold path: Return0 with active hooks — delegates to generic poscall
#[inline(never)]
pub fn return0_with_hook(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    a_pos: usize,
    pc: usize,
) -> LuaResult<()> {
    lua_state.set_top_raw(a_pos);
    ci.save_pc(pc);
    poscall(lua_state, ci, 0, pc)
}

/// Cold path: Return1 with active hooks — delegates to generic poscall
#[inline(never)]
pub fn return1_with_hook(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    a_pos: usize,
    pc: usize,
) -> LuaResult<()> {
    lua_state.set_top_raw(a_pos + 1);
    ci.save_pc(pc);
    poscall(lua_state, ci, 1, pc)
}
