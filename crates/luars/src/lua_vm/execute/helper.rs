use crate::{
    Chunk, LuaResult, LuaValue,
    lua_value::{LUA_VNUMFLT, LUA_VNUMINT},
    lua_vm::{LuaState, execute},
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
pub unsafe fn psetivalue(v: *mut LuaValue, i: i64) {
    unsafe { *v = LuaValue::integer(i); }
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
    unsafe { *v = LuaValue::float(n); }
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
    unsafe { *v = LuaValue::boolean(false); }
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
    unsafe { *v = LuaValue::boolean(true); }
}

/// setbtvalue_ref - 引用版本（保留兼容性）
#[inline(always)]
pub fn setbtvalue(v: &mut LuaValue) {
    *v = LuaValue::boolean(true);
}

/// setnilvalue - 设置nil
#[inline(always)]
pub unsafe fn psetnilvalue(v: *mut LuaValue) {
    unsafe { *v = LuaValue::nil(); }
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
        let tm = get_index_metamethod(lua_state, &t)?;

        // If __index is a function, call it
        if tm.is_function() {
            // Call metamethod: tm(t, key) -> result
            // Similar to call_metamethod in metamethod.rs

            // CRITICAL: Save current stack_top to restore later
            let saved_top = lua_state.get_top();
            let func_pos = saved_top;

            // Ensure stack has enough space for function call
            let needed_size = func_pos + 3;
            if let Err(_) = lua_state.grow_stack(needed_size) {
                return None;
            }

            // Push function and arguments onto stack
            {
                let stack = lua_state.stack_mut();
                stack[func_pos] = tm;
                stack[func_pos + 1] = t;
                stack[func_pos + 2] = *key;
            }
            lua_state.set_top(func_pos + 3);

            let caller_depth = lua_state.call_depth();
            let new_base = func_pos + 1;

            // Push frame and execute
            match lua_state.push_frame(tm, new_base, 2, 1) {
                Ok(_) => {}
                Err(_) => {
                    // Restore stack_top on error
                    lua_state.set_top(saved_top);
                    return None;
                }
            }

            match crate::lua_vm::execute::lua_execute_until(lua_state, caller_depth) {
                Ok(_) => {}
                Err(_) => {
                    // Restore stack_top on error
                    lua_state.set_top(saved_top);
                    return None;
                }
            }

            // Get result from func_pos (where function was)
            let result = {
                let stack = lua_state.stack_mut();
                stack[func_pos]
            };

            // CRITICAL: Restore saved stack_top, not func_pos
            // This ensures caller's frame.top is not corrupted
            lua_state.set_top(saved_top);

            return Some(result);
        }

        // __index is a table, try to access tm[key]
        t = tm;

        // Try direct table access first (fast path)
        if let Some(table_id) = t.as_table_id() {
            if let Some(table) = lua_state.vm_mut().object_pool.get_table(table_id) {
                if let Some(value) = table.raw_get(key) {
                    return Some(value);
                }
            }
        }

        // If not found, loop again to check if tm has __index
    }

    // Too many iterations - possible loop
    None
}

/// Get __len metamethod for a value
/// Similar to get_index_metamethod but for __len
pub fn get_len_metamethod(lua_state: &mut LuaState, obj: &LuaValue) -> Option<LuaValue> {
    // For string: use string_mt
    if obj.is_string() {
        let mt_val = lua_state.vm_mut().string_mt?;
        return get_metamethod_from_metatable(lua_state, mt_val, "__len");
    }
    // For userdata: use userdata's metatable
    else if let Some(ud) = obj.as_userdata_mut() {
        let mt_val = ud.get_metatable();
        return get_metamethod_from_metatable(lua_state, mt_val, "__len");
    }
    // For table: check if it has metatable
    else if let Some(table) = obj.as_table_mut() {
        let mt_id = table.get_metatable()?;
        let mt_val = lua_state.vm_mut().object_pool.get_table_value(mt_id)?;
        return get_metamethod_from_metatable(lua_state, mt_val, "__len");
    }

    None
}

/// Get __index metamethod for a value
fn get_index_metamethod(lua_state: &mut LuaState, obj: &LuaValue) -> Option<LuaValue> {
    get_metamethod_event(lua_state, obj, "__index")
}

/// Get a metamethod from a metatable value
fn get_metamethod_from_metatable(
    lua_state: &mut LuaState,
    mt_val: LuaValue,
    event: &str,
) -> Option<LuaValue> {
    let mt_table_id = mt_val.as_table_id()?;
    let vm = lua_state.vm_mut();
    let event_key = vm.object_pool.get_tm_value_by_str(event);
    let mt = vm.object_pool.get_table(mt_table_id)?;
    mt.raw_get(&event_key)
}

/// Store a value using __newindex metamethod if key doesn't exist
/// Port of luaV_finishset from lvm.c:332
pub fn store_to_metatable(
    lua_state: &mut LuaState,
    obj: &LuaValue,
    key: &LuaValue,
    value: LuaValue,
) -> LuaResult<bool> {
    const MAXTAGLOOP: usize = 2000;

    let mut t = *obj;

    for _ in 0..MAXTAGLOOP {
        // Check if t is a table
        if let Some(table_id) = t.as_table_id() {
            // Check if key exists and get metatable in one go
            let (key_exists, mt_val) = {
                let vm = lua_state.vm_mut();
                let table_opt = vm.object_pool.get_table(table_id);

                if let Some(tbl) = table_opt {
                    let ke = tbl.raw_get(key).is_some();
                    let mt_id = tbl.get_metatable();
                    let mt = if let Some(mid) = mt_id {
                        vm.object_pool.get_table_value(mid)
                    } else {
                        None
                    };
                    (ke, mt)
                } else {
                    return Err(lua_state.error(format!(
                        "Cannot get table reference for table_id {:?}",
                        table_id
                    )));
                }
            };

            if key_exists {
                // Key exists, do direct assignment (no __newindex)
                let table_result = lua_state.vm_mut().object_pool.get_table_mut(table_id);

                if table_result.is_none() {
                    return Err(lua_state.error(format!(
                        "Cannot get mutable table for existing key, table_id {:?}",
                        table_id
                    )));
                }

                table_result.unwrap().raw_set(key, value);
                lua_state.vm_mut().check_gc();
                return Ok(true);
            }

            // Key doesn't exist, check for __newindex metamethod
            if let Some(mt) = mt_val {
                if let Some(tm) = get_metamethod_from_metatable(lua_state, mt, "__newindex") {
                    // Has __newindex metamethod
                    if tm.is_function() {
                        // **CRITICAL**: Like Lua 5.5's Protect macro, set stack_top to caller frame's top
                        // before pushing arguments. This prevents overwriting active registers.
                        let caller_frame_idx = lua_state.call_depth() - 1;
                        let caller_frame_top = lua_state.get_call_info(caller_frame_idx).top;
                        lua_state.set_top(caller_frame_top);

                        // Call metamethod: tm(t, key, value)
                        let func_pos = lua_state.get_top();

                        // Push function and arguments using push_value to ensure stack grows
                        // CRITICAL: Push the ORIGINAL table (*obj), not the loop variable t
                        lua_state.push_value(tm)?;
                        lua_state.push_value(*obj)?;
                        lua_state.push_value(*key)?;
                        lua_state.push_value(value)?;

                        let caller_depth = lua_state.call_depth();
                        let new_base = func_pos + 1;
                        // Call metamethod (0 results expected)
                        lua_state.push_frame(tm, new_base, 3, 0)?;
                        execute::lua_execute_until(lua_state, caller_depth)?;

                        lua_state.set_top(func_pos); // Clean up stack
                        return Ok(true);
                    }

                    // __newindex is a table, repeat assignment over it
                    t = tm;
                    continue;
                }
            }

            // No __newindex metamethod, do direct assignment
            let table_result = lua_state.vm_mut().object_pool.get_table_mut(table_id);

            if table_result.is_none() {
                return Err(lua_state.error(format!(
                    "Cannot get mutable table reference for table_id {:?}",
                    table_id
                )));
            }

            table_result.unwrap().raw_set(key, value);
            // GC write barrier: check GC after table modification
            lua_state.vm_mut().check_gc();
            return Ok(true);
        }

        // Not a table, get __newindex metamethod
        let tm = get_newindex_metamethod(lua_state, &t);
        if tm.is_none() {
            return Err(lua_state.error(format!("attempt to index a {} value", t.type_name())));
        }

        let tm = tm.unwrap();

        // If __newindex is a function, call it
        if tm.is_function() {
            let func_pos = lua_state.get_top();

            // Push function and arguments: tm(t, key, value)
            {
                let stack = lua_state.stack_mut();
                stack[func_pos] = tm;
                stack[func_pos + 1] = t;
                stack[func_pos + 2] = *key;
                stack[func_pos + 3] = value;
            }
            lua_state.set_top(func_pos + 4);

            let caller_depth = lua_state.call_depth();
            let new_base = func_pos + 1;

            // Call metamethod (0 results expected)
            lua_state.push_frame(tm, new_base, 3, 0)?;
            crate::lua_vm::execute::lua_execute_until(lua_state, caller_depth)?;

            lua_state.set_top(func_pos); // Clean up stack
            return Ok(true);
        }

        // __newindex is a table, repeat assignment over it
        t = tm;
    }

    // Too many iterations - possible loop
    Err(lua_state.error("'__newindex' chain too long; possible loop".to_string()))
}

/// Get __newindex metamethod for a value
fn get_newindex_metamethod(lua_state: &mut LuaState, obj: &LuaValue) -> Option<LuaValue> {
    get_metamethod_event(lua_state, obj, "__newindex")
}

/// Get __call metamethod for a value
pub fn get_call_metamethod(lua_state: &mut LuaState, value: &LuaValue) -> Option<LuaValue> {
    get_metamethod_event(lua_state, value, "__call")
}

pub fn get_metamethod_event(
    lua_state: &mut LuaState,
    value: &LuaValue,
    event: &str,
) -> Option<LuaValue> {
    let mt = get_metatable(lua_state, value)?;
    get_metamethod_from_metatable(lua_state, mt, event)
}

/// Get binary operation metamethod from either of two values
/// Checks v1's metatable first, then v2's if not found
pub fn get_binop_metamethod(
    lua_state: &mut LuaState,
    v1: &LuaValue,
    v2: &LuaValue,
    event: &str,
) -> Option<LuaValue> {
    // Try v1's metatable first
    if let Some(mt) = get_metatable(lua_state, v1) {
        if let Some(mm) = get_metamethod_from_metatable(lua_state, mt, event) {
            return Some(mm);
        }
    }

    // Try v2's metatable
    if let Some(mt) = get_metatable(lua_state, v2) {
        if let Some(mm) = get_metamethod_from_metatable(lua_state, mt, event) {
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
        let mt_id = table.get_metatable();
        return lua_state.vm_mut().object_pool.get_table_value(mt_id?);
    } else if let Some(ud) = value.as_userdata_mut() {
        return Some(ud.get_metatable());
    }

    None
}
