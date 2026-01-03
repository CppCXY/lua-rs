/*----------------------------------------------------------------------
  Return Instructions Handler - Lua 5.5 Style

  Based on Lua 5.5.0 lvm.c:1763-1827 and ldo.c:605-614

  Implements:
  - OP_RETURN: Generic return with N values
  - OP_RETURN0: Optimized no-value return
  - OP_RETURN1: Optimized single-value return

  Key operations:
  1. Move return values to caller's expected position
  2. Close upvalues if needed (k flag)
  3. Adjust for vararg functions
  4. Restore previous CallInfo
  5. Set top pointer correctly
----------------------------------------------------------------------*/

use crate::{
    lua_value::LuaValue,
    lua_vm::{LuaResult, LuaState},
};

use super::call::FrameAction;

/// Handle OP_RETURN instruction
/// Returns N values from R[A] to R[A+B-2]
///
/// Based on lvm.c:1763-1783
pub fn handle_return(
    lua_state: &mut LuaState,
    stack_ptr: *mut LuaValue,
    base: usize,
    frame_idx: usize,
    a: usize,
    b: usize,
    c: usize,
    k: bool,
) -> LuaResult<FrameAction> {
    // n = number of results (B-1), if B=0 then return all values to top
    let mut nres = if b == 0 {
        // Return all values from R[A] to top
        let top = lua_state.stack_len();
        let ra_pos = base + a;
        if top > ra_pos { top - ra_pos } else { 0 }
    } else {
        b - 1
    };

    // Close upvalues if k flag is set
    if k {
        close_upvalues(lua_state, base)?;
    }

    // Adjust for vararg functions (nparams1 = C)
    // In vararg functions, we need to move the function pointer back
    if c > 0 {
        // This adjusts for extra arguments that were pushed
        // TODO: Implement vararg adjustment when vararg system is complete
    }

    // Move return values to correct position
    // Caller expects results at ci->func (function slot), which is base-1
    let call_info = lua_state.get_call_info(frame_idx);
    let func_pos = if call_info.base > 0 {
        call_info.base - 1 // Function is at base-1
    } else {
        0
    };
    let wanted_results = if call_info.nresults < 0 {
        nres // LUA_MULTRET: return all results
    } else {
        call_info.nresults as usize
    };

    // Copy results from R[A]..R[A+nres-1] to func_pos..func_pos+nres-1
    unsafe {
        for i in 0..nres {
            let src = stack_ptr.add(base + a + i);
            let dst = lua_state.stack_ptr_mut().add(func_pos + i);
            *dst = *src;
        }
    }

    // Adjust top to point after the last result
    lua_state.set_top(func_pos + nres);

    // Fill with nil if caller wants more results than we have
    if wanted_results > nres {
        for i in nres..wanted_results {
            lua_state.stack_set(func_pos + i, LuaValue::nil())?;
        }
        lua_state.set_top(func_pos + wanted_results);
        nres = wanted_results;
    }

    // Pop current call frame
    lua_state.pop_call_frame();

    // Update caller frame's top to reflect the actual number of results
    // This is crucial - the caller needs to know where its stack values are
    if let Some(caller_frame) = lua_state.current_frame_mut() {
        caller_frame.top = func_pos + nres;
    }

    // Check if this was the top-level frame
    if lua_state.call_depth() == 0 {
        // No more frames, execution complete
        return Ok(FrameAction::Return);
    }

    // Continue execution in caller's frame
    Ok(FrameAction::Continue)
}

/// Handle OP_RETURN0 instruction (optimized for no return values)
///
/// Based on lvm.c:1784-1800
pub fn handle_return0(lua_state: &mut LuaState, frame_idx: usize) -> LuaResult<FrameAction> {
    // Get caller's expected results
    let call_info = lua_state.get_call_info(frame_idx);
    let func_pos = if call_info.base > 0 {
        call_info.base - 1
    } else {
        0
    };
    let wanted_results = if call_info.nresults < 0 {
        0 // LUA_MULTRET for return0 means 0
    } else {
        call_info.nresults as usize
    };

    // Fill with nil if caller expects results
    if wanted_results > 0 {
        for i in 0..wanted_results {
            lua_state.stack_set(func_pos + i, LuaValue::nil())?;
        }
        lua_state.set_top(func_pos + wanted_results);
    } else {
        lua_state.set_top(func_pos);
    }

    // Pop current call frame
    lua_state.pop_call_frame();

    // Update caller frame's top to reflect the actual number of results
    // This is crucial - the caller needs to know where its stack values are
    if let Some(caller_frame) = lua_state.current_frame_mut() {
        caller_frame.top = func_pos + wanted_results;
    }

    // Check if this was the top-level frame
    if lua_state.call_depth() == 0 {
        return Ok(FrameAction::Return);
    }

    Ok(FrameAction::Continue)
}

/// Handle OP_RETURN1 instruction (optimized for single return value)
///
/// Based on lvm.c:1801-1827
pub fn handle_return1(
    lua_state: &mut LuaState,
    stack_ptr: *mut LuaValue,
    base: usize,
    frame_idx: usize,
    a: usize,
) -> LuaResult<FrameAction> {
    // Get the single return value
    let return_val = unsafe {
        let ra = stack_ptr.add(base + a);
        *ra
    };

    // Get caller's expected results
    let call_info = lua_state.get_call_info(frame_idx);
    let func_pos = if call_info.base > 0 {
        call_info.base - 1
    } else {
        0
    };
    let wanted_results = if call_info.nresults < 0 {
        1 // LUA_MULTRET for return1 means 1
    } else {
        call_info.nresults as usize
    };

    if wanted_results == 0 {
        // Caller doesn't want any results
        lua_state.set_top(func_pos);
    } else {
        // Set the first result
        lua_state.stack_set(func_pos, return_val)?;

        // Fill remaining with nil if caller wants more
        for i in 1..wanted_results {
            lua_state.stack_set(func_pos + i, LuaValue::nil())?;
        }
        lua_state.set_top(func_pos + wanted_results);
    }

    // Pop current call frame
    lua_state.pop_call_frame();

    // Update caller frame's top to reflect the actual number of results
    // This is crucial - the caller needs to know where its stack values are
    if let Some(caller_frame) = lua_state.current_frame_mut() {
        caller_frame.top = func_pos + wanted_results;
    }

    // Check if this was the top-level frame
    if lua_state.call_depth() == 0 {
        return Ok(FrameAction::Return);
    }

    Ok(FrameAction::Continue)
}

/// Close all open upvalues >= level
/// Based on lfunc.c luaF_close
fn close_upvalues(lua_state: &mut LuaState, level: usize) -> LuaResult<()> {
    // Get all open upvalues in the current frame
    let upvalues_to_close: Vec<_> = lua_state
        .vm_mut()
        .object_pool
        .iter_upvalues()
        .filter_map(|(id, upval)| {
            if let Some(stack_idx) = upval.get_stack_index() {
                if stack_idx >= level {
                    Some((id, stack_idx))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Close each upvalue
    for (upval_id, stack_idx) in upvalues_to_close {
        // Get the value from the stack
        let value = lua_state.stack_get(stack_idx).unwrap_or(LuaValue::nil());

        // Close the upvalue (move value from stack to upvalue storage)
        if let Some(upval) = lua_state.vm_mut().object_pool.get_upvalue_mut(upval_id) {
            unsafe {
                upval.close_with_value(value);
            }
        }
    }

    Ok(())
}
