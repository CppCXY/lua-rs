use crate::{
    lua_value::{LUA_VNUMFLT, LUA_VNUMINT}, lua_vm::{execute, LuaState, TmKind}, Chunk, LuaResult, LuaValue
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
    let stack_top = lua_state.get_top();

    let stack = lua_state.stack_mut();
    let mut top = stack_top;

    // Step 1: Copy function to top (after all arguments)
    // setobjs2s(L, L->top.p++, ci->func.p);
    let func_src = stack[func_pos];
    stack[top] = func_src;
    top += 1;

    // Step 2: Copy fixed parameters to after copied function
    // for (i = 1; i <= nfixparams; i++)
    for i in 0..nfixparams {
        let src = stack[func_pos + 1 + i];
        stack[top] = src;
        top += 1;
        // Erase original parameter with nil (for GC)
        unsafe {
            psetnilvalue(&mut stack[func_pos + 1 + i] as *mut LuaValue);
        }
    }

    // Step 3: Update ci->func.p and ci->top.p
    // ci->func.p += totalargs + 1;
    // ci->top.p += totalargs + 1;
    let new_func_pos = func_pos + totalargs + 1;
    let new_base = new_func_pos + 1;

    let new_call_info_top = {
        let call_info = lua_state.get_call_info_mut(frame_idx);
        call_info.base = new_base;
        call_info.top += totalargs + 1;
        call_info.func_offset = new_base - func_pos; // Distance from new_base to original func
        call_info.top
    };

    // Ensure enough stack space for new base + registers
    let new_needed_size = new_base + chunk.max_stack_size;
    if new_needed_size > lua_state.stack_len() {
        lua_state.grow_stack(new_needed_size - lua_state.stack_len())?;
    }

    // Update lua_state.top to match call_info.top
    // This ensures that subsequent set_top calls preserve our data
    lua_state.set_top(new_call_info_top);

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

/// setbtvalue - 设置true
#[inline(always)]
#[allow(unused)]
pub unsafe fn psetbtvalue(v: *mut LuaValue) {
    unsafe {
        *v = LuaValue::boolean(true);
    }
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

// ============ 类型转换辅助函数 ============

/// tointegerns - 尝试转换为整数 (不抛出错误)
/// 对应 Lua 的 tointegerns 宏
#[inline(always)]
pub unsafe fn ptointegerns(v: *const LuaValue, out: *mut i64) -> bool {
    unsafe {
        if pttisinteger(v) {
            *out = pivalue(v);
            true
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

/// tonumberns_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn tonumberns(v: &LuaValue, out: &mut f64) -> bool {
    unsafe { ptonumberns(v as *const LuaValue, out as *mut f64) }
}

/// tonumber - 从LuaValue引用转换为浮点数 (用于常量)
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
    } else {
        false
    }
}

/// Lookup value from object's metatable __index
/// Returns Some(value) if found, None if not found or no metatable
pub fn lookup_from_metatable(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
) -> Option<LuaValue> {
    // Port of luaV_finishget from lvm.c:291
    const MAXTAGLOOP: usize = 2000;

    let mut t = *obj;

    for _ in 0..MAXTAGLOOP {
        // Get __index metamethod
        let tm = get_metamethod_event(lua_state, &t, TmKind::Index)?;

        // If __index is a function, call it using call_tm_res
        if tm.is_function() {
            // Use call_tm_res which correctly handles stack and Protect pattern
            match execute::metamethod::call_tm_res(lua_state, tm, t, *key) {
                Ok(result) => return Some(result),
                Err(_) => return None,
            }
        }

        // __index is a table, try to access tm[key]
        t = tm;

        // Try direct table access first (fast path)
        if let Some(table) = t.as_table() {
            if let Some(value) = table.raw_get(key) {
                return Some(value);
            }
        }

        // If not found, loop again to check if tm has __index
    }

    // Too many iterations - possible loop
    None
}

/// Get a metamethod from a metatable value
fn get_metamethod_from_metatable(
    lua_state: &mut LuaState,
    metatable: LuaValue,
    tm_kind: TmKind,
) -> Option<LuaValue> {
    // OLD optimization: resolve key once
    // const KEY_CACHE: &str = event;
    // let event_key = vm.object_pool.get_tm_value_by_str(event);

    // NEW Optimization: Use direct pointer access if possible
    if let Some(mt) = metatable.as_table() {
        let vm = lua_state.vm_mut();
        let event_key = vm.object_pool.get_tm_value(tm_kind);

        return mt.raw_get(&event_key);
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

    let mut t = *obj;

    for _ in 0..MAXTAGLOOP {
        // Check if t is a table
        if let Some(table) = t.as_table() {
            // Get metatable
            let metatable = table.get_metatable();
            
            // Try to get __newindex metamethod
            let tm_val = if let Some(metatable) = metatable {
                get_metamethod_from_metatable(lua_state, metatable, TmKind::NewIndex)
            } else {
                None
            };

            if tm_val.is_none() {
                // No metamethod - set directly
                if let Some(table_ref) = t.as_table_mut() {
                    table_ref.raw_set(key, value);
                    
                    // CRITICAL: GC write barrier
                    let table_ptr = t.as_table_ptr().unwrap();
                    lua_state.gc_barrier_back(table_ptr.into());
                    
                    return Ok(true);
                }
            }
            
            // Found metamethod - check if it's a function
            if let Some(tm) = tm_val {
                if tm.is_function() {
                    // Call metamethod: luaT_callTM(L, tm, t, key, val)
                    use crate::lua_vm::execute;

                    // Call luaT_callTM with 3 arguments, no return value
                    execute::metamethod::call_tm(lua_state, tm, t, *key, value)?;

                    return Ok(true);
                }
                
                // Metamethod is a table - repeat assignment over 'tm'
                // t = tm; then try luaV_fastset again
                t = tm;
                // Continue loop to try setting t[key] = value
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
            return Err(lua_state.error(format!("attempt to index a {} value", t.type_name())));
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
    if value.is_string() {
        return lua_state.vm_mut().string_mt;
    } else if let Some(table) = value.as_table_mut() {
        return table.get_metatable();
    } else if let Some(ud) = value.as_userdata_mut() {
        return ud.get_metatable();
    }

    None
}
