use crate::{
    CallInfo,
    lua_value::{Chunk, LuaValue},
    lua_vm::{
        LuaResult, LuaState,
        execute::helper::{ivalue, setivalue, ttisinteger},
        lua_limits::EXTRA_STACK,
    },
};

use super::helper::{buildhiddenargs, setnilvalue};

/// Get the number of vararg arguments from the vararg table's "n" field.
/// Validates that "n" is a non-negative integer not larger than INT_MAX/2.
/// Equivalent to C Lua 5.5's `getnumargs` when a vararg table exists.
fn get_vatab_len(lua_state: &mut LuaState, base: usize, vatab_reg: usize) -> LuaResult<usize> {
    let table_val = {
        let stack = lua_state.stack_mut();
        unsafe { *stack.get_unchecked(base + vatab_reg) }
    };

    if let Some(table) = table_val.as_table_mut() {
        // Read the "n" field from the table
        let n_key = lua_state.create_string("n")?;
        let n_val = table.raw_get(&n_key);

        match n_val {
            Some(val) if val.is_integer() => {
                let n = val.as_integer().unwrap();
                // l_castS2U(n) > cast_uint(INT_MAX/2) — treat as unsigned, must be <= i32::MAX/2
                if (n as u64) > (i32::MAX as u64 / 2) {
                    return Err(lua_state.error("vararg table has no proper 'n'".to_string()));
                }
                Ok(n as usize)
            }
            _ => Err(lua_state.error("vararg table has no proper 'n'".to_string())),
        }
    } else {
        Err(lua_state.error("vararg table has no proper 'n'".to_string()))
    }
}

/// VARARG: R[A], ..., R[A+C-2] = varargs
///
/// Lua 5.5: When k flag is set, B is the register of the vararg table.
/// The number of varargs is read from the table's "n" field (which may
/// have been modified by user code), not from CallInfo.nextraargs.
pub fn get_varargs(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base: usize,
    a: usize,
    b: usize,
    vatab: i32,
    wanted: i32,
    chunk: &Chunk,
) -> LuaResult<()> {
    // Get the number of vararg arguments.
    // If vatab mode, read "n" from the vararg table (user may have modified it).
    // Otherwise, use nextraargs from CallInfo.
    let nargs: usize = if vatab >= 0 {
        get_vatab_len(lua_state, base, vatab as usize)?
    } else {
        ci.nextraargs as usize
    };

    // Calculate how many to copy
    let touse = if wanted < 0 {
        nargs // Get all
    } else if (wanted as usize) > nargs {
        nargs
    } else {
        wanted as usize
    };

    // Always update stack_top to accommodate the results
    // NOTE: Only update L->top (stack_top), NOT ci->top.
    // ci->top must remain at base + max_stack_size to protect ALL registers.
    // C Lua's luaT_getvarargs only sets L->top.p = where + nvar, never ci->top.
    let new_top = base + a + touse;
    lua_state.set_top(new_top)?;

    let ra_pos = base + a;

    if vatab < 0 {
        // No vararg table - get from stack
        let nfixparams = chunk.param_count;
        let totalargs = nfixparams + nargs;

        let new_func_pos = base - 1;
        let old_func_pos = if totalargs > 0 && new_func_pos > totalargs {
            new_func_pos - totalargs - 1
        } else {
            base + nfixparams
        };
        let vararg_start = old_func_pos + 1 + nfixparams;

        // Varargs are stored below base, ra is at base+a, so ranges never overlap.
        // Use copy_within to avoid heap allocation.
        let stack = lua_state.stack_mut();
        stack.copy_within(vararg_start..vararg_start + touse, ra_pos);
    } else {
        // Get from vararg table at R[B]
        let table_val = { unsafe { *lua_state.stack().get_unchecked(base + b) } };

        if let Some(table) = table_val.as_table_mut() {
            let stack = lua_state.stack_mut();
            for i in 0..touse {
                unsafe {
                    *stack.get_unchecked_mut(ra_pos + i) =
                        table.raw_geti((i + 1) as i64).unwrap_or(LuaValue::nil())
                };
            }
        } else {
            // Not a table, fill with nil
            let stack = lua_state.stack_mut();
            for i in 0..touse {
                setnilvalue(unsafe { stack.get_unchecked_mut(ra_pos + i) });
            }
        }
    }

    // Fill remaining with nil
    if wanted >= 0 {
        let stack = lua_state.stack_mut();
        for i in touse..(wanted as usize) {
            setnilvalue(unsafe { stack.get_unchecked_mut(ra_pos + i) });
        }
    }

    Ok(())
}

pub fn get_vararg(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base: usize,
    ra_pos: usize,
    rc_pos: usize,
) -> LuaResult<()> {
    let nextra = ci.nextraargs as usize;

    // Ensure stack is large enough for rc_pos access
    if rc_pos >= lua_state.stack_len() {
        lua_state.grow_stack(rc_pos + 1)?;
    }

    let stack = lua_state.stack_mut();
    let rc = stack[rc_pos];

    if let Some(s) = rc.as_str() {
        if s == "n" {
            let stack = lua_state.stack_mut();
            setivalue(unsafe { stack.get_unchecked_mut(ra_pos) }, nextra as i64);
        } else {
            let stack = lua_state.stack_mut();
            setnilvalue(unsafe { stack.get_unchecked_mut(ra_pos) });
        }
    } else if ttisinteger(&rc) {
        let n = ivalue(&rc);
        let stack = lua_state.stack_mut();
        if nextra > 0 && n >= 1 && (n as usize) <= nextra {
            let slot = (base - 1) - nextra + (n as usize) - 1;
            stack[ra_pos] = stack[slot];
        } else {
            setnilvalue(unsafe { stack.get_unchecked_mut(ra_pos) });
        }
    } else if rc.is_float() {
        // Lua 5.5: tointegerns - convert integer-valued float to integer
        let f = rc.as_float().unwrap();
        let n = f as i64;
        if (n as f64) == f {
            // Float is integer-valued
            let stack = lua_state.stack_mut();
            if nextra > 0 && n >= 1 && (n as usize) <= nextra {
                let slot = (base - 1) - nextra + (n as usize) - 1;
                stack[ra_pos] = stack[slot];
            } else {
                setnilvalue(unsafe { stack.get_unchecked_mut(ra_pos) });
            }
        } else {
            let stack = lua_state.stack_mut();
            setnilvalue(unsafe { stack.get_unchecked_mut(ra_pos) });
        }
    } else {
        let stack = lua_state.stack_mut();
        setnilvalue(unsafe { stack.get_unchecked_mut(ra_pos) });
    }
    Ok(())
}

/// VARARGPREP: Adjust varargs (prepare vararg function)
pub fn exec_varargprep(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    chunk: &Chunk,
    base: &mut usize,
) -> LuaResult<()> {
    // Use the nextraargs already computed correctly by push_frame,
    // which knows the actual argument count from the CALL instruction.
    // We must NOT recalculate from stack_top because push_frame inflates
    // stack_top to frame_top (base + maxstacksize) for GC safety,
    // which would give a wrong totalargs.

    let nextra = ci.nextraargs as usize;
    let func_pos = ci.base - 1;
    let nfixparams = chunk.param_count;
    let totalargs = nfixparams + nextra;

    // Handle Lua 5.5 named varargs (requires table)
    if chunk.needs_vararg_table {
        // Create table with size 'nextra'
        // Collect arguments first to avoid borrow conflicts
        let mut args = Vec::with_capacity(nextra);
        {
            let stack = lua_state.stack();
            let args_start = func_pos + 1 + nfixparams;
            for i in 0..nextra {
                if args_start + i < stack.len() {
                    args.push(stack[args_start + i]);
                } else {
                    args.push(LuaValue::nil());
                }
            }
        }

        // Create table
        // Use 0 for hash part to encourage ValueArray creation for efficient named varargs
        // ValueArray now supports "n" key natively
        let table_val = lua_state.create_table(nextra, 0)?;
        let n_str = lua_state.create_string("n")?;
        let n_val = LuaValue::integer(nextra as i64);

        // Populate table
        {
            if let Some(table_ref) = table_val.as_table_mut() {
                // Populate array first (ValueArray will push)
                for (i, val) in args.into_iter().enumerate() {
                    table_ref.raw_seti((i + 1) as i64, val);
                }
                // Set "n" last (ValueArray will confirm matching length or resize)
                table_ref.raw_set(&n_str, n_val);
            }
        }

        // Place table at base + nfixparams (overwriting first extra arg or empty slot)
        // Ensure stack is large enough
        let target_idx = func_pos + 1 + nfixparams;

        // If target_idx is beyond current stack, we need to push
        if target_idx >= lua_state.stack_len() {
            lua_state.grow_stack(target_idx + 1)?;
        }

        let stack = lua_state.stack_mut();
        unsafe { *stack.get_unchecked_mut(target_idx) = table_val };

        // In Lua 5.5 C implementation "luaT_adjustvarargs", the table is placed
        // at the slot after fixed parameters. L->top is adjusted to include the table.
        // The remaining extra args on the stack are not explicitly cleared but are effectively ignored.
        // However, for safety in Rust VM, we might want to clear them or just leave them.
        // We will leave them be, as they are "above" the relevant stack usage for this function frame.
    }
    // Implement buildhiddenargs if there are extra args (and no table needed)
    else if nextra > 0 {
        // Ensure stack has enough space
        let required_size =
            func_pos + totalargs + 1 + nfixparams + chunk.max_stack_size + EXTRA_STACK;
        if lua_state.stack_len() < required_size {
            lua_state.grow_stack(required_size)?;
        }

        let new_base = buildhiddenargs(lua_state, ci, chunk, totalargs, nfixparams, nextra)?;
        *base = new_base;

        // Lua 5.5: set vararg parameter register to nil (ltm.c:288)
        // The named vararg param (e.g., 't' in '...t') is at register nfixparams
        // It should be nil when using hidden args mode (GETVARG accesses hidden area directly)
        let stack = lua_state.stack_mut();
        setnilvalue(unsafe { stack.get_unchecked_mut(new_base + nfixparams) });
    }
    // No vararg table needed and no extra args: still need to nil the vararg register
    // so it doesn't contain stale stack values
    else if chunk.is_vararg {
        let current_base = ci.base;
        let stack = lua_state.stack_mut();
        setnilvalue(unsafe { stack.get_unchecked_mut(current_base + nfixparams) });
    }

    Ok(())
}
