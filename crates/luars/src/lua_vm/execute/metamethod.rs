use crate::gc::TablePtr;
use crate::lua_value::{LuaValue, lua_value_to_udvalue, udvalue_to_lua_value};
use crate::lua_vm::call_info::CallInfo;
use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
use crate::lua_vm::execute::call::{self, call_c_function};
use crate::lua_vm::execute::core::lua_execute;
use crate::lua_vm::execute::helper::{get_binop_metamethod, get_metamethod_from_meta_ptr};
/// Metamethod operations
///
/// Implements MMBIN, MMBINI, MMBINK opcodes
/// Based on Lua 5.5 ltm.c
use crate::lua_vm::{LuaError, LuaResult, LuaState, StkId, get_metamethod_event};
use crate::stdlib::debug;

/// Try unary metamethod (for __unm, __bnot)
/// Port of luaT_trybinTM for unary operations
pub fn try_unary_tm(
    lua_state: &mut LuaState,
    operand: LuaValue,
    result_pos: usize,
    tm_kind: TmKind,
) -> LuaResult<()> {
    // Try trait-based __unm for userdata
    if tm_kind == TmKind::Unm
        && operand.ttisfulluserdata()
        && let Some(ud) = operand.as_userdata_mut()
    {
        let trait_obj = ud.get_trait()?;
        if let Some(udv) = trait_obj.lua_unm() {
            let result = udvalue_to_lua_value(lua_state, udv)?;
            let stack = lua_state.stack_mut();
            stack[result_pos] = result;
            return Ok(());
        }
    }
    // Try trait-based __bnot for userdata
    if tm_kind == TmKind::Bnot
        && operand.ttisfulluserdata()
        && let Some(ud) = operand.as_userdata_mut()
    {
        let trait_obj = ud.get_trait()?;
        if let Some(udv) = trait_obj.lua_bnot() {
            let result = udvalue_to_lua_value(lua_state, udv)?;
            let stack = lua_state.stack_mut();
            stack[result_pos] = result;
            return Ok(());
        }
    }

    // Try to get metamethod from operand
    let metamethod = get_metamethod_event(lua_state, &operand, tm_kind);
    if let Some(mm) = metamethod {
        // Call metamethod: mm(operand, operand) -> result
        let result = call_tm_res(lua_state, mm, operand, operand)?;

        // Store result
        let stack = lua_state.stack_mut();
        stack[result_pos] = result;
        Ok(())
    } else {
        // No metamethod found
        if tm_kind == TmKind::Bnot && operand.is_number() {
            // Float that can't be converted to integer
            Err(lua_state.error("number has no integer representation".to_string()))
        } else {
            // Use descriptive operation name like C Lua
            let op_desc = match tm_kind {
                TmKind::Bnot => "perform bitwise operation on",
                TmKind::Unm => "perform arithmetic on",
                TmKind::Len => "get length of",
                _ => "perform arithmetic on",
            };
            Err(crate::stdlib::debug::typeerror(
                lua_state, &operand, op_desc,
            ))
        }
    }
}

/// Try binary metamethod
/// Corresponds to luaT_trybinTM in ltm.c
/// Like Lua 5.5's luaT_trybinTM:
/// ```c
/// void luaT_trybinTM (lua_State *L, const TValue *p1, const TValue *p2,
///                     StkId res, TMS event) {
///   if (l_unlikely(callbinTM(L, p1, p2, res, event) < 0)) {
///     switch (event) {
///       case TM_BAND: case TM_BOR: case TM_BXOR:
///       case TM_SHL: case TM_SHR: case TM_BNOT: {
///         if (ttisnumber(p1) && ttisnumber(p2))
///           luaG_tointerror(L, p1, p2);
///         else
///           luaG_opinterror(L, p1, p2, "perform bitwise operation on");
///       }
///       /* calls never return, but to avoid warnings: *//* FALLTHROUGH */
///       default:
///         luaG_opinterror(L, p1, p2, "perform arithmetic on");
///     }
///   }
/// }
/// ```
/// Try to convert a LuaValue to integer (NO string coercion).
/// Returns Some(i64) if the value is an integer or an integral float.
pub fn try_bin_tm(
    lua_state: &mut LuaState,
    p1: LuaValue,
    p2: LuaValue,
    res: u32,
    p1_reg: u32,
    p2_reg: u32,
    tm_kind: TmKind,
) -> LuaResult<()> {
    // Try trait-based arithmetic for userdata
    if p1.ttisfulluserdata() || p2.ttisfulluserdata() {
        let trait_result = if let Some(ud) = p1.as_userdata_mut() {
            let trait_obj = match ud.get_trait() {
                Ok(t) => t,
                Err(_) => {
                    return Ok(()); /* expired */
                }
            };
            let other = lua_value_to_udvalue(&p2);
            Some(match tm_kind {
                TmKind::Add => trait_obj.lua_add(&other),
                TmKind::Sub => trait_obj.lua_sub(&other),
                TmKind::Mul => trait_obj.lua_mul(&other),
                TmKind::Div => trait_obj.lua_div(&other),
                TmKind::Mod => trait_obj.lua_mod(&other),
                TmKind::Pow => trait_obj.lua_pow(&other),
                TmKind::IDiv => trait_obj.lua_idiv(&other),
                TmKind::Band => trait_obj.lua_band(&other),
                TmKind::Bor => trait_obj.lua_bor(&other),
                TmKind::Bxor => trait_obj.lua_bxor(&other),
                TmKind::Shl => trait_obj.lua_shl(&other),
                TmKind::Shr => trait_obj.lua_shr(&other),
                _ => None,
            })
        } else {
            None
        };
        if let Some(Some(udv)) = trait_result {
            lua_state.stack_mut()[res as usize] = udvalue_to_lua_value(lua_state, udv)?;
            return Ok(());
        }
        let trait_result2 = if let Some(ud) = p2.as_userdata_mut() {
            let trait_obj = match ud.get_trait() {
                Ok(t) => t,
                Err(_) => {
                    return Ok(());
                }
            };
            let other = lua_value_to_udvalue(&p1);
            Some(match tm_kind {
                TmKind::Add => trait_obj.lua_add(&other),
                TmKind::Sub => trait_obj.lua_sub(&other),
                TmKind::Mul => trait_obj.lua_mul(&other),
                TmKind::Div => trait_obj.lua_div(&other),
                TmKind::Mod => trait_obj.lua_mod(&other),
                TmKind::Pow => trait_obj.lua_pow(&other),
                TmKind::IDiv => trait_obj.lua_idiv(&other),
                TmKind::Band => trait_obj.lua_band(&other),
                TmKind::Bor => trait_obj.lua_bor(&other),
                TmKind::Bxor => trait_obj.lua_bxor(&other),
                TmKind::Shl => trait_obj.lua_shl(&other),
                TmKind::Shr => trait_obj.lua_shr(&other),
                _ => None,
            })
        } else {
            None
        };
        if let Some(Some(udv)) = trait_result2 {
            lua_state.stack_mut()[res as usize] = udvalue_to_lua_value(lua_state, udv)?;
            return Ok(());
        }
    }

    // Try to get metamethod from p1, then p2
    let metamethod = get_binop_metamethod(lua_state, &p1, &p2, tm_kind);
    if let Some(mm) = metamethod {
        // Call metamethod with (p1, p2) as arguments
        let r = call_tm_res(lua_state, mm, p1, p2)?;
        lua_state.stack_mut()[res as usize] = r;
        Ok(())
    } else {
        // No metamethod found, return error
        let msg = match tm_kind {
            TmKind::Band
            | TmKind::Bor
            | TmKind::Bxor
            | TmKind::Shl
            | TmKind::Shr
            | TmKind::Bnot => {
                // Check if both values are numbers — if so, the issue is
                // that they can't be converted to integers.
                // Mirror C Lua's luaG_tointerror: include varinfo for the
                // problematic operand (the float that can't convert to int).
                if p1.is_number() && p2.is_number() {
                    // Blame the operand that's a float (not integer)
                    let blame_reg = if !p1.is_integer() { p1_reg } else { p2_reg };
                    let info = debug::varinfo_for_reg(lua_state, blame_reg);
                    return Err(
                        lua_state.error(format!("number has no integer representation{}", info))
                    );
                } else {
                    "perform bitwise operation on"
                }
            }
            _ => "perform arithmetic on",
        };
        Err(debug::opinterror(lua_state, p1_reg, p2_reg, &p1, &p2, msg))
    }
}

/// Call a metamethod with two arguments
/// Based on Lua 5.5's luaT_callTMres - returns the result value directly
/// Port of Lua 5.5's luaT_callTMres from ltm.c:119
/// ```c
/// lu_byte luaT_callTMres (lua_State *L, const TValue *f, const TValue *p1,
///                         const TValue *p2, StkId res) {
///   ptrdiff_t result = savestack(L, res);
///   StkId func = L->top.p;
///   setobj2s(L, func, f);  /* push function (assume EXTRA_STACK) */
///   setobj2s(L, func + 1, p1);  /* 1st argument */
///   setobj2s(L, func + 2, p2);  /* 2nd argument */
///   L->top.p += 3;
///   /* metamethod may yield only when called from Lua code */
///   if (isLuacode(L->ci))
///     luaD_call(L, func, 1);
///   else
///     luaD_callnoyield(L, func, 1);
///   res = restorestack(L, result);
///   setobjs2s(L, res, --L->top.p);  /* move result to its place */
///   return ttypetag(s2v(res));  /* return tag of the result */
/// }
/// ```
pub fn call_tm_res(
    lua_state: &mut LuaState,
    metamethod: LuaValue,
    arg1: LuaValue,
    arg2: LuaValue,
) -> LuaResult<LuaValue> {
    // Sync top to ci_top — callers in the inline hot path already did set_top_raw(ci_top),
    // so the comparison is almost always true. We still check for safety in other callers.
    let func_pos = {
        let ci_top = unsafe { (*lua_state.current_ci_ptr()).top as usize };
        let top = lua_state.get_top();
        if top != ci_top {
            lua_state.set_top_raw(ci_top);
        }
        ci_top
    };

    // Direct stack write using raw pointers — like Lua 5.5's setobj2s.
    // EXTRA_STACK (5 slots) guarantees space above ci->top.
    {
        let stack = lua_state.stack_mut();
        stack[func_pos] = metamethod;
        stack[func_pos + 1] = arg1;
        stack[func_pos + 2] = arg2;
    }
    lua_state.set_top_raw(func_pos + 3);

    // Call the metamethod with nresults=1
    if metamethod.is_lua_function() {
        let lua_func = unsafe { metamethod.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        let upvalue_ptrs = lua_func.upvalues().as_ptr();

        let new_base = func_pos + 1;
        let caller_depth = lua_state.call_depth();

        if !(chunk.param_count == 2
            && lua_state.try_push_lua_frame_exact(
                new_base,
                1,
                chunk.max_stack_size,
                chunk as *const _,
                upvalue_ptrs,
            )?)
        {
            lua_state.push_lua_frame(
                new_base,
                2,
                1,
                chunk.param_count,
                chunk.max_stack_size,
                chunk as *const _,
                upvalue_ptrs,
            )?;
        }
        lua_state.inc_n_ccalls()?;
        let r = lua_execute(lua_state, caller_depth);
        lua_state.dec_n_ccalls();
        r?;
    } else if metamethod.is_cfunction() {
        call_c_function(lua_state, func_pos, 2, 1)?;
    } else {
        return Err(crate::stdlib::debug::callerror(lua_state, &metamethod));
    }

    let result_val = lua_state.stack()[func_pos];
    lua_state.set_top_raw(func_pos);

    Ok(result_val)
}

#[allow(dead_code)]
pub fn call_tm_res1(
    lua_state: &mut LuaState,
    metamethod: LuaValue,
    arg1: LuaValue,
) -> LuaResult<LuaValue> {
    let func_pos = {
        let ci_top = unsafe { (*lua_state.current_ci_ptr()).top as usize };
        let top = lua_state.get_top();
        if top != ci_top {
            lua_state.set_top_raw(ci_top);
        }
        ci_top
    };

    {
        let stack = lua_state.stack_mut();
        stack[func_pos] = metamethod;
        stack[func_pos + 1] = arg1;
    }
    lua_state.set_top_raw(func_pos + 2);

    if metamethod.is_lua_function() {
        let lua_func = unsafe { metamethod.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        let upvalue_ptrs = lua_func.upvalues().as_ptr();

        let new_base = func_pos + 1;
        let caller_depth = lua_state.call_depth();

        if !(chunk.param_count == 1
            && lua_state.try_push_lua_frame_exact(
                new_base,
                1,
                chunk.max_stack_size,
                chunk as *const _,
                upvalue_ptrs,
            )?)
        {
            lua_state.push_lua_frame(
                new_base,
                1,
                1,
                chunk.param_count,
                chunk.max_stack_size,
                chunk as *const _,
                upvalue_ptrs,
            )?;
        }
        lua_state.inc_n_ccalls()?;
        let r = lua_execute(lua_state, caller_depth);
        lua_state.dec_n_ccalls();
        r?;
    } else if metamethod.is_cfunction() {
        call_c_function(lua_state, func_pos, 1, 1)?;
    } else {
        return Err(crate::stdlib::debug::callerror(lua_state, &metamethod));
    }

    let result_val = lua_state.stack()[func_pos];
    lua_state.set_top_raw(func_pos);

    Ok(result_val)
}

#[inline]
pub fn call_tm_res_into(
    lua_state: &mut LuaState,
    metamethod: &LuaValue,
    arg1: &LuaValue,
    arg2: &LuaValue,
    dest_stk_id: StkId,
) -> LuaResult<()> {
    let func_pos = {
        let ci_top = unsafe { (*lua_state.current_ci_ptr()).top as usize };
        let top = lua_state.get_top();
        if top != ci_top {
            lua_state.set_top_raw(ci_top);
        }
        ci_top
    };

    {
        let stack = lua_state.stack_mut();
        stack[func_pos] = *metamethod;
        stack[func_pos + 1] = *arg1;
        stack[func_pos + 2] = *arg2;
    }
    lua_state.set_top_raw(func_pos + 3);

    if metamethod.is_lua_function() {
        let lua_func = unsafe { metamethod.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        let upvalue_ptrs = lua_func.upvalues().as_ptr();

        let new_base = func_pos + 1;
        let caller_depth = lua_state.call_depth();

        if !(chunk.param_count == 2
            && lua_state.try_push_lua_frame_exact(
                new_base,
                1,
                chunk.max_stack_size,
                chunk as *const _,
                upvalue_ptrs,
            )?)
        {
            lua_state.push_lua_frame(
                new_base,
                2,
                1,
                chunk.param_count,
                chunk.max_stack_size,
                chunk as *const _,
                upvalue_ptrs,
            )?;
        }
        lua_state.inc_n_ccalls()?;
        let r = lua_execute(lua_state, caller_depth);
        lua_state.dec_n_ccalls();
        r?;
    } else if metamethod.is_cfunction() {
        call_c_function(lua_state, func_pos, 2, 1)?;
    } else {
        return Err(debug::callerror(lua_state, metamethod));
    }

    let result = lua_state.stack()[func_pos];
    dest_stk_id.write(&result);
    lua_state.set_top_raw(func_pos);
    Ok(())
}

/// Port of Lua 5.5's luaT_callTM from ltm.c:103
/// Calls metamethod without expecting a return value
/// ```c
/// void luaT_callTM (lua_State *L, const TValue *f, const TValue *p1,
///                   const TValue *p2, const TValue *p3) {
///   StkId func = L->top.p;
///   setobj2s(L, func, f);  /* push function (assume EXTRA_STACK) */
///   setobj2s(L, func + 1, p1);  /* 1st argument */
///   setobj2s(L, func + 2, p2);  /* 2nd argument */
///   setobj2s(L, func + 3, p3);  /* 3rd argument */
///   L->top.p = func + 4;
///   /* metamethod may yield only when called from Lua code */
///   if (isLuacode(L->ci))
///     luaD_call(L, func, 0);
///   else
///     luaD_callnoyield(L, func, 0);
/// }
/// ```
pub fn call_tm(
    lua_state: &mut LuaState,
    metamethod: LuaValue,
    arg1: LuaValue,
    arg2: LuaValue,
    arg3: LuaValue,
) -> LuaResult<()> {
    // Sync top to ci_top
    let func_pos = {
        let ci_top = unsafe { (*lua_state.current_ci_ptr()).top as usize };
        let top = lua_state.get_top();
        if top != ci_top {
            lua_state.set_top_raw(ci_top);
        }
        ci_top
    };

    // Direct stack write using raw pointers
    {
        let stack = lua_state.stack_mut();
        stack[func_pos] = metamethod;
        stack[func_pos + 1] = arg1;
        stack[func_pos + 2] = arg2;
        stack[func_pos + 3] = arg3;
    }
    lua_state.set_top_raw(func_pos + 4);

    // Call with 0 results (nresults=0)
    if metamethod.is_lua_function() {
        let lua_func = unsafe { metamethod.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        let upvalue_ptrs = lua_func.upvalues().as_ptr();

        let new_base = func_pos + 1;
        let caller_depth = lua_state.call_depth();

        if !(chunk.param_count == 3
            && lua_state.try_push_lua_frame_exact(
                new_base,
                0,
                chunk.max_stack_size,
                chunk as *const _,
                upvalue_ptrs,
            )?)
        {
            lua_state.push_lua_frame(
                new_base,
                3,
                0,
                chunk.param_count,
                chunk.max_stack_size,
                chunk as *const _,
                upvalue_ptrs,
            )?;
        }
        lua_state.inc_n_ccalls()?;
        let r = lua_execute(lua_state, caller_depth);
        lua_state.dec_n_ccalls();
        r?;
    } else if metamethod.is_cfunction() {
        call::call_c_function(lua_state, func_pos, 3, 0)?;
    } else {
        return Err(crate::stdlib::debug::callerror(lua_state, &metamethod));
    }

    Ok(())
}

/// Try comparison metamethod (for Lt and Le)
/// Returns Some(bool) if metamethod was called, None if no metamethod
pub fn try_comp_tm(
    lua_state: &mut LuaState,
    p1: LuaValue,
    p2: LuaValue,
    tm_kind: TmKind,
) -> LuaResult<Option<bool>> {
    // Try trait-based comparison for userdata
    if p1.ttisfulluserdata()
        && let Some(ud1) = p1.as_userdata_mut()
        && let Some(ud2) = p2.as_userdata_mut()
    {
        let t1 = ud1.get_trait()?;
        let t2 = ud2.get_trait()?;
        let result = match tm_kind {
            TmKind::Lt => t1.lua_lt(t2),
            TmKind::Le => t1.lua_le(t2),
            _ => None,
        };
        if let Some(b) = result {
            return Ok(Some(b));
        }
    }

    // Try to get metamethod from p1, then p2
    let metamethod = get_binop_metamethod(lua_state, &p1, &p2, tm_kind);

    if let Some(mm) = metamethod {
        // Call metamethod and convert result to boolean
        let result = call_tm_res(lua_state, mm, p1, p2)?;
        // GC check is already done in luaT_callTMres
        Ok(Some(!result.is_falsy()))
    } else {
        Ok(None)
    }
}

#[inline]
pub fn call_newindex_tm_fast(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    obj: LuaValue,
    meta: TablePtr,
    key: LuaValue,
    value: LuaValue,
) -> LuaResult<bool> {
    let Some(tm) = get_metamethod_from_meta_ptr(lua_state, meta, TmKind::NewIndex) else {
        return Ok(false);
    };
    if !tm.is_function() {
        return Ok(false);
    }

    match call_tm(lua_state, tm, obj, key, value) {
        Ok(()) => Ok(true),
        Err(LuaError::Yield) => {
            ci.set_pending_finish_get(-2);
            ci.call_status |= CIST_PENDING_FINISH;
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Tag Method types (TMS from ltm.h)
///
/// 与旧 execute 共享同一枚举：TmKind 只是事件编号，不携带执行状态。
/// 重复定义会让 crate::lua_vm::{get_metamethod_event, get_binop_metamethod}
/// 等同名 API 出现两个不兼容的枚举类型。
pub use crate::lua_vm::TmKind;
