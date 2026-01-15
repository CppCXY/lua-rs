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
    GcId,
    lua_value::LuaValue,
    lua_vm::{Instruction, LuaResult, LuaState},
};

use super::helper;

/// GETTABLE: R[A] := R[B][R[C]]
#[inline(always)]
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

    // CRITICAL: Update frame.top before potential metamethod call
    let write_pos = base + a;
    let call_info = lua_state.get_call_info_mut(frame_idx);
    if write_pos + 1 > call_info.top {
        call_info.top = write_pos + 1;
        lua_state.set_top(write_pos + 1);
    }

    let rb = lua_state.stack_mut()[base + b];
    let rc = lua_state.stack_mut()[base + c];

    let result = if let Some(table) = rb.as_table_mut() {
        // Fast path for table - OPTIMIZED: Direct pointer access
        let direct_result = unsafe {
            if pttisinteger(&rc as *const LuaValue) {
                let key = pivalue(&rc as *const LuaValue);
                table.get_int(key)
            } else {
                table.raw_get(&rc)
            }
        };

        if direct_result.is_some() {
            direct_result
        } else {
            // Key not found in table, try __index metamethod
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            let result = helper::lookup_from_metatable(lua_state, &rb, &rc);
            // Restore base after potential frame change
            let new_base = lua_state.get_frame_base(frame_idx);
            if new_base != base {
                return Err(lua_state.error("base changed in GETTABLE".to_string()));
            }
            result
        }
    } else {
        // Not a table, try __index metamethod with Protect pattern
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        let result = helper::lookup_from_metatable(lua_state, &rb, &rc);
        // Check base consistency
        let new_base = lua_state.get_frame_base(frame_idx);
        if new_base != base {
            return Err(lua_state.error("base changed in GETTABLE".to_string()));
        }
        result
    };

    lua_state.stack_mut()[base + a] = result.unwrap_or(LuaValue::nil());
    Ok(())
}

/// SETTABLE: R[A][R[B]] := RK(C)
#[inline(always)]
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
    if let Some(table) = ra.as_table_mut() {
        if !table.has_metatable() {
            // Fast path: no metatable, directly set
            table.raw_set(&rb, val);

            // CRITICAL: GC write barrier
            // When modifying a BLACK table, we must call barrier_back to re-gray it
            // This ensures weak tables are re-traversed in subsequent GC cycles
            let table_gc_id = GcId::TableId(ra.hvalue());
            lua_state.gc_barrier_back(table_gc_id);

            lua_state.check_gc()?;
            return Ok(());
        }
    }

    // CRITICAL: Update frame.top to protect all registers before calling metamethod
    let max_reg = a.max(b).max(c) + 1;
    let required_top = base + max_reg;
    let call_info = lua_state.get_call_info_mut(frame_idx);
    if required_top > call_info.top {
        call_info.top = required_top;
        lua_state.set_top(required_top);
    }

    // Slow path: has __newindex or not a table
    lua_state.set_frame_pc(frame_idx, *pc as u32);
    helper::store_to_metatable(lua_state, &ra, &rb, val)?;

    // Verify base hasn't changed
    let new_base = lua_state.get_frame_base(frame_idx);
    if new_base != base {
        return Err(lua_state.error("base changed in SETTABLE".to_string()));
    }

    Ok(())
}

/// GETI: R[A] := R[B][C] (integer key)
#[inline(always)]
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
        let direct_result = table.get_int(c as i64);

        if direct_result.is_some() {
            direct_result
        } else {
            // Key not found, try __index metamethod
            let key = LuaValue::integer(c as i64);
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            let result = helper::lookup_from_metatable(lua_state, &rb, &key);
            let new_base = lua_state.get_frame_base(frame_idx);
            if new_base != base {
                return Err(lua_state.error("base changed in GETI".to_string()));
            }
            result
        }
    } else {
        // Not a table, try __index metamethod
        let key = LuaValue::integer(c as i64);
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        let result = helper::lookup_from_metatable(lua_state, &rb, &key);
        let new_base = lua_state.get_frame_base(frame_idx);
        if new_base != base {
            return Err(lua_state.error("base changed in GETI".to_string()));
        }
        result
    };

    // Update frame.top FIRST if we're writing beyond current top
    let write_pos = base + a;
    let call_info = lua_state.get_call_info_mut(frame_idx);
    if write_pos >= call_info.top {
        call_info.top = write_pos + 1;
        lua_state.set_top(write_pos + 1);
    }

    let stack = lua_state.stack_mut();
    stack[base + a] = result.unwrap_or(LuaValue::nil());
    Ok(())
}

/// SETI: R[A][B] := RK(C) (integer key)
#[inline(always)]
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

    // CRITICAL: Update frame.top to protect all registers
    let max_reg = a.max(c) + 1;
    let required_top = base + max_reg;
    let call_info = lua_state.get_call_info_mut(frame_idx);
    if required_top > call_info.top {
        call_info.top = required_top;
        lua_state.set_top(required_top);
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
    if let Some(table) = ra.as_table_mut() {
        if !table.has_metatable() {
            // Fast path: no __newindex, directly set
            table.set_int(b as i64, value);
            lua_state.check_gc()?;
        } else {
            // Slow path: has __newindex metamethod
            let key = LuaValue::integer(b as i64);
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            helper::store_to_metatable(lua_state, &ra, &key, value)?;
            let new_base = lua_state.get_frame_base(frame_idx);
            if new_base != base {
                return Err(lua_state.error("base changed in SETI".to_string()));
            }
        }
    } else {
        // Not a table, use __newindex metamethod
        let key = LuaValue::integer(b as i64);
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        helper::store_to_metatable(lua_state, &ra, &key, value)?;
        let new_base = lua_state.get_frame_base(frame_idx);
        if new_base != base {
            return Err(lua_state.error("base changed in SETI".to_string()));
        }
    }

    Ok(())
}

/// GETFIELD: R[A] := R[B][K[C]:string]
#[inline(always)]
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

    // CRITICAL: Update frame.top before potential metamethod call
    let write_pos = base + a;
    let call_info = lua_state.get_call_info_mut(frame_idx);
    if write_pos + 1 > call_info.top {
        call_info.top = write_pos + 1;
        lua_state.set_top(write_pos + 1);
    }

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
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            let result = helper::lookup_from_metatable(lua_state, &rb, key);
            let new_base = lua_state.get_frame_base(frame_idx);
            if new_base != base {
                return Err(lua_state.error("base changed in GETFIELD".to_string()));
            }
            result
        }
    } else {
        // Not a table, try metatable lookup
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        let result = helper::lookup_from_metatable(lua_state, &rb, key);
        let new_base = lua_state.get_frame_base(frame_idx);
        if new_base != base {
            return Err(lua_state.error("base changed in GETFIELD".to_string()));
        }
        result
    };

    let stack = lua_state.stack_mut();
    stack[base + a] = result.unwrap_or(LuaValue::nil());
    Ok(())
}

/// SETFIELD: R[A][K[B]:string] := RK(C)
#[inline(always)]
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
    if let Some(table) = ra.as_table_mut() {
        if !table.has_metatable() {
            // Fast path: no __newindex metamethod, directly set
            table.raw_set(&key, value);

            // CRITICAL: GC write barrier
            let table_gc_id = GcId::TableId(ra.hvalue());
            lua_state.gc_barrier_back(table_gc_id);

            lua_state.check_gc()?;
            return Ok(());
        }
    }

    // Slow path: has __newindex or not a table
    lua_state.set_frame_pc(frame_idx, *pc as u32);
    helper::store_to_metatable(lua_state, &ra, &key, value)?;

    let new_base = lua_state.get_frame_base(frame_idx);
    if new_base != base {
        return Err(lua_state.error("base changed in SETFIELD".to_string()));
    }

    Ok(())
}

/// SELF: R[A+1] := R[B]; R[A] := R[B][K[C]:string]
#[inline(always)]
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

    // CRITICAL: Update frame.top to cover R[A+1] (the object) and R[A] (the method)
    // We write to A+1 and A. So we need top >= base + a + 2.
    let write_top = base + a + 2;
    let call_info = lua_state.get_call_info_mut(frame_idx);
    if write_top > call_info.top {
        call_info.top = write_top;
        lua_state.set_top(write_top);
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
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        let result = helper::lookup_from_metatable(lua_state, &rb, key);
        let new_base = lua_state.get_frame_base(frame_idx);
        if new_base != base {
            return Err(lua_state.error("base changed in SELF".to_string()));
        }
        let stack = lua_state.stack_mut();
        stack[base + a] = result.unwrap_or(LuaValue::nil());
    }

    Ok(())
}
