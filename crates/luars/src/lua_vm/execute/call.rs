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
    lua_vm::{CFunction, LuaError, LuaResult, LuaState},
};

pub enum FrameAction {
    Return,   // Frame finished, return to caller
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
) -> LuaResult<FrameAction> {
    // CRITICAL: Sync stack_top with frame.top before reading arguments
    // During normal execution, frame.top tracks the current top, but stack_top
    // may lag behind. We need to sync them before function calls.
    if let Some(frame) = lua_state.current_frame() {
        let frame_top = frame.top;
        lua_state.set_top(frame_top);
    }

    let nargs = if b == 0 {
        // Variable args: use current frame's top
        // Arguments are from base+a+1 to current frame's top
        let func_idx = base + a;
        let first_arg = func_idx + 1;

        // Get current frame's top (not global stack top!)
        let frame_top = if let Some(frame) = lua_state.current_frame() {
            frame.top
        } else {
            lua_state.get_top() // Fallback to logical stack top
        };

        if frame_top > first_arg {
            frame_top - first_arg
        } else {
            0
        }
    } else {
        b - 1
    };

    let nresults = if c == 0 {
        -1 // Multiple return
    } else {
        (c - 1) as i32
    };

    // Get function to call
    let func_idx = base + a;
    let func = lua_state
        .stack_get(func_idx)
        .ok_or_else(|| lua_state.error("CALL: function not found".to_string()))?;

    // Check if it's a light C function (most common case for stdlib functions)
    if func.is_cfunction() {
        // Light C function call: execute directly
        call_c_function(lua_state, func_idx, nargs, nresults)?;

        // C function executed synchronously, continue with current frame
        return Ok(FrameAction::Continue);
    }

    // Check if it's a GC function (Lua or C)
    if let Some(new_func_id) = func.as_function_id() {
        let new_func = lua_state
            .vm_mut()
            .object_pool
            .get_function(new_func_id)
            .ok_or(LuaError::RuntimeError)?;

        if new_func.is_lua_function() {
            // Lua function call: push new frame
            let new_base = func_idx + 1; // Arguments start after function

            // Push new call frame with nresults from caller
            lua_state.push_frame(func, new_base, nargs, nresults)?;

            // Return FrameAction::Call - main loop will load new chunk and continue
            Ok(FrameAction::Call)
        } else if new_func.is_c_function() {
            // GC C function call: execute directly
            call_c_function(lua_state, func_idx, nargs, nresults)?;

            // C function executed synchronously, continue with current frame
            Ok(FrameAction::Continue)
        } else {
            Err(lua_state.error("CALL: unknown function type".to_string()))
        }
    } else {
        // Not a function - check for __call metamethod
        // Port of Lua 5.5's handling in ldo.c:tryfuncTM
        if let Some(mm) = get_call_metamethod(lua_state, &func) {
            // We have __call metamethod
            // Need to shift arguments and insert function as first arg
            // Stack layout before: [func, arg1, arg2, ...]
            // Stack layout after:  [__call, func, arg1, arg2, ...]
            
            // First, shift arguments to make room for func as first arg
            let first_arg = func_idx + 1;
            for i in (0..nargs).rev() {
                let val = lua_state.stack_get(first_arg + i).unwrap_or(LuaValue::nil());
                lua_state.stack_set(first_arg + i + 1, val)?;
            }
            
            // Set func as first arg of metamethod
            lua_state.stack_set(first_arg, func)?;
            
            // Set metamethod as the function to call
            lua_state.stack_set(func_idx, mm)?;
            
            // Update frame top to include the shifted argument
            let new_top = first_arg + nargs + 1;
            if let Some(frame) = lua_state.current_frame_mut() {
                frame.top = new_top;
            }
            lua_state.set_top(new_top);
            
            // Now call the metamethod with nargs+1 (including original func)
            // We need to adjust b: if b was 0 (varargs), keep it 0
            // otherwise increment it to account for the extra argument
            let new_b = if b == 0 { 0 } else { b + 1 };
            return handle_call(lua_state, base, a, new_b, c);
        } else {
            Err(lua_state.error(format!(
                "attempt to call a {} value",
                func.type_name()
            )))
        }
    }
}

/// Get __call metamethod for a value
fn get_call_metamethod(lua_state: &mut LuaState, value: &LuaValue) -> Option<LuaValue> {
    // For table: check metatable
    if let Some(table_id) = value.as_table_id() {
        let mt_val = lua_state
            .vm_mut()
            .object_pool
            .get_table(table_id)?
            .get_metatable()?;
        
        let mt_table_id = mt_val.as_table_id()?;
        let vm = lua_state.vm_mut();
        let key = vm.create_string("__call");
        let mt = vm.object_pool.get_table(mt_table_id)?;
        return mt.raw_get(&key);
    }
    
    None
}

/// Call a C function and handle results  
/// Similar to Lua's precallC - much simpler than our initial attempt
/// Lua 的做法：C 函数直接在当前栈上执行，返回结果数量
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
    let c_func: CFunction = if func.is_cfunction() {
        // Light C function - extract directly from value
        unsafe {
            let func_ptr = func.value_.f as usize;
            std::mem::transmute(func_ptr)
        }
    } else if let Some(func_id) = func.as_function_id() {
        // GC function - need to get from object pool
        let gc_func = lua_state
            .vm_mut()
            .object_pool
            .get_function(func_id)
            .ok_or(LuaError::RuntimeError)?;

        gc_func
            .c_function()
            .ok_or_else(|| lua_state.error("Not a C function".to_string()))?
    } else {
        return Err(lua_state.error("Not a callable value".to_string()));
    };

    // Lua 的做法很简单：
    // 1. Push 一个临时 CallInfo (用于 C 函数访问参数)
    // 2. 调用 C 函数
    // 3. 调用 luaD_poscall 处理返回值
    // 我们简化版本：
    let call_base = func_idx + 1;

    // Push temporary frame for C function with nresults
    lua_state.push_frame(func, call_base, nargs, nresults)?;

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
    // This is critical: old values remain in stack array but are "hidden"
    // This preserves caller's local variables which live below this top
    lua_state.set_top(new_top);

    // Update current frame's top limit
    if let Some(frame) = lua_state.current_frame_mut() {
        frame.top = new_top;
    }

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
    // CRITICAL: Sync stack_top with frame.top before reading arguments
    // Same as handle_call - we need current top to calculate variable args
    if let Some(frame) = lua_state.current_frame() {
        let frame_top = frame.top;
        lua_state.set_top(frame_top);
    }

    let nargs = if b == 0 {
        // Variable args: use current frame's top
        let func_idx = base + a;
        let first_arg = func_idx + 1;

        let frame_top = if let Some(frame) = lua_state.current_frame() {
            frame.top
        } else {
            lua_state.get_top() // Use logical stack top
        };

        if frame_top > first_arg {
            frame_top - first_arg
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
    if let Some(new_func_id) = func.as_function_id() {
        let new_func = lua_state
            .vm_mut()
            .object_pool
            .get_function(new_func_id)
            .ok_or(LuaError::RuntimeError)?;

        if new_func.is_lua_function() {
            // Move arguments to current frame base
            let mut args = Vec::with_capacity(nargs);
            for i in 0..nargs {
                let src_idx = func_idx + 1 + i;
                if let Some(arg) = lua_state.stack_get(src_idx) {
                    args.push(arg);
                }
            }

            for (i, arg) in args.into_iter().enumerate() {
                let dst_idx = base + i;
                lua_state.stack_set(dst_idx, arg)?;
            }

            // Replace function at base-1 (where current function is)
            lua_state.stack_set(base - 1, func)?;

            // CRITICAL: Update current frame's func field so main loop loads new chunk
            let current_frame_idx = lua_state.call_depth() - 1;
            lua_state.set_frame_func(current_frame_idx, func);

            // Reset PC to 0 to start executing the new function from beginning
            lua_state.set_frame_pc(current_frame_idx, 0);

            // Return FrameAction::TailCall - main loop will load new chunk and continue
            Ok(FrameAction::TailCall)
        } else if new_func.is_c_function() {
            // C function tail call: execute directly and continue
            // Lua \u4e5f\u662f\u8fd9\u6837\u505a\u7684\uff1aluaD_pretailcall \u8c03\u7528 precallC
            call_c_function(lua_state, func_idx, nargs, -1)?;

            // Move results to current frame base (for return)
            // Use stack_top to determine actual number of results
            let result_top = lua_state.get_top();
            let nresults = if result_top > func_idx {
                result_top - func_idx
            } else {
                0
            };

            for i in 0..nresults {
                let result = lua_state.stack_get(func_idx + i).unwrap_or(LuaValue::nil());
                lua_state.stack_set(base + i, result)?;
            }

            // CRITICAL: Update frame top after moving results
            // The next RETURN instruction will use frame.top to determine how many values to return
            let new_top = base + nresults;
            lua_state.set_top(new_top);
            if let Some(frame) = lua_state.current_frame_mut() {
                frame.top = new_top;
            }

            // C function done, just continue (it's like a return for tail call)
            Ok(FrameAction::Continue)
        } else if func.is_cfunction() {
            // Light C function tail call
            call_c_function(lua_state, func_idx, nargs, -1)?;

            // Move results to current frame base
            // Use stack_top to determine actual number of results
            let result_top = lua_state.get_top();
            let nresults = if result_top > func_idx {
                result_top - func_idx
            } else {
                0
            };

            for i in 0..nresults {
                let result = lua_state.stack_get(func_idx + i).unwrap_or(LuaValue::nil());
                lua_state.stack_set(base + i, result)?;
            }

            Ok(FrameAction::Continue)
        } else {
            Err(lua_state.error("TAILCALL: unknown function type".to_string()))
        }
    } else if func.is_cfunction() {
        // Light C function tail call (direct, not from object pool)
        call_c_function_tailcall(lua_state, func_idx, nargs, base)?;
        Ok(FrameAction::Continue)
    } else {
        Err(lua_state.error("TAILCALL: attempt to call a non-function".to_string()))
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
    let c_func: CFunction = if func.is_cfunction() {
        unsafe {
            let func_ptr = func.value_.f as usize;
            std::mem::transmute(func_ptr)
        }
    } else if let Some(func_id) = func.as_function_id() {
        let gc_func = lua_state
            .vm_mut()
            .object_pool
            .get_function(func_id)
            .ok_or(LuaError::RuntimeError)?;
        gc_func
            .c_function()
            .ok_or_else(|| lua_state.error("Not a C function".to_string()))?
    } else {
        return Err(lua_state.error("Not a callable value".to_string()));
    };

    let call_base = func_idx + 1;

    // Push temporary frame for C function
    // Tail call inherits caller's nresults (-1 for multi-return)
    lua_state.push_frame(func, call_base, nargs, -1)?;

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
    lua_state.set_top(new_top);
    if let Some(frame) = lua_state.current_frame_mut() {
        frame.top = new_top;
    }

    Ok(())
}
