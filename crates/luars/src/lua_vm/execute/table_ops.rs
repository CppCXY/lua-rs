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
    lua_vm::{Instruction, LuaError, LuaResult, LuaState, OpCode},
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

    // CRITICAL: Update frame.top to protect all registers before calling metamethod
    let max_reg = a.max(b).max(c) + 1;
    let required_top = base + max_reg;
    let call_info = lua_state.get_call_info_mut(frame_idx);
    if required_top > call_info.top {
        call_info.top = required_top;
        lua_state.set_top(required_top);
    }

    // CRITICAL: Copy all values BEFORE any metamethod calls
    let (ra_value, key, value) = {
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
        (ra, rb, val)
    };

    // Always use store_to_metatable which handles __newindex metamethod
    lua_state.set_frame_pc(frame_idx, *pc as u32);
    helper::store_to_metatable(lua_state, &ra_value, &key, value)?;
    
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

    // Check if table has __newindex metamethod - OPTIMIZED: Direct pointer access
    if let Some(table) = ra.as_table_mut() {
        let has_metatable = table.get_metatable().is_some();

        if has_metatable {
            // Has metatable, might have __newindex
            let key = LuaValue::integer(b as i64);
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            helper::store_to_metatable(lua_state, &ra, &key, value)?;
            let new_base = lua_state.get_frame_base(frame_idx);
            if new_base != base {
                return Err(lua_state.error("base changed in SETI".to_string()));
            }
        } else {
            // No metatable, use fast path
            table.set_int(b as i64, value);
            lua_state.vm_mut().check_gc();
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

    // Always use store_to_metatable
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

    let stack = lua_state.stack_mut();
    let rb = stack[base + b];

    // R[A+1] := R[B] (save object)
    stack[base + a + 1] = rb;

    // R[A] := R[B][K[C]] (get method)
    let key = &constants[c];

    let result = if let Some(table) = rb.as_table_mut() {
        // Fast path: direct table access - OPTIMIZED: Direct pointer
        table.raw_get(key)
    } else {
        // Try metatable lookup
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        let result = helper::lookup_from_metatable(lua_state, &rb, key);
        let new_base = lua_state.get_frame_base(frame_idx);
        if new_base != base {
            return Err(lua_state.error("base changed in SELF".to_string()));
        }
        result
    };

    let stack = lua_state.stack_mut();
    stack[base + a] = result.unwrap_or(LuaValue::nil());
    Ok(())
}

/// NEWTABLE: R[A] := {} (new table)
#[inline(always)]
pub fn exec_newtable(
    lua_state: &mut LuaState,
    instr: Instruction,
    code: &[Instruction],
    base: usize,
    _frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let vb = instr.get_vb() as usize;
    let mut vc = instr.get_vc() as usize;
    let k = instr.get_k();

    // Calculate hash size
    let hash_size = if vb > 0 {
        if vb > 31 {
            0
        } else {
            1usize << (vb - 1)
        }
    } else {
        0
    };

    // Check for EXTRAARG instruction for larger array sizes
    if k {
        if *pc < code.len() {
            let extra_instr = code[*pc];
            if extra_instr.get_opcode() == OpCode::ExtraArg {
                let extra = extra_instr.get_ax() as usize;
                vc += extra * 1024;
            }
        }
    }

    // ALWAYS skip the next instruction (EXTRAARG)
    *pc += 1;

    // Create table with pre-allocated sizes
    let value = lua_state.create_table(vc, hash_size);

    let stack = lua_state.stack_mut();
    stack[base + a] = value;
    Ok(())
}
