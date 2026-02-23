/// Function call implementation
///
/// Implements CALL and TAILCALL opcodes
///
/// IMPORTANT: These do NOT recursively call execute_frame!
/// Following Lua's design:
/// - CALL: push new frame, return FrameAction::Call (main loop loads new chunk)
/// - TAILCALL: replace current frame, return FrameAction::TailCall (main loop loads new chunk)
use crate::{
    LuaValue,
    lua_vm::call_info::call_status,
    lua_vm::{CFunction, LuaError, LuaResult, LuaState, TmKind, get_metamethod_event},
};

pub enum FrameAction {
    Call,     // Pushed new frame, execute callee
    TailCall, // Replaced current frame, execute tail callee
    Continue, // C function executed, continue current frame
}

/// Handle CALL opcode - Lua style (push frame, don't recurse)
/// R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
#[inline(always)]
pub fn handle_call(
    lua_state: &mut LuaState,
    base: usize,
    a: usize,
    b: usize,
    c: usize,
    status: u32,
) -> LuaResult<FrameAction> {
    // Get function position
    let func_idx = base + a;

    // Calculate nargs and set stack_top
    let nargs = if b == 0 {
        let current_top = lua_state.get_top();
        if current_top > func_idx + 1 {
            current_top - func_idx - 1
        } else {
            0
        }
    } else {
        // Fixed arg count — stack was already grown, just move the top pointer
        lua_state.set_top_raw(func_idx + b);
        b - 1
    };

    let nresults = if c == 0 {
        -1 // Multiple return
    } else {
        (c - 1) as i32
    };

    // Get function to call — unchecked since stack was grown by push_lua_frame
    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };

    // Check if it's a Lua function (most common hot path)
    if func.is_lua_function() {
        // Extract chunk metadata once, pass to push_lua_frame to avoid redundant as_lua_function()
        let lua_func = unsafe { func.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        let param_count = chunk.param_count;
        let max_stack_size = chunk.max_stack_size;

        let new_base = func_idx + 1;
        lua_state.push_lua_frame(
            &func,
            new_base,
            nargs,
            nresults,
            param_count,
            max_stack_size,
        )?;

        // Update call_status with __call count if status is non-zero
        if status != 0 {
            let frame_idx = lua_state.call_depth() - 1;
            let ci = lua_state.get_call_info_mut(frame_idx);
            ci.call_status |= status;
        }

        Ok(FrameAction::Call)
    } else if func.is_c_callable() {
        call_c_function(lua_state, func_idx, nargs, nresults)?;
        Ok(FrameAction::Continue)
    } else {
        // Handle __call metamethod
        if let Some(mm) = get_metamethod_event(lua_state, &func, TmKind::Call) {
            // Check __call chain depth (bits 8-11 can hold 0-15, so max is 15)
            // Lua 5.5: if ((status & MAX_CCMT) == MAX_CCMT) error
            // MAX_CCMT = 0xF << 8, so when all 4 bits are 1 (count == 15), we error
            let ccmt_count = call_status::get_ccmt_count(status);
            if ccmt_count == 15 {
                return Err(lua_state.error("'__call' chain too long".to_string()));
            }

            // Shift arguments to make room for original func as first arg
            let first_arg = func_idx + 1;
            for i in (0..nargs).rev() {
                let val = lua_state.stack_get(first_arg + i).unwrap_or_default();
                lua_state.stack_set(first_arg + i + 1, val)?;
            }

            // Set func as first arg of metamethod
            lua_state.stack_set(first_arg, func)?;

            // Set metamethod as the function to call
            lua_state.stack_set(func_idx, mm)?;

            // Update stack top
            let new_top = first_arg + nargs + 1;
            lua_state.set_top(new_top)?;

            // Increment __call counter in status
            let new_status = call_status::set_ccmt_count(status, ccmt_count + 1);

            // Recursively call with adjusted parameters
            let new_b = if b == 0 { 0 } else { b + 1 };
            handle_call(lua_state, base, a, new_b, c, new_status)
        } else {
            Err(crate::stdlib::debug::typeerror(lua_state, &func, "call"))
        }
    }
}

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

/// Call a C function and handle results  
/// Similar to Lua's precallC - much simpler than our initial attempt
pub fn call_c_function(
    lua_state: &mut LuaState,
    func_idx: usize,
    nargs: usize,
    nresults: i32,
) -> LuaResult<()> {
    // Get the function (already validated as c_callable by caller)
    let func = lua_state.stack_mut()[func_idx];

    // Get the C function pointer - handle light C functions, GC C closures, and Rust closures
    let c_func: Option<CFunction> = if let Some(c_func) = func.as_cfunction() {
        Some(c_func)
    } else if let Some(cclosure) = func.as_cclosure() {
        Some(cclosure.func())
    } else if func.is_rclosure() {
        None // RClosure - will be called via trait object below
    } else {
        return Err(lua_state.error("Not a callable value".to_string()));
    };

    let call_base = func_idx + 1;

    // Use lean push_c_frame (skips type dispatch)
    lua_state.push_c_frame(&func, call_base, nargs, nresults)?;

    // Call the function (it returns number of results)
    let n = if let Some(c_func) = c_func {
        match c_func(lua_state) {
            Ok(n) => n,
            Err(LuaError::Yield) => {
                return Err(LuaError::Yield);
            }
            Err(e) => return Err(e),
        }
    } else {
        // RClosure path: call through the trait object.
        // Safety: func is a Copy of the stack value, as_rclosure() dereferences
        // the GcPtr into GC heap memory. The GC won't run during this call
        // because we hold &mut LuaState.
        let rclosure = func.as_rclosure().unwrap();
        match rclosure.call(lua_state) {
            Ok(n) => n,
            Err(LuaError::Yield) => {
                return Err(LuaError::Yield);
            }
            Err(e) => return Err(e),
        }
    };

    // Results positions
    let stack_top = lua_state.get_top();
    let first_result = if stack_top >= n {
        stack_top - n
    } else {
        call_base
    };

    // Pop frame (lean path, no call_status bit check)
    lua_state.pop_c_frame();

    // Move results using unsafe fast path
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

    let final_nresults = if nresults == -1 { n } else { nresults as usize };
    let new_top = func_idx + final_nresults;

    // Clear stale references above new_top to prevent dead objects from being
    // kept alive when stack_top is later raised (e.g. by push_lua_frame).
    {
        let clear_end = stack_top.min(lua_state.stack_len());
        if clear_end > new_top {
            let stack = lua_state.stack_mut();
            for i in new_top..clear_end {
                stack[i] = LuaValue::nil();
            }
        }
    }

    // Restore caller frame top
    if lua_state.call_depth() > 0 {
        let ci_idx = lua_state.call_depth() - 1;
        if nresults == -1 {
            let ci_top = lua_state.get_call_info(ci_idx).top;
            if ci_top < new_top {
                lua_state.get_call_info_mut(ci_idx).top = new_top;
            }
            lua_state.set_top_raw(new_top);
        } else {
            let frame_top = lua_state.get_call_info(ci_idx).top;
            lua_state.set_top_raw(frame_top);
        }
    } else {
        lua_state.set_top_raw(new_top);
    }

    Ok(())
}

/// Fast path for calling a known C function, e.g. from TForCall.
/// Caller already extracted the CFunction pointer, so we skip all type
/// dispatch. Uses `push_c_frame` / `pop_c_frame` (no call_status bit check)
/// and unsafe move_results with no per-element bounds checking.
#[inline(always)]
pub fn call_c_function_fast(
    lua_state: &mut LuaState,
    func: &LuaValue,
    c_func: CFunction,
    func_idx: usize,
    nargs: usize,
    nresults: i32,
) -> LuaResult<()> {
    let call_base = func_idx + 1;

    // Lean frame push — no type dispatch
    lua_state.push_c_frame(func, call_base, nargs, nresults)?;

    // Call the C function
    let n = match c_func(lua_state) {
        Ok(n) => n,
        Err(LuaError::Yield) => return Err(LuaError::Yield),
        Err(e) => return Err(e),
    };

    // Results positions
    let stack_top = lua_state.get_top();
    let first_result = if stack_top >= n {
        stack_top - n
    } else {
        call_base
    };

    // Pop frame — no call_status bit check
    lua_state.pop_c_frame();

    // Fast unsafe move_results for small fixed result counts
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
            _ => {
                // General case (e.g. TForCall with c results)
                let wanted = if nresults < 0 { n } else { nresults as usize };
                let copy_count = n.min(wanted);
                for i in 0..copy_count {
                    *stack.get_unchecked_mut(func_idx + i) = *stack.get_unchecked(first_result + i);
                }
                // Pad with nil if n < wanted
                for i in copy_count..wanted {
                    *stack.get_unchecked_mut(func_idx + i) = LuaValue::nil();
                }
            }
        }
    }

    // Restore caller frame top
    let final_n = if nresults == -1 { n } else { nresults as usize };
    let new_top = func_idx + final_n;

    if lua_state.call_depth() > 0 {
        let ci_idx = lua_state.call_depth() - 1;
        if nresults == -1 {
            let ci_top = lua_state.get_call_info(ci_idx).top;
            if ci_top < new_top {
                lua_state.get_call_info_mut(ci_idx).top = new_top;
            }
            lua_state.set_top_raw(new_top);
        } else {
            let frame_top = lua_state.get_call_info(ci_idx).top;
            lua_state.set_top_raw(frame_top);
        }
    } else {
        lua_state.set_top_raw(new_top);
    }

    Ok(())
}

/// Handle TAILCALL opcode - Lua style (replace frame, don't recurse)
/// Tail call optimization: return R[A](R[A+1], ... ,R[A+B-1])
#[inline]
pub fn handle_tailcall(
    lua_state: &mut LuaState,
    base: usize,
    a: usize,
    b: usize,
) -> LuaResult<FrameAction> {
    // Save the actual stack_top BEFORE syncing with frame.top
    // This is needed for variable args (b==0) calculation
    let actual_stack_top = lua_state.get_top();

    // REMOVED: Sync stack_top with frame.top
    // This was causing issues when resolve_call_chain increased stack_top beyond frame.top
    // and then this sync would truncate it, losing arguments
    //
    // if let Some(frame) = lua_state.current_frame() {
    //     let frame_top = frame.top;
    //     lua_state.set_top(frame_top)?;
    // }

    let nargs = if b == 0 {
        // Variable args: use actual_stack_top (saved above), NOT frame.top
        // frame.top is the stack limit (base + maxstacksize), not current top
        let func_idx = base + a;
        let first_arg = func_idx + 1;

        actual_stack_top.saturating_sub(first_arg)
    } else {
        b - 1
    };

    // Get function to call
    let func_idx = base + a;
    let func = lua_state
        .stack_get(func_idx)
        .ok_or_else(|| lua_state.error("TAILCALL: function not found".to_string()))?;

    // Check if it's a function
    if func.is_lua_function() {
        let current_frame_idx = lua_state.call_depth() - 1;

        // Close upvalues from current call before moving arguments
        // This is critical: like Lua 5.5's OP_TAILCALL which calls luaF_closeupval(L, base)
        // We need to close upvalues that reference the current frame's locals
        // because we're about to overwrite them with the new function's arguments
        lua_state.close_upvalues(base);

        // Like Lua 5.5's luaD_pretailcall: move function and arguments down
        // to ci->func.p position (= base - func_offset), NOT base-1.
        // This is critical for vararg functions where buildhiddenargs shifts
        // base forward, making func_offset > 1. The return handler uses
        // func_pos = base - func_offset to find where to place results,
        // so func must be moved there to keep return positions correct.
        let current_frame = lua_state.get_call_info(current_frame_idx);
        let func_offset = current_frame.func_offset;
        let func_pos = base - func_offset;

        let narg1 = nargs + 1; // Include function itself
        for i in 0..narg1 {
            let src_idx = func_idx + i;
            let dst_idx = func_pos + i;
            if let Some(val) = lua_state.stack_get(src_idx) {
                lua_state.stack_set(dst_idx, val)?;
            } else {
                lua_state.stack_set(dst_idx, LuaValue::nil())?;
            }
        }

        // After moving, update func reference to the moved position
        let new_base = func_pos + 1;
        let moved_func = lua_state.stack_get(func_pos).unwrap_or(func);

        // Get the moved function body for parameter info
        let moved_func_body = moved_func.as_lua_function().ok_or_else(|| {
            lua_state.error("TAILCALL: moved function is not a Lua function".to_string())
        })?;

        // Pad missing parameters with nil if needed
        let chunk = moved_func_body.chunk();
        let numparams = chunk.param_count;
        let mut current_nargs = nargs;

        // Pad fixed parameters with nil if needed
        while current_nargs < numparams {
            lua_state.stack_set(new_base + current_nargs, LuaValue::nil())?;
            current_nargs += 1;
        }

        // nextraargs = max(0, nargs - numparams)
        let new_nextraargs = if nargs > numparams {
            (nargs - numparams) as i32
        } else {
            0
        };

        lua_state.set_frame_nextraargs(current_frame_idx, new_nextraargs);

        // Update frame base and func_offset: reset to standard layout
        // (func at new_base - 1, func_offset = 1). VARARGPREP will adjust
        // again if the new function is vararg.
        {
            let ci = lua_state.get_call_info_mut(current_frame_idx);
            ci.base = new_base;
            ci.func_offset = 1;
        }

        // Update frame top: func + 1 + maxstacksize
        let frame_top = func_pos + 1 + chunk.max_stack_size;
        // Ensure physical stack is large enough for the new function.
        // The tailcalled function may need more stack space than the caller.
        let needed_physical = frame_top + 5; // +5 = EXTRA_STACK
        if needed_physical > lua_state.stack_len() {
            lua_state.grow_stack(needed_physical)?;
        }
        lua_state.set_frame_top(current_frame_idx, frame_top);

        // Set stack top: new_base + current_nargs
        let stack_top = new_base + current_nargs;
        lua_state.set_top(stack_top)?;

        // Update frame func pointer to the moved function
        lua_state.set_frame_func(current_frame_idx, moved_func);

        // Reset PC to 0 to start executing the new function from beginning
        lua_state.set_frame_pc(current_frame_idx, 0);

        // Return FrameAction::TailCall - main loop will load new chunk and continue
        Ok(FrameAction::TailCall)
    } else if func.is_cfunction() || func.is_cclosure() || func.is_rclosure() {
        // C function / Rust closure tail call
        call_c_function_tailcall(lua_state, func_idx, nargs, base)?;
        Ok(FrameAction::Continue)
    } else {
        // Not a function - resolve __call chain first
        let (actual_nargs, _ccmt_depth) = resolve_call_chain(lua_state, func_idx, nargs)?;

        // After resolution, recurse once to handle the actual call
        // (but won't recurse again since __call chain is now resolved)
        let new_b = if b == 0 {
            0 // Keep varargs indicator
        } else {
            // Adjust b for the additional arguments from __call chain
            let delta = actual_nargs - nargs;
            b + delta
        };
        handle_tailcall(lua_state, base, a, new_b)
    }
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
