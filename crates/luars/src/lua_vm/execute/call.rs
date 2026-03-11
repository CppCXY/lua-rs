/// Function call implementation
///
/// Implements CALL and TAILCALL opcodes via `precall` / `pretailcall`.
use crate::{
    LuaValue,
    lua_vm::call_info::call_status,
    lua_vm::lua_limits::EXTRA_STACK,
    lua_vm::{CFunction, LuaResult, LuaState, TmKind, get_metamethod_event},
};

/// Resolve __call metamethod chain in place
/// Modifies stack to replace non-callable with its __call chain
/// Returns (actual_arg_count, ccmt_depth) after resolution
/// func_idx position stays the same, but stack content is modified
pub fn resolve_call_chain(
    lua_state: &mut LuaState,
    func_idx: usize,
    arg_count: usize,
) -> LuaResult<(usize, u8)> {
    let mut current_arg_count = arg_count;
    let mut ccmt_depth = 0;

    loop {
        let func = lua_state
            .stack_get(func_idx)
            .ok_or_else(|| lua_state.error("resolve_call_chain: function not found".to_string()))?;

        // Check if we have a callable function
        if func.is_c_callable() || func.is_lua_function() {
            // Found a real function - done
            return Ok((current_arg_count, ccmt_depth));
        }

        // Try userdata lua_call trait method first
        if func.ttisfulluserdata()
            && let Some(ud) = func.as_userdata_mut()
            && let Some(call_fn) = ud.get_trait().lua_call()
        {
            let first_arg = func_idx + 1;
            for i in (0..current_arg_count).rev() {
                let val = lua_state.stack_get(first_arg + i).unwrap_or_default();
                lua_state.stack_set(first_arg + i + 1, val)?;
            }
            lua_state.stack_set(first_arg, func)?;
            lua_state.stack_set(func_idx, LuaValue::cfunction(call_fn))?;
            current_arg_count += 1;
            lua_state.set_top(first_arg + current_arg_count)?;
            // CFunction is callable — will match on next iteration
            continue;
        }

        // Try to get __call metamethod
        if let Some(mm) = get_metamethod_event(lua_state, &func, TmKind::Call) {
            // Check chain depth (Lua 5.5 allows up to 15 __call layers)
            // We check BEFORE incrementing, so if we're already at 15, error
            if ccmt_depth == 15 {
                return Err(lua_state.error("'__call' chain too long".to_string()));
            }
            ccmt_depth += 1;

            // Shift arguments right to make room for original func as first arg
            // Stack: [func, arg1, arg2, ...] -> [mm, func, arg1, arg2, ...]
            let first_arg = func_idx + 1;
            for i in (0..current_arg_count).rev() {
                let val = lua_state.stack_get(first_arg + i).unwrap_or_default();
                lua_state.stack_set(first_arg + i + 1, val)?;
            }

            // Set original func as first argument
            lua_state.stack_set(first_arg, func)?;

            // Set metamethod as the new function
            lua_state.stack_set(func_idx, mm)?;

            // Update arg count and stack top
            current_arg_count += 1;
            lua_state.set_top(first_arg + current_arg_count)?;

            // Continue loop to check if mm also needs __call resolution
        } else {
            // No __call metamethod and not a function
            return Err(crate::stdlib::debug::typeerror(lua_state, &func, "call"));
        }
    }
}

/// Call a C function and handle results.
/// Like C Lua's precallC + poscall combined.
///
/// Caller (precall / the dispatch loop) is responsible for setting
/// `lua_state.oldpc` after this returns — we skip it here to avoid
/// redundant loads on the hot path.
pub fn call_c_function(
    lua_state: &mut LuaState,
    func_idx: usize,
    nargs: usize,
    nresults: i32,
) -> LuaResult<()> {
    // Get the function (already validated as c_callable by caller)
    let func = lua_state.stack_mut()[func_idx];

    // Extract CFunction pointer (light C function or CClosure).
    // RClosure has no raw fn ptr — handled via trait object.
    let c_func: Option<CFunction> = if let Some(f) = func.as_cfunction() {
        Some(f)
    } else {
        func.as_cclosure().map(|cc| cc.func())
    };

    let call_base = func_idx + 1;

    // Push C frame (lean path)
    lua_state.push_c_frame(&func, call_base, nargs, nresults)?;

    // Call hook (cold — almost never fires)
    if lua_state.hook_mask & crate::lua_vm::LUA_MASKCALL != 0 && lua_state.allow_hook {
        let narg = (lua_state.get_top() - call_base) as i32;
        lua_state.run_hook(crate::lua_vm::LUA_HOOKCALL, -1, 1, narg)?;
    }

    // Call the function — `?` propagates errors including Yield
    // (on Yield the frame stays on the stack for resume)
    let n = if let Some(c_func) = c_func {
        c_func(lua_state)?
    } else {
        func.as_rclosure().unwrap().call(lua_state)?
    };

    // Results positions
    let stack_top = lua_state.get_top();
    let first_result = if stack_top >= n {
        stack_top - n
    } else {
        call_base
    };

    // Return hook (cold)
    if lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0 && lua_state.allow_hook {
        let ftransfer = (first_result - call_base + 1) as i32;
        lua_state.run_hook(crate::lua_vm::LUA_HOOKRET, -1, ftransfer, n as i32)?;
    }

    // Pop frame
    lua_state.pop_c_frame();

    // Move results + restore caller top
    // call_depth >= 1 guaranteed: call_c_function is always called from
    // within a Lua frame (precall / dispatch), so popping the C frame
    // leaves at least the caller Lua frame.
    debug_assert!(lua_state.call_depth() > 0);
    unsafe {
        let stack = lua_state.stack_mut();
        match nresults {
            0 => { /* nothing to move */ }
            1 => {
                *stack.get_unchecked_mut(func_idx) = if n > 0 {
                    *stack.get_unchecked(first_result)
                } else {
                    LuaValue::nil()
                };
            }
            _ if nresults > 0 => {
                let wanted = nresults as usize;
                let copy_count = n.min(wanted);
                for i in 0..copy_count {
                    *stack.get_unchecked_mut(func_idx + i) = *stack.get_unchecked(first_result + i);
                }
                for i in copy_count..wanted {
                    *stack.get_unchecked_mut(func_idx + i) = LuaValue::nil();
                }
            }
            _ => {
                // MULTRET (-1)
                for i in 0..n {
                    *stack.get_unchecked_mut(func_idx + i) = *stack.get_unchecked(first_result + i);
                }
            }
        }
    }

    // Restore caller top
    let ci_idx = lua_state.call_depth() - 1;
    if nresults >= 0 {
        // Fixed results: restore caller's frame top
        let frame_top = lua_state.get_call_info(ci_idx).top as usize;
        lua_state.set_top_raw(frame_top);
    } else {
        // MULTRET: top = func_idx + n
        let new_top = func_idx + n;
        let ci_top = lua_state.get_call_info(ci_idx).top as usize;
        if ci_top < new_top {
            lua_state.get_call_info_mut(ci_idx).top = new_top as u32;
        }
        lua_state.set_top_raw(new_top);
    }

    Ok(())
}

/// Lua-to-Lua tail call core: move args, update CI, return chunk_ptr.
/// Kept out-of-line to avoid bloating lua_execute's dispatch loop.
#[inline(never)]
pub fn pretailcall_lua(
    lua_state: &mut LuaState,
    func: LuaValue,
    lua_func: &crate::lua_value::LuaFunction,
    func_idx: usize,
    base: usize,
    nargs: usize,
    frame_idx: usize,
) -> LuaResult<*const crate::lua_value::Chunk> {
    let chunk = lua_func.chunk();
    let new_chunk_ptr = chunk as *const crate::lua_value::Chunk;
    let numparams = chunk.param_count;

    // Get current frame's func position (handles vararg func_offset)
    let func_offset = lua_state.get_call_info(frame_idx).func_offset;
    let func_pos = base - func_offset as usize;

    // Move function + arguments down (like C Lua's setobjs2s loop)
    let narg1 = nargs + 1;
    unsafe {
        let stack_ptr = lua_state.stack_mut().as_mut_ptr();
        std::ptr::copy(stack_ptr.add(func_idx), stack_ptr.add(func_pos), narg1);
    }

    let new_base = func_pos + 1;

    // Pad missing parameters with nil
    if nargs < numparams {
        let stack = lua_state.stack_mut();
        for i in nargs..numparams {
            unsafe { *stack.get_unchecked_mut(new_base + i) = LuaValue::nil() };
        }
    }

    let actual_nargs = nargs.max(numparams);
    let frame_top = func_pos + 1 + chunk.max_stack_size;

    // Ensure physical stack is large enough
    let needed_physical = frame_top + EXTRA_STACK;
    if needed_physical > lua_state.stack_len() {
        lua_state.grow_stack(needed_physical)?;
    }

    // Batch update CI fields (reuse current frame, no push/pop)
    let ci = lua_state.get_call_info_mut(frame_idx);
    ci.func = func;
    ci.base = new_base;
    ci.func_offset = 1;
    ci.top = frame_top as u32;
    ci.pc = 0;
    ci.nextraargs = if nargs > numparams {
        (nargs - numparams) as i32
    } else {
        0
    };
    ci.call_status |= call_status::CIST_TAIL;
    ci.chunk_ptr = new_chunk_ptr;
    ci.upvalue_ptrs = unsafe { func.as_lua_function_unchecked().upvalues().as_ptr() };

    lua_state.set_top_raw(new_base + actual_nargs);

    Ok(new_chunk_ptr)
}

/// Handle C function in tail call position
/// Results are moved to frame base for proper return
fn call_c_function_tailcall(
    lua_state: &mut LuaState,
    func_idx: usize,
    nargs: usize,
    _base: usize,
) -> LuaResult<()> {
    // Get the function
    let func = lua_state.stack_mut()[func_idx];

    // Get the C function pointer (None for RClosure)
    let c_func: Option<CFunction> = if let Some(c_func) = func.as_cfunction() {
        Some(c_func)
    } else if let Some(cclosure) = func.as_cclosure() {
        Some(cclosure.func())
    } else if func.is_rclosure() {
        None
    } else {
        return Err(lua_state.error("Not a callable value".to_string()));
    };

    let call_base = func_idx + 1;

    // Use lean push_c_frame
    lua_state.push_c_frame(&func, call_base, nargs, -1)?;

    // Call the function
    let n = if let Some(c_func) = c_func {
        c_func(lua_state)?
    } else {
        let rclosure = func.as_rclosure().unwrap();
        rclosure.call(lua_state)?
    };

    // Get the position of results BEFORE popping frame
    let stack_top = lua_state.get_top();
    let first_result = if stack_top >= n {
        stack_top - n
    } else {
        call_base
    };

    // Pop the frame (lean path)
    lua_state.pop_c_frame();

    // For tail call, move results to func_idx
    unsafe {
        let stack = lua_state.stack_mut();
        for i in 0..n {
            *stack.get_unchecked_mut(func_idx + i) = *stack.get_unchecked(first_result + i);
        }
    }

    let new_top = func_idx + n;
    lua_state.set_top_raw(new_top);

    Ok(())
}

/// Like C Lua's `luaD_precall`
/// Caller MUST set stack top before calling:
///   `if b != 0 { lua_state.set_top_raw(func_idx + b); }`
///
/// Returns:
///   `Ok(true)`  — Lua call: new frame pushed, caller should `continue 'startfunc`
///   `Ok(false)` — C call: completed inline, caller should `updatetrap` + continue
#[inline(never)]
pub fn precall(lua_state: &mut LuaState, func_idx: usize, nresults: i32) -> LuaResult<bool> {
    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };

    // Hot path: Lua function
    if func.is_lua_function() {
        let lua_func = unsafe { func.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        let narg = lua_state.get_top() - func_idx - 1;
        lua_state.push_lua_frame(
            &func,
            func_idx + 1,
            narg,
            nresults,
            chunk.param_count,
            chunk.max_stack_size,
            chunk as *const _,
        )?;
        return Ok(true);
    }

    // Hot path #2: C callable (light C function, CClosure, RClosure)
    if func.is_c_callable() {
        let nargs = lua_state.get_top() - func_idx - 1;
        call_c_function(lua_state, func_idx, nargs, nresults)?;
        return Ok(false);
    }

    // Cold: __call metamethod, userdata lua_call, or error
    precall_meta(lua_state, func_idx, nresults)
}

/// Cold path for precall: resolve __call chain then retry.
#[cold]
#[inline(never)]
fn precall_meta(lua_state: &mut LuaState, func_idx: usize, nresults: i32) -> LuaResult<bool> {
    let nargs = lua_state.get_top() - func_idx - 1;
    let (_, ccmt_depth) = resolve_call_chain(lua_state, func_idx, nargs)?;

    // After resolution, func_idx has the real callable
    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };
    let nargs = lua_state.get_top() - func_idx - 1;

    if func.is_lua_function() {
        let lua_func = unsafe { func.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        lua_state.push_lua_frame(
            &func,
            func_idx + 1,
            nargs,
            nresults,
            chunk.param_count,
            chunk.max_stack_size,
            chunk as *const _,
        )?;
        if ccmt_depth > 0 {
            let fi = lua_state.call_depth() - 1;
            let status = call_status::set_ccmt_count(0, ccmt_depth);
            lua_state.get_call_info_mut(fi).call_status |= status;
        }
        return Ok(true);
    }

    if func.is_c_callable() {
        call_c_function(lua_state, func_idx, nargs, nresults)?;
        return Ok(false);
    }

    Err(crate::stdlib::debug::typeerror(lua_state, &func, "call"))
}

/// Like C Lua's `luaD_pretailcall` (ldo.c:668-713).
/// Caller MUST set stack top and close upvalues before calling.
///
/// `narg1`: number of arguments + 1 (includes the function itself), matching C Lua.
///
/// Returns:
///   `Ok(true)`  — Lua tail call: CI reused in place, caller should `continue 'startfunc`
///   `Ok(false)` — C tail call: completed, caller continues (falls to next instruction)
#[inline(never)]
pub fn pretailcall(
    lua_state: &mut LuaState,
    func_idx: usize,
    narg1: usize,
    frame_idx: usize,
) -> LuaResult<bool> {
    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };

    // Hot path: Lua function
    if func.is_lua_function() {
        let lua_func = unsafe { func.as_lua_function_unchecked() };
        let base = lua_state.get_call_info(frame_idx).base;
        pretailcall_lua(
            lua_state,
            func,
            lua_func,
            func_idx,
            base,
            narg1 - 1,
            frame_idx,
        )?;
        return Ok(true);
    }

    // C callable
    if func.is_c_callable() {
        let base = lua_state.get_call_info(frame_idx).base;
        call_c_function_tailcall(lua_state, func_idx, narg1 - 1, base)?;
        return Ok(false);
    }

    // Cold: __call metamethod
    pretailcall_meta(lua_state, func_idx, narg1, frame_idx)
}

/// Cold path for pretailcall: resolve __call chain then retry.
#[cold]
#[inline(never)]
fn pretailcall_meta(
    lua_state: &mut LuaState,
    func_idx: usize,
    narg1: usize,
    frame_idx: usize,
) -> LuaResult<bool> {
    let nargs = narg1 - 1;
    let (actual_nargs, _) = resolve_call_chain(lua_state, func_idx, nargs)?;

    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };

    if func.is_lua_function() {
        let lua_func = unsafe { func.as_lua_function_unchecked() };
        let base = lua_state.get_call_info(frame_idx).base;
        pretailcall_lua(
            lua_state,
            func,
            lua_func,
            func_idx,
            base,
            actual_nargs,
            frame_idx,
        )?;
        return Ok(true);
    }

    if func.is_c_callable() {
        let base = lua_state.get_call_info(frame_idx).base;
        call_c_function_tailcall(lua_state, func_idx, actual_nargs, base)?;
        return Ok(false);
    }

    Err(crate::stdlib::debug::typeerror(lua_state, &func, "call"))
}
