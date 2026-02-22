/*----------------------------------------------------------------------
  Table Operations Module - Extracted from main execution loop

  This module contains non-hot-path table instructions:
  - GetTable, SetTable
  - GetI, SetI
  - GetField, SetField
  - Self_
  - NewTable

  These operations involve complex logic and metamethod calls,
  so extracting them reduces main loop size without hurting performance.
----------------------------------------------------------------------*/

use crate::{
    lua_value::LuaValue,
    lua_vm::{Instruction, LuaError, LuaResult, LuaState},
};

use super::helper;

/// GETTABLE: R[A] := R[B][R[C]]
pub fn exec_gettable(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    use super::helper::{pivalue, pttisinteger};

    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;

    // Read operands FIRST before any stack manipulation
    let rb = lua_state.stack_mut()[base + b];
    let rc = lua_state.stack_mut()[base + c];

    let result = if let Some(table) = rb.as_table_mut() {
        // Fast path for table - OPTIMIZED: Direct pointer access
        let direct_result = unsafe {
            if pttisinteger(&rc as *const LuaValue) {
                let key = pivalue(&rc as *const LuaValue);
                table.raw_geti(key)
            } else {
                table.raw_get(&rc)
            }
        };

        if direct_result.is_some() {
            direct_result
        } else {
            // Key not found in table, try __index metamethod
            let call_info_top = lua_state.get_call_info(frame_idx).top;
            lua_state.set_top(call_info_top)?;
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            match helper::lookup_from_metatable(lua_state, &rb, &rc) {
                Ok(result) => {
                    let new_base = lua_state.get_frame_base(frame_idx);
                    if new_base != base {
                        return Err(lua_state.error("base changed in GETTABLE".to_string()));
                    }
                    result
                }
                Err(LuaError::Yield) => {
                    let ci = lua_state.get_call_info_mut(frame_idx);
                    ci.pending_finish_get = a as i32;
                    ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                    return Err(LuaError::Yield);
                }
                Err(e) => return Err(e),
            }
        }
    } else {
        // Not a table, try __index metamethod
        let call_info_top = lua_state.get_call_info(frame_idx).top;
        lua_state.set_top(call_info_top)?;
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        match helper::lookup_from_metatable(lua_state, &rb, &rc) {
            Ok(result) => {
                let new_base = lua_state.get_frame_base(frame_idx);
                if new_base != base {
                    return Err(lua_state.error("base changed in GETTABLE".to_string()));
                }
                result
            }
            Err(LuaError::Yield) => {
                let ci = lua_state.get_call_info_mut(frame_idx);
                ci.pending_finish_get = a as i32;
                ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                return Err(LuaError::Yield);
            }
            Err(e) => return Err(e),
        }
    };

    lua_state.stack_mut()[base + a] = result.unwrap_or(LuaValue::nil());
    Ok(())
}

/// SETTABLE: R[A][R[B]] := RK(C)
pub fn exec_settable(
    lua_state: &mut LuaState,
    instr: Instruction,
    constants: &[LuaValue],
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let k = instr.get_k();

    let stack = lua_state.stack();
    let ra = stack[base + a];
    let rb = stack[base + b];

    // Check for nil key - "table index is nil"
    if rb.is_nil() {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error("table index is nil".to_string()));
    }
    // Check for NaN key - "table index is NaN"
    if rb.ttisfloat() && rb.fltvalue().is_nan() {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error("table index is NaN".to_string()));
    }

    let val = if k {
        if c >= constants.len() {
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            return Err(lua_state.error("SETTABLE: invalid constant".to_string()));
        }
        constants[c]
    } else {
        stack[base + c]
    };

    // OPTIMIZED: Fast path - check if we can avoid metamethod call
    if let Some(table) = ra.as_table() {
        if !table.has_metatable() {
            // No metatable - directly set, no __newindex check needed
            // NOTE: No check_gc() here - matches C Lua 5.5's OP_SETTABLE which
            // does NOT have checkGC. Running GC here would scan the stack and
            // mark the value register alive, preventing weak table clearing.
            lua_state.raw_set(&ra, rb, val);
            return Ok(());
        }
        // Has metatable: if key already exists with non-nil value,
        // __newindex is NOT consulted (Lua semantics). Try a fast check.
        if let Some(existing) = table.impl_table.raw_get(&rb)
            && !existing.is_nil()
        {
            lua_state.raw_set(&ra, rb, val);
            return Ok(());
        }
    }

    //  Ensure stack_top protects call_info.top before calling metamethod
    // call_info.top should NOT be modified - it's set once at function call
    // See Lua 5.5's savestate macro: L->top.p = ci->top.p
    let call_info_top = lua_state.get_call_info(frame_idx).top;
    if lua_state.get_top() < call_info_top {
        lua_state.set_top(call_info_top)?;
    }

    // Slow path: has __newindex or not a table
    lua_state.set_frame_pc(frame_idx, *pc as u32);
    match helper::finishset(lua_state, &ra, &rb, val) {
        Ok(_) => {}
        Err(LuaError::Yield) => {
            let ci = lua_state.get_call_info_mut(frame_idx);
            ci.pending_finish_get = -2;
            ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
            return Err(LuaError::Yield);
        }
        Err(e) => return Err(e),
    }

    //  Restore top after metamethod call
    // The metamethod may have changed stack_top, so we need to reset it
    lua_state.set_top(call_info_top)?;

    // Verify base hasn't changed
    let new_base = lua_state.get_frame_base(frame_idx);
    if new_base != base {
        return Err(lua_state.error("base changed in SETTABLE".to_string()));
    }

    Ok(())
}

/// GETI: R[A] := R[B][C] (integer key)
pub fn exec_geti(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;

    let stack = lua_state.stack_mut();
    let rb = stack[base + b];

    let result = if let Some(table) = rb.as_table_mut() {
        // Fast path: try direct table access - OPTIMIZED: Direct pointer
        let direct_result = table.raw_geti(c as i64);

        if direct_result.is_some() {
            direct_result
        } else {
            // Key not found, try __index metamethod
            let call_info_top = lua_state.get_call_info(frame_idx).top;
            lua_state.set_top(call_info_top)?;
            let key = LuaValue::integer(c as i64);
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            match helper::lookup_from_metatable(lua_state, &rb, &key) {
                Ok(result) => {
                    let new_base = lua_state.get_frame_base(frame_idx);
                    if new_base != base {
                        return Err(lua_state.error("base changed in GETI".to_string()));
                    }
                    result
                }
                Err(LuaError::Yield) => {
                    let ci = lua_state.get_call_info_mut(frame_idx);
                    ci.pending_finish_get = a as i32;
                    ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                    return Err(LuaError::Yield);
                }
                Err(e) => return Err(e),
            }
        }
    } else {
        // Not a table, try __index metamethod
        let call_info_top = lua_state.get_call_info(frame_idx).top;
        lua_state.set_top(call_info_top)?;
        let key = LuaValue::integer(c as i64);
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        match helper::lookup_from_metatable(lua_state, &rb, &key) {
            Ok(result) => {
                let new_base = lua_state.get_frame_base(frame_idx);
                if new_base != base {
                    return Err(lua_state.error("base changed in GETI".to_string()));
                }
                result
            }
            Err(LuaError::Yield) => {
                let ci = lua_state.get_call_info_mut(frame_idx);
                ci.pending_finish_get = a as i32;
                ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                return Err(LuaError::Yield);
            }
            Err(e) => return Err(e),
        }
    };

    let stack = lua_state.stack_mut();
    stack[base + a] = result.unwrap_or(LuaValue::nil());
    Ok(())
}

/// SETI: R[A][B] := RK(C) (integer key)
pub fn exec_seti(
    lua_state: &mut LuaState,
    instr: Instruction,
    constants: &[LuaValue],
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let k = instr.get_k();

    //  Ensure stack_top protects call_info.top
    let call_info_top = lua_state.get_call_info(frame_idx).top;
    if lua_state.get_top() < call_info_top {
        lua_state.set_top(call_info_top)?;
    }

    let stack = lua_state.stack();
    let ra = stack[base + a];

    // Get value (RK: register or constant)
    let value = if k {
        if c >= constants.len() {
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            return Err(lua_state.error("SETI: invalid constant".to_string()));
        }
        constants[c]
    } else {
        stack[base + c]
    };

    // OPTIMIZED: Fast path - check if we need metamethod call
    if let Some(table) = ra.as_table() {
        if !table.has_metatable() {
            // Fast path: no __newindex, directly set
            // NOTE: No check_gc() here - matches C Lua 5.5's OP_SETI which
            // does NOT have checkGC. Running GC here would scan the stack and
            // mark the value register alive, preventing weak table clearing.
            lua_state.raw_seti(&ra, b as i64, value);
        } else {
            // Has metatable: check if key already exists with non-nil value.
            // If yes, __newindex is NOT consulted (Lua semantics).
            let key_int = b as i64;
            let existing = table.impl_table.get_int(key_int);
            if existing.is_some_and(|v| !v.is_nil()) {
                lua_state.raw_seti(&ra, key_int, value);
            } else {
                // Slow path: key doesn't exist or is nil → check __newindex
                let key = LuaValue::integer(b as i64);
                lua_state.set_frame_pc(frame_idx, *pc as u32);
                match helper::finishset(lua_state, &ra, &key, value) {
                    Ok(_) => {}
                    Err(LuaError::Yield) => {
                        let ci = lua_state.get_call_info_mut(frame_idx);
                        ci.pending_finish_get = -2;
                        ci.call_status |=
                            crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                        return Err(LuaError::Yield);
                    }
                    Err(e) => return Err(e),
                }
                // Restore top after metamethod call
                lua_state.set_top(call_info_top)?;
                let new_base = lua_state.get_frame_base(frame_idx);
                if new_base != base {
                    return Err(lua_state.error("base changed in SETI".to_string()));
                }
            }
        }
    } else {
        // Not a table, use __newindex metamethod
        let key = LuaValue::integer(b as i64);
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        match helper::finishset(lua_state, &ra, &key, value) {
            Ok(_) => {}
            Err(LuaError::Yield) => {
                let ci = lua_state.get_call_info_mut(frame_idx);
                ci.pending_finish_get = -2;
                ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                return Err(LuaError::Yield);
            }
            Err(e) => return Err(e),
        }
        // Restore top after metamethod call
        lua_state.set_top(call_info_top)?;
        let new_base = lua_state.get_frame_base(frame_idx);
        if new_base != base {
            return Err(lua_state.error("base changed in SETI".to_string()));
        }
    }

    Ok(())
}

/// GETFIELD: R[A] := R[B][K[C]:string]
pub fn exec_getfield(
    lua_state: &mut LuaState,
    instr: Instruction,
    constants: &[LuaValue],
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;

    if c >= constants.len() {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error(format!("GETFIELD: invalid constant index {}", c)));
    }

    let stack = lua_state.stack_mut();
    let rb = stack[base + b];
    let key = &constants[c];

    let result = if let Some(table) = rb.as_table_mut() {
        // Fast path: try direct table access - OPTIMIZED: Direct pointer
        let direct_result = table.raw_get(key);

        if direct_result.is_some() {
            direct_result
        } else {
            // Key not found, try __index metamethod
            let call_info_top = lua_state.get_call_info(frame_idx).top;
            lua_state.set_top(call_info_top)?;
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            match helper::lookup_from_metatable(lua_state, &rb, key) {
                Ok(result) => {
                    let new_base = lua_state.get_frame_base(frame_idx);
                    if new_base != base {
                        return Err(lua_state.error("base changed in GETFIELD".to_string()));
                    }
                    result
                }
                Err(LuaError::Yield) => {
                    let ci = lua_state.get_call_info_mut(frame_idx);
                    ci.pending_finish_get = a as i32;
                    ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                    return Err(LuaError::Yield);
                }
                Err(e) => return Err(e),
            }
        }
    } else {
        // Not a table, try metatable lookup
        let call_info_top = lua_state.get_call_info(frame_idx).top;
        lua_state.set_top(call_info_top)?;
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        match helper::lookup_from_metatable(lua_state, &rb, key) {
            Ok(result) => {
                let new_base = lua_state.get_frame_base(frame_idx);
                if new_base != base {
                    return Err(lua_state.error("base changed in GETFIELD".to_string()));
                }
                result
            }
            Err(LuaError::Yield) => {
                let ci = lua_state.get_call_info_mut(frame_idx);
                ci.pending_finish_get = a as i32;
                ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                return Err(LuaError::Yield);
            }
            Err(e) => return Err(e),
        }
    };

    let stack = lua_state.stack_mut();
    stack[base + a] = result.unwrap_or(LuaValue::nil());
    Ok(())
}

/// SETFIELD: R[A][K[B]:string] := RK(C)
pub fn exec_setfield(
    lua_state: &mut LuaState,
    instr: Instruction,
    constants: &[LuaValue],
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let k = instr.get_k();

    if b >= constants.len() {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error(format!("SETFIELD: invalid constant index {}", b)));
    }

    let stack = lua_state.stack_mut();
    let ra = stack[base + a];
    let key = constants[b];

    // Get value (RK: register or constant)
    let value = if k {
        if c >= constants.len() {
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            return Err(lua_state.error("SETFIELD: invalid constant".to_string()));
        }
        constants[c]
    } else {
        stack[base + c]
    };

    // OPTIMIZED: Fast path - check if we can avoid metamethod call
    if let Some(table) = ra.as_table() {
        if !table.has_metatable() {
            // No metatable - directly set, no __newindex check needed
            // NOTE: No check_gc() here - matches C Lua 5.5's OP_SETFIELD which
            // does NOT have checkGC. Running GC here would scan the stack and
            // mark the value register alive, preventing weak table clearing.
            lua_state.raw_set(&ra, key, value);
            return Ok(());
        }
        // Has metatable: if key already exists with non-nil value,
        // __newindex is NOT consulted (Lua semantics). Try fast path.
        if let Some(existing) = table.impl_table.raw_get(&key)
            && !existing.is_nil()
        {
            lua_state.raw_set(&ra, key, value);
            return Ok(());
        }
    }

    // savestate: L->top = ci->top before metamethod call
    let call_info_top = lua_state.get_call_info(frame_idx).top;
    if lua_state.get_top() < call_info_top {
        lua_state.set_top(call_info_top)?;
    }

    // Slow path: has __newindex or not a table
    lua_state.set_frame_pc(frame_idx, *pc as u32);
    match helper::finishset(lua_state, &ra, &key, value) {
        Ok(_) => {}
        Err(LuaError::Yield) => {
            let ci = lua_state.get_call_info_mut(frame_idx);
            ci.pending_finish_get = -2;
            ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
            return Err(LuaError::Yield);
        }
        Err(e) => return Err(e),
    }

    // Restore top after metamethod call
    lua_state.set_top(call_info_top)?;

    let new_base = lua_state.get_frame_base(frame_idx);
    if new_base != base {
        return Err(lua_state.error("base changed in SETFIELD".to_string()));
    }

    Ok(())
}

/// SELF: R[A+1] := R[B]; R[A] := R[B][K[C]:string]
pub fn exec_self(
    lua_state: &mut LuaState,
    instr: Instruction,
    constants: &[LuaValue],
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;

    if c >= constants.len() {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error(format!("SELF: invalid constant index {}", c)));
    }

    let stack = lua_state.stack_mut();
    let rb = stack[base + b];

    // R[A+1] := R[B] (save object)
    stack[base + a + 1] = rb;

    // R[A] := R[B][K[C]] (get method)
    let key = &constants[c];

    // OPTIMIZED: Try raw_get first if it's a table
    let fast_result = if let Some(table) = rb.as_table_mut() {
        table.raw_get(key)
    } else {
        None
    };

    if let Some(val) = fast_result {
        let stack = lua_state.stack_mut();
        stack[base + a] = val;
    } else {
        // Key not found in table OR not a table: try __index metamethod
        let call_info_top = lua_state.get_call_info(frame_idx).top;
        lua_state.set_top(call_info_top)?;
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        match helper::lookup_from_metatable(lua_state, &rb, key) {
            Ok(result) => {
                let new_base = lua_state.get_frame_base(frame_idx);
                if new_base != base {
                    return Err(lua_state.error("base changed in SELF".to_string()));
                }
                let stack = lua_state.stack_mut();
                stack[base + a] = result.unwrap_or(LuaValue::nil());
            }
            Err(LuaError::Yield) => {
                let ci = lua_state.get_call_info_mut(frame_idx);
                ci.pending_finish_get = a as i32;
                ci.call_status |= crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                return Err(LuaError::Yield);
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}
