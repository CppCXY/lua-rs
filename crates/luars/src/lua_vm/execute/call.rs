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
#[inline]
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
        lua_state.set_top(func_idx + b)?;
        b - 1
    };

    let nresults = if c == 0 {
        -1 // Multiple return
    } else {
        (c - 1) as i32
    };

    // Get function to call
    let func = lua_state
        .stack_get(func_idx)
        .ok_or_else(|| lua_state.error("CALL: function not found".to_string()))?;

    // Check if it's a GC function (Lua or C)
    if func.is_lua_function() {
        // Lua function call: push new frame
        let new_base = func_idx + 1;
        lua_state.push_frame(&func, new_base, nargs, nresults)?;

        // Update call_status with __call count if status is non-zero
        if status != 0 {
            let frame_idx = lua_state.call_depth() - 1;
            let current_status = lua_state
                .get_frame(frame_idx)
                .map(|f| f.call_status)
                .unwrap_or(0);
            let new_status = current_status | status;
            lua_state.set_frame_call_status(frame_idx, new_status);
        }

        Ok(FrameAction::Call)
    } else if func.is_c_callable() {
        call_c_function(lua_state, func_idx, nargs, nresults)?;
        return Ok(FrameAction::Continue);
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
                let val = lua_state
                    .stack_get(first_arg + i)
                    .unwrap_or(LuaValue::nil());
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
            return handle_call(lua_state, base, a, new_b, c, new_status);
        } else {
            Err(lua_state.error(format!("attempt to call a {} value", func.type_name())))
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
                let val = lua_state
                    .stack_get(first_arg + i)
                    .unwrap_or(LuaValue::nil());
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
            return Err(lua_state.error(format!("attempt to call a {} value", func.type_name())));
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
    // Get the function
    let func = lua_state
        .stack_get(func_idx)
        .ok_or_else(|| lua_state.error("C function not found".to_string()))?;

    // Get the C function pointer - handle both light C functions and GC C functions
    let c_func: CFunction = if let Some(c_func) = func.as_cfunction() {
        // Light C function - extract directly from value
        c_func
    } else if let Some(cclsoure) = func.as_cclosure() {
        // GC function - need to get from object pool
        cclsoure.func()
    } else {
        return Err(lua_state.error("Not a callable value".to_string()));
    };

    let call_base = func_idx + 1;

    // Push temporary frame for C function with nresults
    lua_state.push_frame(&func, call_base, nargs, nresults)?;

    // Call the C function (it returns number of results)
    let result = c_func(lua_state);

    // Now handle the result
    let n = match result {
        Ok(n) => n,
        Err(LuaError::Yield) => {
            // Special case: C function yielded
            // Keep the frame on stack so resume can continue
            return Err(LuaError::Yield);
        }
        Err(e) => return Err(e),
    };

    // Get logical stack top BEFORE popping frame (L->top.p in Lua)
    // C function pushes results, so first result is at top - n
    let stack_top = lua_state.get_top();
    let first_result = if stack_top >= n {
        stack_top - n
    } else {
        call_base
    };

    // Pop the frame BEFORE moving results
    lua_state.pop_frame();

    // Move results from first_result to func_idx (Lua's moveresults)
    // Implements Lua's moveresults logic from ldo.c
    move_results(lua_state, func_idx, first_result, n, nresults)?;

    // Update caller frame's top to reflect the actual number of results
    // This is crucial for nested calls - the outer call needs to know
    // where the returned values end
    let final_nresults = if nresults == -1 {
        n // MULTRET: all results
    } else {
        nresults as usize // Fixed number (may be 0)
    };

    let new_top = func_idx + final_nresults;

    // Set logical stack top (L->top.p) - does NOT truncate physical stack
    // This is  old values remain in stack array but are "hidden"
    // This preserves caller's local variables which live below this top
    lua_state.set_top(new_top)?;

    lua_state.check_gc()?;

    Ok(())
}

/// Move function results to the correct position
/// Implements Lua's moveresults and genmoveresults logic
///
/// Parameters:
/// - res: target position (where results should be moved to)
/// - first_result: position of first result on stack
/// - nres: number of actual results
/// - wanted: number of results wanted (-1 for MULTRET)
#[inline]
fn move_results(
    lua_state: &mut LuaState,
    res: usize,
    first_result: usize,
    nres: usize,
    wanted: i32,
) -> LuaResult<()> {
    // Fast path for common cases (like Lua's switch in moveresults)
    match wanted {
        0 => {
            // No values needed - just return (caller will set top)
            return Ok(());
        }
        1 => {
            // One value needed (most common case for expression results)
            if nres == 0 {
                // No results - set nil
                lua_state.stack_set(res, LuaValue::nil())?;
            } else {
                // At least one result - move first one
                let val = lua_state.stack_get(first_result).unwrap_or(LuaValue::nil());
                lua_state.stack_set(res, val)?;
            }
            return Ok(());
        }
        -1 => {
            // MULTRET - want all results
            for i in 0..nres {
                let val = lua_state
                    .stack_get(first_result + i)
                    .unwrap_or(LuaValue::nil());
                lua_state.stack_set(res + i, val)?;
            }
            return Ok(());
        }
        _ => {
            // General case: specific number of results (2+)
            let wanted = wanted as usize;
            let copy_count = nres.min(wanted);

            // Move actual results
            for i in 0..copy_count {
                let val = lua_state
                    .stack_get(first_result + i)
                    .unwrap_or(LuaValue::nil());
                lua_state.stack_set(res + i, val)?;
            }

            // Pad with nil if needed
            for i in copy_count..wanted {
                lua_state.stack_set(res + i, LuaValue::nil())?;
            }

            return Ok(());
        }
    }
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

    //  Sync stack_top with frame.top before reading arguments
    // This ensures stack is properly bounded for subsequent operations
    if let Some(frame) = lua_state.current_frame() {
        let frame_top = frame.top;
        lua_state.set_top(frame_top)?;
    }

    let nargs = if b == 0 {
        // Variable args: use actual_stack_top (saved above), NOT frame.top
        // frame.top is the stack limit (base + maxstacksize), not current top
        let func_idx = base + a;
        let first_arg = func_idx + 1;

        if actual_stack_top > first_arg {
            actual_stack_top - first_arg
        } else {
            0
        }
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

        // Like Lua 5.5's luaD_pretailcall: move function and arguments down together
        // Move func + args: [func, arg1, arg2, ...] to [ci->func, ci->func+1, ci->func+2, ...]
        // This is: [base-1, base, base+1, ...] positions
        let narg1 = nargs + 1; // Include function itself
        for i in 0..narg1 {
            let src_idx = func_idx + i;
            let dst_idx = (base - 1) + i;
            if let Some(val) = lua_state.stack_get(src_idx) {
                lua_state.stack_set(dst_idx, val)?;
            } else {
                lua_state.stack_set(dst_idx, LuaValue::nil())?;
            }
        }

        // After moving, update func reference to the moved position
        let moved_func = lua_state.stack_get(base - 1).unwrap_or(func);

        // Get the moved function body for parameter info
        let moved_func_body = moved_func.as_lua_function().ok_or_else(|| {
            lua_state.error("TAILCALL: moved function is not a Lua function".to_string())
        })?;

        // Pad missing parameters with nil if needed
        let chunk = moved_func_body.chunk();
        let numparams = chunk.param_count as usize;
        let mut current_nargs = nargs;

        // Pad fixed parameters with nil if needed
        while current_nargs < numparams {
            lua_state.stack_set(base + current_nargs, LuaValue::nil())?;
            current_nargs += 1;
        }

        // nextraargs = max(0, nargs - numparams)
        let new_nextraargs = if nargs > numparams {
            (nargs - numparams) as i32
        } else {
            0
        };

        lua_state.set_frame_nextraargs(current_frame_idx, new_nextraargs);

        // Update frame top: func + 1 + maxstacksize
        let frame_top = (base - 1) + 1 + chunk.max_stack_size;
        lua_state.set_frame_top(current_frame_idx, frame_top);

        // Set stack top: func + narg1 (after padding)
        let stack_top = base + current_nargs;
        lua_state.set_top(stack_top)?;

        // Update frame func pointer to the moved function
        lua_state.set_frame_func(current_frame_idx, moved_func);

        // Reset PC to 0 to start executing the new function from beginning
        lua_state.set_frame_pc(current_frame_idx, 0);

        // Return FrameAction::TailCall - main loop will load new chunk and continue
        Ok(FrameAction::TailCall)
    } else if func.is_cfunction() || func.is_cclosure() {
        // Light C function tail call (direct, not from object pool)
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
        return handle_tailcall(lua_state, base, a, new_b);
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
    let func = lua_state
        .stack_get(func_idx)
        .ok_or_else(|| lua_state.error("C function not found".to_string()))?;

    // Get the C function pointer
    let c_func: CFunction = if let Some(c_func) = func.as_cfunction() {
        c_func
    } else if let Some(cclosure) = func.as_cclosure() {
        cclosure.func()
    } else {
        return Err(lua_state.error("Not a callable value".to_string()));
    };

    let call_base = func_idx + 1;

    // Push temporary frame for C function
    // Tail call inherits caller's nresults (-1 for multi-return)
    lua_state.push_frame(&func, call_base, nargs, -1)?;

    // Call the C function
    let n = c_func(lua_state)?;

    // Get the position of results BEFORE popping frame
    // C function pushes results to stack, so they are at stack_top - n
    let stack_top = lua_state.get_top();
    let first_result = if stack_top >= n {
        stack_top - n
    } else {
        call_base
    };

    // Pop the frame
    lua_state.pop_frame();

    // For tail call, move results to func_idx (not base)
    // Because the next RETURN instruction will return from R[A] where A is from TAILCALL
    move_results(lua_state, func_idx, first_result, n, -1)?;

    // CRITICAL FIX: Update frame top so next RETURN instruction knows the correct top
    // After moving results to func_idx, the new top should be func_idx + n
    let new_top = func_idx + n;
    lua_state.set_top(new_top)?;
    if let Some(frame) = lua_state.current_frame_mut() {
        frame.top = new_top;
    }

    Ok(())
}
