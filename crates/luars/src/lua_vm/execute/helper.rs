use crate::{
    Chunk, LuaResult, LuaValue,
    lua_value::{LUA_VNUMFLT, LUA_VNUMINT},
    lua_vm::{
        LuaError, LuaState, TmKind,
        call_info::call_status::{CIST_RECST, CIST_YPCALL},
        execute,
    },
    stdlib::basic::parse_number::parse_lua_number,
};

/// Build hidden arguments for vararg functions
/// Port of ltm.c:245-270 buildhiddenargs
///
/// Initial stack:  func arg1 ... argn extra1 ...
///                 ^ ci->func                    ^ L->top
/// Final stack: func nil ... nil extra1 ... func arg1 ... argn
///                                          ^ ci->func
pub fn buildhiddenargs(
    lua_state: &mut LuaState,
    frame_idx: usize,
    chunk: &Chunk,
    totalargs: usize,
    nfixparams: usize,
    _nextra: usize,
) -> LuaResult<usize> {
    let call_info = lua_state.get_call_info(frame_idx);
    let old_base = call_info.base;
    let func_pos = if old_base > 0 { old_base - 1 } else { 0 };

    // The new function position is right after all the original arguments.
    // This way the extra (vararg) arguments are "hidden" between the old and new func positions.
    let new_func_pos = func_pos + totalargs + 1;
    let new_base = new_func_pos + 1;

    // Ensure enough stack space for new base + registers + EXTRA_STACK
    let new_needed_size = new_base + chunk.max_stack_size + 5; // +5 = EXTRA_STACK
    if new_needed_size > lua_state.stack_len() {
        lua_state.grow_stack(new_needed_size)?;
    }

    let stack = lua_state.stack_mut();

    // Step 1: Copy function to new_func_pos
    stack[new_func_pos] = stack[func_pos];

    // Step 2: Copy fixed parameters to after new function position
    for i in 0..nfixparams {
        stack[new_base + i] = stack[func_pos + 1 + i];
        // Erase original parameter with nil (for GC)
        unsafe {
            psetnilvalue(&mut stack[func_pos + 1 + i] as *mut LuaValue);
        }
    }

    // Step 3: Update ci->func.p and ci->top.p
    {
        let call_info = lua_state.get_call_info_mut(frame_idx);
        call_info.base = new_base;
        call_info.top = new_base + chunk.max_stack_size;
        call_info.func_offset = new_base - func_pos; // Distance from new_base to original func
    }

    // Update lua_state.top to match call_info.top
    let new_call_info_top = new_base + chunk.max_stack_size;
    lua_state.set_top(new_call_info_top)?;

    Ok(new_base)
}

// ============ Type tag检查宏 (对应 Lua 的 ttis* 宏) ============
// OPTIMIZED: 指针版本，避免引用/解引用开销

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

#[allow(unused)]
/// ttisstring - 检查是否是字符串
#[inline(always)]
pub unsafe fn pttisstring(v: *const LuaValue) -> bool {
    unsafe { (*v).is_string() }
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

/// setivalue - 设置整数值
/// OPTIMIZATION: Direct field access matching Lua 5.5's setivalue macro
#[inline(always)]
#[allow(unused)]
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

/// chgivalue - change integer value without writing tag (matching C Lua's chgivalue)
/// Only writes 8 bytes (Value) instead of 16 bytes (full LuaValue).
/// Caller MUST ensure tt is already LUA_VNUMINT.
#[inline(always)]
pub fn chgivalue(v: &mut LuaValue, i: i64) {
    v.value.i = i;
}

/// chgfltvalue - change float value without writing tag
/// Only writes 8 bytes. Caller MUST ensure tt is already LUA_VNUMFLT.
#[inline(always)]
pub fn chgfltvalue(v: &mut LuaValue, n: f64) {
    v.value.n = n;
}

/// setfltvalue - 设置浮点值  
/// OPTIMIZATION: Direct field access matching Lua 5.5's setfltvalue macro
#[allow(unused)]
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

/// setbfvalue - 设置false
#[inline(always)]
#[allow(unused)]
pub unsafe fn psetbfvalue(v: *mut LuaValue) {
    unsafe {
        *v = LuaValue::boolean(false);
    }
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
    if (a ^ b) < 0 && a % b != 0 { q.wrapping_sub(1) } else { q }
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
    if m != 0 && (m ^ b) < 0 { m.wrapping_add(b) } else { m }
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
            let f = pfltvalue(v);
            if f == (f as i64 as f64) && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
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

/// tonumberns - 尝试转换为浮点数 (不抛出错误)
/// Supports string-to-number coercion per Lua 5.5 spec
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
            // String coercion: try to convert string to number
            let val = &*v;
            if let Some(s) = val.as_str() {
                let parsed = parse_lua_number(s);
                if let Some(n) = parsed.as_number() {
                    *out = n;
                    return true;
                }
                if let Some(i) = parsed.as_integer() {
                    *out = i as f64;
                    return true;
                }
            }
            false
        }
    }
}

/// tonumberns_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn tonumberns(v: &LuaValue, out: &mut f64) -> bool {
    unsafe { ptonumberns(v as *const LuaValue, out as *mut f64) }
}

/// tonumber - 从LuaValue引用转换为浮点数 (用于常量)
/// Supports string-to-number coercion per Lua 5.5 spec
#[inline(always)]
pub fn tonumber(v: &LuaValue, out: &mut f64) -> bool {
    if v.tt() == LUA_VNUMFLT {
        unsafe {
            *out = v.value.n;
        }
        true
    } else if v.tt() == LUA_VNUMINT {
        unsafe {
            *out = v.value.i as f64;
        }
        true
    } else if let Some(s) = v.as_str() {
        let parsed = parse_lua_number(s);
        if let Some(n) = parsed.as_number() {
            *out = n;
            return true;
        }
        if let Some(i) = parsed.as_integer() {
            *out = i as f64;
            return true;
        }
        false
    } else {
        false
    }
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
        let f = unsafe { v.value.n };
        if f == (f as i64 as f64) && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            *out = f as i64;
            true
        } else {
            false
        }
    } else {
        false
    }
}

/// Lookup value from object's metatable __index
/// Returns Ok(Some(value)) if found, Ok(None) if not found in table chain,
/// or Err if attempting to index a non-table value without __index metamethod.
///
/// Optimized hot path: inline fasttm check for __index to avoid function call overhead.
/// Matches Lua 5.5's luaV_finishget pattern.
pub fn lookup_from_metatable(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
) -> LuaResult<Option<LuaValue>> {
    const MAXTAGLOOP: usize = 2000;
    const TM_INDEX_BIT: u8 = TmKind::Index as u8; // = 0

    let mut t = *obj;

    for _ in 0..MAXTAGLOOP {
        // Inline fasttm for __index on tables (hot path optimization)
        let tm = if let Some(table) = t.as_table_mut() {
            let meta = table.meta_ptr();
            if meta.is_null() {
                return Ok(None); // No metatable → no __index
            }
            let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
            // fasttm: check cached bit
            if mt.no_tm(TM_INDEX_BIT) {
                return Ok(None); // __index known absent
            }
            // Slow path: hash lookup
            let vm = lua_state.vm_mut();
            let event_key = vm.const_strings.get_tm_value(TmKind::Index);
            match mt.raw_get(&event_key) {
                Some(v) => v,
                None => {
                    mt.set_tm_absent(TM_INDEX_BIT); // Cache absence
                    return Ok(None);
                }
            }
        } else {
            // Non-table (string, userdata): fall back to general path
            match get_metamethod_event(lua_state, &t, TmKind::Index) {
                Some(tm) => tm,
                None => {
                    // No __index metamethod on non-table value → error
                    // Use typeerror for enhanced error message with varinfo
                    return Err(crate::stdlib::debug::typeerror(
                        lua_state,
                        &t.type_name(),
                        "index",
                    ));
                }
            }
        };

        // If __index is a function, call it using call_tm_res
        if tm.is_function() {
            let result = execute::metamethod::call_tm_res(lua_state, tm, t, *key)?;
            return Ok(Some(result));
        }

        // __index is a table, try to access tm[key] directly
        t = tm;

        if let Some(table) = t.as_table() {
            if let Some(value) = table.raw_get(key) {
                return Ok(Some(value));
            }
        }

        // If not found, loop again to check if tm has __index
    }

    // Too many iterations - possible loop
    Err(lua_state.error("'__index' chain too long; possible loop".to_string()))
}

/// Get a metamethod from a metatable value — implements Lua 5.5's fasttm/luaT_gettm pattern.
/// For TmKind <= Eq (first 6 metamethods), uses bit-flag cache to skip hash lookups
/// when the metamethod is known absent.
#[inline]
fn get_metamethod_from_metatable(
    lua_state: &mut LuaState,
    metatable: LuaValue,
    tm_kind: TmKind,
) -> Option<LuaValue> {
    if let Some(mt) = metatable.as_table_mut() {
        let tm_idx = tm_kind as u8;

        // fasttm: for cacheable TMs (Index..Eq), check bit-flag first
        if tm_idx <= TmKind::Eq as u8 {
            if mt.no_tm(tm_idx) {
                return None; // Known absent — skip hash lookup entirely
            }
        }

        let vm = lua_state.vm_mut();
        let event_key = vm.const_strings.get_tm_value(tm_kind);
        let result = mt.raw_get(&event_key);

        if result.is_none() && tm_idx <= TmKind::Eq as u8 {
            // Cache that this TM is absent (luaT_gettm pattern)
            mt.set_tm_absent(tm_idx);
        }

        return result;
    }

    None
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
pub fn finishset(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    value: LuaValue,
) -> LuaResult<bool> {
    const MAXTAGLOOP: usize = 2000;
    const TM_NEWINDEX_BIT: u8 = TmKind::NewIndex as u8; // = 1

    let mut t = *obj;

    for _ in 0..MAXTAGLOOP {
        // Check if t is a table — use inline fasttm for __newindex
        if let Some(table) = t.as_table_mut() {
            let meta = table.meta_ptr();
            let tm_val = if meta.is_null() {
                None
            } else {
                let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
                // fasttm: check cached bit for __newindex
                if mt.no_tm(TM_NEWINDEX_BIT) {
                    None
                } else {
                    let vm = lua_state.vm_mut();
                    let event_key = vm.const_strings.get_tm_value(TmKind::NewIndex);
                    let result = mt.raw_get(&event_key);
                    if result.is_none() {
                        mt.set_tm_absent(TM_NEWINDEX_BIT);
                    }
                    result
                }
            };

            if tm_val.is_none() {
                // No metamethod - set directly
                lua_state.raw_set(&t, *key, value);
                return Ok(true);
            }

            // Check if key already exists in the table.
            // If it does, do a raw set regardless of __newindex.
            if let Some(existing) = table.raw_get(key) {
                if !existing.is_nil() {
                    lua_state.raw_set(&t, *key, value);
                    return Ok(true);
                }
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
            // Not a table - get __newindex metamethod
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
            return Err(crate::stdlib::debug::typeerror(
                lua_state,
                &t.type_name(),
                "index",
            ));
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
    if let Some(mt) = get_metatable(lua_state, v1) {
        if let Some(mm) = get_metamethod_from_metatable(lua_state, mt, tm_kind) {
            return Some(mm);
        }
    }

    // Try v2's metatable
    if let Some(mt) = get_metatable(lua_state, v2) {
        if let Some(mm) = get_metamethod_from_metatable(lua_state, mt, tm_kind) {
            return Some(mm);
        }
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

// ============================================================
// Integer-Float comparison helpers (Lua 5.5 semantics)
// These handle the tricky edge cases where converting to f64
// loses precision (e.g., i64::MAX as f64 rounds to 2^63).
// ============================================================

/// Is integer i less than float f?  (i < f)
/// Handles NaN, infinities, and precision loss at i64 boundaries.
#[inline]
pub fn int_lt_float(i: i64, f: f64) -> bool {
    if f.is_nan() {
        return false;
    }
    // i64::MAX as f64 = 2^63 (rounds up), so f >= 2^63 means f > any i64
    if f >= (i64::MAX as f64) {
        return true;
    }
    // i64::MIN as f64 = -2^63 (exact), so f < -2^63 means f < any i64
    if f < (i64::MIN as f64) {
        return false;
    }
    // f is in castable range: truncate toward zero
    let fi = f as i64;
    if i < fi {
        true
    } else if i > fi {
        false
    } else {
        // i == fi: true iff f has a positive fractional part beyond fi
        f > (fi as f64)
    }
}

/// Is float f less than integer i?  (f < i)
#[inline]
pub fn float_lt_int(f: f64, i: i64) -> bool {
    if f.is_nan() {
        return false;
    }
    if f >= (i64::MAX as f64) {
        return false;
    }
    if f < (i64::MIN as f64) {
        return true;
    }
    let fi = f as i64;
    if fi < i {
        true
    } else if fi > i {
        false
    } else {
        // fi == i: true iff f has a negative fractional part (truncated away)
        f < (fi as f64)
    }
}

/// Is integer i less than or equal to float f?  (i <= f)
#[inline]
pub fn int_le_float(i: i64, f: f64) -> bool {
    // NaN: i <= NaN is always false
    if f.is_nan() {
        return false;
    }
    !float_lt_int(f, i)
}

/// Is float f less than or equal to integer i?  (f <= i)
#[inline]
pub fn float_le_int(f: f64, i: i64) -> bool {
    // NaN: NaN <= i is always false
    if f.is_nan() {
        return false;
    }
    !int_lt_float(i, f)
}

/// Finish a C frame left on the call stack after yield-resume.
/// This is the Rust equivalent of Lua 5.5's finishCcall.
#[cold]
#[inline(never)]
fn finish_c_frame(lua_state: &mut LuaState, frame_idx: usize) -> LuaResult<()> {
    let ci = lua_state.get_call_info(frame_idx);
    let pcall_func_pos = ci.base - ci.func_offset;
    let nresults = ci.nresults;
    let has_recst = ci.call_status & CIST_RECST != 0;

    if ci.call_status & CIST_YPCALL != 0 {
        if has_recst {
            // Error recovery completed (or continuing) after yield.
            // Retrieve the saved error value and try to close remaining TBC entries.
            let error_val = std::mem::replace(&mut lua_state.error_object, LuaValue::nil());
            lua_state.clear_error();
            let close_level = pcall_func_pos + 1; // body's base position

            // Try to close remaining TBC entries
            let close_result = lua_state.close_tbc_with_error(close_level, error_val.clone());

            match close_result {
                Ok(()) => {
                    // All TBC entries closed. Set up (false, error) result.
                    let final_err = std::mem::replace(&mut lua_state.error_object, LuaValue::nil());
                    let result_err = if !final_err.is_nil() {
                        final_err
                    } else {
                        error_val
                    };
                    lua_state.clear_error();

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
                            let ci_top = lua_state.get_call_info(ci_idx).top;
                            if ci_top < new_top {
                                lua_state.get_call_info_mut(ci_idx).top = new_top;
                            }
                        } else {
                            let frame_top = lua_state.get_call_info(ci_idx).top;
                            lua_state.set_top_raw(frame_top);
                        }
                    }

                    Ok(())
                }
                Err(LuaError::Yield) => {
                    // Another TBC close method yielded. Save cascaded error and yield.
                    let cascaded = std::mem::replace(&mut lua_state.error_object, LuaValue::nil());
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
            let body_nres = if stack_top > body_results_start {
                stack_top - body_results_start
            } else {
                0
            };

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
                    let ci_top = lua_state.get_call_info(ci_idx).top;
                    if ci_top < new_top {
                        lua_state.get_call_info_mut(ci_idx).top = new_top;
                    }
                } else {
                    let frame_top = lua_state.get_call_info(ci_idx).top;
                    lua_state.set_top_raw(frame_top);
                }
            }

            Ok(())
        }
    } else {
        // Generic C frame after yield — just pop it.
        // This shouldn't normally happen, but be safe.
        lua_state.pop_frame();
        Ok(())
    }
}

/// Handle pending metamethod finish (cold path, extracted from main loop).
/// Returns true if a C frame was finished and execution should restart.
#[cold]
#[inline(never)]
pub fn handle_pending_ops(lua_state: &mut LuaState, frame_idx: usize) -> LuaResult<bool> {
    use crate::lua_vm::call_info::call_status::{CIST_C, CIST_PENDING_FINISH};

    let ci = lua_state.get_call_info(frame_idx);
    if ci.call_status & CIST_C != 0 {
        finish_c_frame(lua_state, frame_idx)?;
        return Ok(true); // restart startfunc
    }
    // === luaV_finishOp equivalent ===
    let ci = lua_state.get_call_info(frame_idx);
    let pending = ci.pending_finish_get;
    let base_tmp = ci.base;
    if pending >= 0 {
        let dest = base_tmp + pending as usize;
        let top = lua_state.get_top();
        if top > 0 {
            let result = lua_state.stack_mut()[top - 1];
            lua_state.stack_mut()[dest] = result;
        }
        let ci_top = lua_state.get_call_info(frame_idx).top;
        lua_state.set_top_raw(ci_top);
    } else {
        let ci_top = lua_state.get_call_info(frame_idx).top;
        lua_state.set_top_raw(ci_top);
    }
    let ci_mut = lua_state.get_call_info_mut(frame_idx);
    ci_mut.pending_finish_get = -1;
    ci_mut.call_status &= !CIST_PENDING_FINISH;
    Ok(false) // continue to hot path
}
