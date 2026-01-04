use crate::{
    Chunk, LuaResult, LuaValue,
    lua_value::{LUA_VNUMFLT, LUA_VNUMINT},
    lua_vm::LuaState,
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
        setnilvalue(&mut stack[func_pos + 1 + i]);
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

/// ttisinteger - 检查是否是整数 (最快的类型检查)
#[inline(always)]
pub fn ttisinteger(v: &LuaValue) -> bool {
    (*v).tt_ == LUA_VNUMINT
}

/// ttisfloat - 检查是否是浮点数
#[inline(always)]
pub fn ttisfloat(v: &LuaValue) -> bool {
    (*v).tt_ == LUA_VNUMFLT
}

/// ttisnumber - 检查是否是任意数字 (整数或浮点)
#[inline(always)]
pub fn ttisnumber(v: &LuaValue) -> bool {
    (*v).tt_ == LUA_VNUMINT || (*v).tt_ == LUA_VNUMFLT
}

// ============ 值访问宏 (对应 Lua 的 ivalue/fltvalue) ============

/// ivalue - 直接获取整数值 (调用前必须用 ttisinteger 检查)
#[inline(always)]
pub fn ivalue(v: &LuaValue) -> i64 {
    unsafe { (*v).value_.i }
}

/// fltvalue - 直接获取浮点值 (调用前必须用 ttisfloat 检查)
#[inline(always)]
pub fn fltvalue(v: &LuaValue) -> f64 {
    unsafe { (*v).value_.n }
}

/// setivalue - 设置整数值
#[inline(always)]
pub fn setivalue(v: &mut LuaValue, i: i64) {
    (*v).value_.i = i;
    (*v).tt_ = LUA_VNUMINT;
}

/// chgivalue - 只修改整数值，不修改类型标签（Lua的chgivalue宏）
/// 调用前必须确认类型已经是整数！
#[inline(always)]
pub fn chgivalue(v: &mut LuaValue, i: i64) {
    (*v).value_.i = i;
}

/// setfltvalue - 设置浮点值
#[inline(always)]
pub fn setfltvalue(v: &mut LuaValue, n: f64) {
    (*v).value_.n = n;
    (*v).tt_ = LUA_VNUMFLT;
}

/// chgfltvalue - 只修改浮点值，不修改类型标签
/// 调用前必须确认类型已经是浮点！
#[inline(always)]
pub fn chgfltvalue(v: &mut LuaValue, n: f64) {
    (*v).value_.n = n;
}

/// setbfvalue - 设置false
#[inline(always)]
pub fn setbfvalue(v: &mut LuaValue) {
    (*v) = LuaValue::boolean(false);
}

/// setbtvalue - 设置true
#[inline(always)]
pub fn setbtvalue(v: &mut LuaValue) {
    (*v) = LuaValue::boolean(true);
}

/// setnilvalue - 设置nil
#[inline(always)]
pub fn setnilvalue(v: &mut LuaValue) {
    *v = LuaValue::nil();
}

// ============ 类型转换辅助函数 ============

/// tointegerns - 尝试转换为整数 (不抛出错误)
/// 对应 Lua 的 tointegerns 宏
#[inline(always)]
pub fn tointegerns(v: &LuaValue, out: &mut i64) -> bool {
    if ttisinteger(v) {
        *out = ivalue(v);
        true
    } else {
        false
    }
}

/// tonumberns - 尝试转换为浮点数 (不抛出错误)
#[inline(always)]
pub fn tonumberns(v: &LuaValue, out: &mut f64) -> bool {
    if ttisfloat(v) {
        *out = fltvalue(v);
        true
    } else if ttisinteger(v) {
        *out = ivalue(v) as f64;
        true
    } else {
        false
    }
}

/// tonumber - 从LuaValue引用转换为浮点数 (用于常量)
#[inline(always)]
pub fn tonumber(v: &LuaValue, out: &mut f64) -> bool {
    if v.tt_ == LUA_VNUMFLT {
        unsafe {
            *out = v.value_.n;
        }
        true
    } else if v.tt_ == LUA_VNUMINT {
        unsafe {
            *out = v.value_.i as f64;
        }
        true
    } else {
        false
    }
}

/// tointeger - 从LuaValue引用获取整数 (用于常量)
#[inline(always)]
pub fn tointeger(v: &LuaValue, out: &mut i64) -> bool {
    if v.tt_ == LUA_VNUMINT {
        unsafe {
            *out = v.value_.i;
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
    // For string: use string_mt
    if obj.is_string() {
        let mt_val = lua_state.vm_mut().string_mt?;
        return lookup_index_from_metatable_value(lua_state, mt_val, key);
    }
    
    // For userdata: use userdata's metatable
    if let Some(ud_id) = obj.as_userdata_id() {
        let mt_val = lua_state.vm_mut().object_pool.get_userdata(ud_id)?.get_metatable();
        return lookup_index_from_metatable_value(lua_state, mt_val, key);
    }
    
    // For table: check if it has metatable
    if let Some(table_id) = obj.as_table_id() {
        let mt_val = lua_state.vm_mut().object_pool.get_table(table_id)?.get_metatable()?;
        return lookup_index_from_metatable_value(lua_state, mt_val, key);
    }
    
    None
}

/// Helper: lookup from metatable's __index field
fn lookup_index_from_metatable_value(
    lua_state: &mut LuaState,
    mt_val: LuaValue,
    key: &LuaValue,
) -> Option<LuaValue> {
    let mt_table_id = mt_val.as_table_id()?;
    let vm = lua_state.vm_mut();
    let index_key = vm.create_string("__index");
    let mt = vm.object_pool.get_table(mt_table_id)?;
    let index_value = mt.raw_get(&index_key)?;
    let index_table_id = index_value.as_table_id()?;
    let index_table = vm.object_pool.get_table(index_table_id)?;
    index_table.raw_get(key)
}
