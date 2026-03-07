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
    lua_vm::lua_limits::EXTRA_STACK,
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
            chunk as *const _,
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
        // Check userdata lua_call trait method first
        if func.ttisfulluserdata()
            && let Some(ud) = func.as_userdata_mut()
            && let Some(call_fn) = ud.get_trait().lua_call()
        {
            // Replace the userdata with the CFunction on stack, shift userdata as arg1
            let first_arg = func_idx + 1;
            for i in (0..nargs).rev() {
                let val = lua_state.stack_get(first_arg + i).unwrap_or_default();
                lua_state.stack_set(first_arg + i + 1, val)?;
            }
            // Set userdata as first argument
            lua_state.stack_set(first_arg, func)?;
            // Set CFunction as the function to call
            lua_state.stack_set(func_idx, LuaValue::cfunction(call_fn))?;
            // Update stack top
            let new_top = first_arg + nargs + 1;
            lua_state.set_top(new_top)?;
            // Recurse with adjusted args
            let new_b = if b == 0 { 0 } else { b + 1 };
            return handle_call(lua_state, base, a, new_b, c, status);
        }

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

    // Fire call hook for C functions (matches C Lua's precallC)
    {
        let hook_mask = lua_state.hook_mask;
        if hook_mask & crate::lua_vm::LUA_MASKCALL != 0 && lua_state.allow_hook {
            // ftransfer=1, ntransfer=narg (actual args passed to C function)
            let narg = (lua_state.get_top() - call_base) as i32;
            lua_state.run_hook(crate::lua_vm::LUA_HOOKCALL, -1, 1, narg)?;
        }
    }

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

    // Fire return hook for C functions (matches C Lua's rethook)
    {
        let hook_mask = lua_state.hook_mask;
        if hook_mask & crate::lua_vm::LUA_MASKRET != 0 && lua_state.allow_hook {
            // ftransfer = 1-based index relative to base (call_base)
            let ftransfer = (first_result - call_base + 1) as i32;
            lua_state.run_hook(crate::lua_vm::LUA_HOOKRET, -1, ftransfer, n as i32)?;
        }
    }

    // Pop frame (lean path, no call_status bit check)
    lua_state.pop_c_frame();

    // Update oldpc for the caller frame (like C Lua's rethook in
    // luaD_poscall: L->oldpc = pcRel(ci->u.l.savedpc, ci_func(ci)->p)).
    // This ensures same-line suppression after C function returns.
    // In C Lua, rethook is called from luaD_poscall for ALL function
    // returns (both C and Lua).
    if lua_state.call_depth() > 0 {
        let ci = lua_state.get_call_info(lua_state.call_depth() - 1);
        if ci.call_status & crate::lua_vm::call_info::call_status::CIST_C == 0 {
            // Caller is a Lua frame: set oldpc to ci.pc - 1
            // (matching hook_check_instruction's npci = pc - 1 convention)
            let saved_pc = ci.pc;
            lua_state.oldpc = if saved_pc > 0 { saved_pc - 1 } else { 0 };
        }
    }

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

    // Restore caller frame top
    if lua_state.call_depth() > 0 {
        let ci_idx = lua_state.call_depth() - 1;
        if nresults == -1 {
            let ci_top = lua_state.get_call_info(ci_idx).top as usize;
            if ci_top < new_top {
                lua_state.get_call_info_mut(ci_idx).top = new_top as u32;
            }
            lua_state.set_top_raw(new_top);
        } else {
            let frame_top = lua_state.get_call_info(ci_idx).top as usize;
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
            let ci_top = lua_state.get_call_info(ci_idx).top as usize;
            if ci_top < new_top {
                lua_state.get_call_info_mut(ci_idx).top = new_top as u32;
            }
            lua_state.set_top_raw(new_top);
        } else {
            let frame_top = lua_state.get_call_info(ci_idx).top as usize;
            lua_state.set_top_raw(frame_top);
        }
    } else {
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

/// Handle TAILCALL opcode - Lua style (replace frame, don't recurse)
/// Tail call optimization: return R[A](R[A+1], ... ,R[A+B-1])
#[inline(never)]
pub fn handle_tailcall(
    lua_state: &mut LuaState,
    base: usize,
    a: usize,
    b: usize,
) -> LuaResult<FrameAction> {
    // Save the actual stack_top BEFORE syncing with frame.top
    // This is needed for variable args (b==0) calculation
    let actual_stack_top = lua_state.get_top();

    let nargs = if b == 0 {
        // Variable args: use actual_stack_top (saved above), NOT frame.top
        let func_idx = base + a;
        actual_stack_top.saturating_sub(func_idx + 1)
    } else {
        b - 1
    };

    // Get function to call — unchecked since stack was grown by push_lua_frame
    let func_idx = base + a;
    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };

    // Hot path: Lua function tail call
    // Combined type check + extraction (one enum match instead of two)
    if let Some(lua_func) = func.as_lua_function() {
        let current_frame_idx = lua_state.call_depth() - 1;

        // Close upvalues from current call before moving arguments
        // This is critical: like Lua 5.5's OP_TAILCALL which calls luaF_closeupval(L, base)
        lua_state.close_upvalues(base);

        // Get function position (handles vararg func_offset)
        let func_offset = lua_state.get_call_info(current_frame_idx).func_offset;
        let func_pos = base - func_offset as usize;

        // Move function + arguments down using ptr::copy (like C Lua's setobjs2s loop)
        // This replaces per-element bounds-checked stack_get/stack_set
        let narg1 = nargs + 1;
        unsafe {
            let stack_ptr = lua_state.stack_mut().as_mut_ptr();
            std::ptr::copy(stack_ptr.add(func_idx), stack_ptr.add(func_pos), narg1);
        }

        let new_base = func_pos + 1;
        let chunk = lua_func.chunk();
        let numparams = chunk.param_count;

        // Pad missing parameters with nil (like C Lua's narg1..nfixparams loop)
        if nargs < numparams {
            let stack = lua_state.stack_mut();
            for i in nargs..numparams {
                unsafe { *stack.get_unchecked_mut(new_base + i) = LuaValue::nil() };
            }
        }

        let actual_nargs = nargs.max(numparams);
        let nextraargs = if nargs > numparams {
            (nargs - numparams) as i32
        } else {
            0
        };

        let frame_top = func_pos + 1 + chunk.max_stack_size;

        // Ensure physical stack is large enough for the new function
        let needed_physical = frame_top + EXTRA_STACK;
        if needed_physical > lua_state.stack_len() {
            lua_state.grow_stack(needed_physical)?;
        }

        // Batch update all CI fields in one get_call_info_mut call
        // (eliminates 6+ separate call_stack[idx] lookups)
        let ci = lua_state.get_call_info_mut(current_frame_idx);
        ci.func = func;
        ci.base = new_base;
        ci.func_offset = 1;
        ci.top = frame_top as u32;
        ci.pc = 0;
        ci.nextraargs = nextraargs;
        ci.call_status |= call_status::CIST_TAIL;
        ci.chunk_ptr = chunk as *const _;
        ci.upvalue_ptrs = unsafe { func.as_lua_function_unchecked().upvalues().as_ptr() };

        // Set stack top (no bounds check needed — we ensured space above)
        lua_state.set_top_raw(new_base + actual_nargs);

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
