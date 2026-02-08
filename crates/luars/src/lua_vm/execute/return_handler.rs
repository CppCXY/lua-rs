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

/// Handle OP_RETURN instruction
/// Returns N values from R[A] to R[A+B-2]
///
/// Based on lvm.c:1763-1783
#[inline]
pub fn handle_return(
    lua_state: &mut LuaState,
    base: usize,
    frame_idx: usize,
    a: usize,
    b: usize,
    _c: usize,
    k: bool,
) -> LuaResult<()> {
    // n = number of results (B-1), if B=0 then return all values to top
    let mut nres = if b == 0 {
        // Return all values from R[A] to logical top (L->top.p)
        let top = lua_state.get_top();
        let ra_pos = base + a;
        if top > ra_pos { top - ra_pos } else { 0 }
    } else {
        b - 1
    };

    // Close upvalues if k flag is set
    if k {
        lua_state.close_upvalues(base);
    }

    // Adjust for vararg functions (nparams1 = C)
    // Lua 5.5 adjusts ci->func.p here: if (nparams1) ci->func.p -= ci->u.l.nextraargs + nparams1;
    // This reverses the shift done by buildhiddenargs (ci->func.p += totalargs + 1)
    // In our implementation, we use func_offset to track the original func position,
    // so we don't need explicit adjustment here. The calculation below already handles it:
    // func_pos = base - func_offset
    // where func_offset was set by buildhiddenargs to (new_base - original_func_pos)

    // Move return values to correct position
    // After buildhiddenargs, we need to use func_offset to find original position
    let call_info = lua_state.get_call_info(frame_idx);
    let func_pos = call_info.base - call_info.func_offset;

    let wanted_results = if call_info.nresults < 0 {
        nres // LUA_MULTRET: return all results
    } else {
        call_info.nresults as usize
    };

    // Copy results from R[A]..R[A+nres-1] to func_pos..func_pos+nres-1
    let stack = lua_state.stack_mut();
    for i in 0..nres {
        let src_val = stack[base + a + i];
        stack[func_pos + i] = src_val;
    }

    // Fill with nil if caller wants more results than we have
    if wanted_results > nres {
        for i in nres..wanted_results {
            stack[func_pos + i] = LuaValue::nil();
        }
        nres = wanted_results;
    }

    let new_top = func_pos + nres;

    // Pop current call frame
    lua_state.pop_call_frame();

    // Update logical stack top (no resize check needed — returning to caller's frame)
    lua_state.set_top_raw(new_top);

    Ok(())
}

/// Handle OP_RETURN0 instruction (optimized for no return values)
///
/// Based on lvm.c:1784-1800
#[inline(always)]
pub fn handle_return0(lua_state: &mut LuaState, frame_idx: usize) -> LuaResult<()> {
    // Get caller's expected results
    let call_info = lua_state.get_call_info(frame_idx);
    let func_pos = call_info.base - call_info.func_offset;
    let wanted_results = if call_info.nresults < 0 {
        0 // LUA_MULTRET for return0 means 0
    } else {
        call_info.nresults as usize
    };

    let new_top = func_pos + wanted_results;

    // Fill with nil if caller expects results
    if wanted_results > 0 {
        let stack = lua_state.stack_mut();
        for i in 0..wanted_results {
            stack[func_pos + i] = LuaValue::nil();
        }
    }

    lua_state.pop_call_frame();
    lua_state.set_top_raw(new_top);
    Ok(())
}

/// Handle OP_RETURN1 instruction (optimized for single return value)
///
/// Based on lvm.c:1801-1827
#[inline(always)]
pub fn handle_return1(
    lua_state: &mut LuaState,
    base: usize,
    frame_idx: usize,
    a: usize,
) -> LuaResult<()> {
    // Get call info first
    let call_info = lua_state.get_call_info(frame_idx);
    let func_pos = call_info.base - call_info.func_offset;
    let nresults = call_info.nresults;

    let stack = lua_state.stack_mut();
    let return_val = stack[base + a];

    let new_top = if nresults == 0 {
        func_pos
    } else if nresults == 1 || nresults == -1 {
        // Most common: caller wants exactly 1 result (or MULTRET with 1 value)
        stack[func_pos] = return_val;
        func_pos + 1
    } else {
        // Caller wants N > 1 results — place first + fill rest with nil
        stack[func_pos] = return_val;
        let wanted = nresults as usize;
        for i in 1..wanted {
            stack[func_pos + i] = LuaValue::nil();
        }
        func_pos + wanted
    };

    lua_state.pop_call_frame();
    lua_state.set_top_raw(new_top);
    Ok(())
}
