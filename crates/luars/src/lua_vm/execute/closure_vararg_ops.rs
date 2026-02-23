/*----------------------------------------------------------------------
  Closure and Vararg Operations Module - Extracted from main execution loop

  This module contains complex but non-hot-path instructions:
  - Closure
  - Vararg, GetVarg, VarargPrep
  - SetList (used in table constructors)

  These operations involve complex logic but are not in critical hot paths,
  so extracting them reduces main loop size.
----------------------------------------------------------------------*/

use crate::{
    lua_value::{Chunk, LuaValue},
    lua_vm::{Instruction, LuaResult, LuaState, OpCode, lua_limits::EXTRA_STACK},
};

use super::helper::{buildhiddenargs, setnilvalue};

/// Get the number of vararg arguments from the vararg table's "n" field.
/// Validates that "n" is a non-negative integer not larger than INT_MAX/2.
/// Equivalent to C Lua 5.5's `getnumargs` when a vararg table exists.
fn get_vatab_len(lua_state: &mut LuaState, base: usize, vatab_reg: usize) -> LuaResult<usize> {
    let table_val = {
        let stack = lua_state.stack_mut();
        stack[base + vatab_reg]
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
pub fn exec_vararg(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    chunk: &Chunk,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let k = instr.get_k();

    // wanted = number of results wanted (C-1), -1 means all
    let wanted = if c == 0 { -1 } else { (c - 1) as i32 };

    // Check if using vararg table (k flag set)
    let vatab = if k { b as i32 } else { -1 };

    // Get the number of vararg arguments.
    // If vatab mode, read "n" from the vararg table (user may have modified it).
    // Otherwise, use nextraargs from CallInfo.
    let nargs: usize = if vatab >= 0 {
        get_vatab_len(lua_state, base, b)?
    } else {
        let call_info = lua_state.get_call_info(frame_idx);
        call_info.nextraargs as usize
    };

    // Calculate how many to copy
    let touse = if wanted < 0 {
        nargs // Get all
    } else if (wanted as usize) > nargs {
        nargs
    } else {
        wanted as usize
    };

    // Always update stack_top and frame.top to accommodate the results
    let new_top = base + a + touse;
    lua_state.set_top(new_top)?;
    let call_info = lua_state.get_call_info_mut(frame_idx);
    call_info.top = new_top;

    let ra = base + a;

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
        stack.copy_within(vararg_start..vararg_start + touse, ra);
    } else {
        // Get from vararg table at R[B]
        let table_val = {
            let stack = lua_state.stack_mut();
            stack[base + b]
        };

        if let Some(table) = table_val.as_table_mut() {
            let stack = lua_state.stack_mut();
            for i in 0..touse {
                stack[ra + i] = table.raw_geti((i + 1) as i64).unwrap_or(LuaValue::nil());
            }
        } else {
            // Not a table, fill with nil
            let stack = lua_state.stack_mut();
            for i in 0..touse {
                setnilvalue(&mut stack[ra + i]);
            }
        }
    }

    // Fill remaining with nil
    if wanted >= 0 {
        let stack = lua_state.stack_mut();
        for i in touse..(wanted as usize) {
            setnilvalue(&mut stack[ra + i]);
        }
    }

    Ok(())
}

/// VARARGPREP: Adjust varargs (prepare vararg function)
pub fn exec_varargprep(
    lua_state: &mut LuaState,
    frame_idx: usize,
    chunk: &Chunk,
    base: &mut usize,
) -> LuaResult<()> {
    // Use the nextraargs already computed correctly by push_frame,
    // which knows the actual argument count from the CALL instruction.
    // We must NOT recalculate from stack_top because push_frame inflates
    // stack_top to frame_top (base + maxstacksize) for GC safety,
    // which would give a wrong totalargs.
    let call_info = lua_state.get_call_info(frame_idx);
    let nextra = call_info.nextraargs as usize;
    let func_pos = call_info.base - 1;
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
        stack[target_idx] = table_val;

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

        let new_base = buildhiddenargs(lua_state, frame_idx, chunk, totalargs, nfixparams, nextra)?;
        *base = new_base;

        // Lua 5.5: set vararg parameter register to nil (ltm.c:288)
        // The named vararg param (e.g., 't' in '...t') is at register nfixparams
        // It should be nil when using hidden args mode (GETVARG accesses hidden area directly)
        let stack = lua_state.stack_mut();
        setnilvalue(&mut stack[new_base + nfixparams]);
    }

    Ok(())
}

/// SETLIST: R[A][vC+i] := R[A+i], 1 <= i <= vB
pub fn exec_setlist(
    lua_state: &mut LuaState,
    instr: Instruction,
    code: &[Instruction],
    base: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let mut vb = instr.get_vb() as usize; // number of elements
    let mut vc = instr.get_vc() as usize; // starting index offset
    let k = instr.get_k();

    // Check for EXTRAARG for larger starting indices
    if k && *pc < code.len() {
        let extra_instr = code[*pc];

        if extra_instr.get_opcode() == OpCode::ExtraArg {
            *pc += 1; // Consume EXTRAARG
            let extra = extra_instr.get_ax() as usize;
            // Add extra to starting index
            vc += extra * 1024;
        }
    }

    // If vB == 0, use all values from ra+1 to top
    if vb == 0 {
        let stack_top = lua_state.get_top();
        let ra = base + a;
        vb = if stack_top > ra + 1 {
            stack_top - ra - 1
        } else {
            0
        };
    }

    // Get table from R[A] - OPTIMIZED: Direct pointer access
    let table_val = lua_state.stack_mut()[base + a];
    let mut is_collectable = false;
    if let Some(table) = table_val.as_table_mut() {
        unsafe {
            let stack_ptr = lua_state.stack_mut().as_mut_ptr();
            let impl_table = &mut table.impl_table;

            // Fast path: all indices fit in the pre-allocated array.
            // NewTable already allocated the array with the right size,
            // so for the common case ({1,2,3,4,5}) we can write directly
            // without bounds checks or set_int's push/rehash logic.
            let last_index = (vc + vb) as i64;
            if last_index >= 1 && last_index <= impl_table.asize as i64 {
                for i in 1..=vb {
                    let val = *stack_ptr.add(base + a + i);
                    if val.iscollectable() {
                        is_collectable = true;
                    }
                    let lua_idx = (vc + i) as i64;
                    impl_table.write_array(lua_idx, val);
                }
            } else {
                // Slow path: some indices outside array, use full set_int
                for i in 1..=vb {
                    let val = *stack_ptr.add(base + a + i);
                    if val.iscollectable() {
                        is_collectable = true;
                    }
                    let index = (vc + i) as i64;
                    table.raw_seti(index, val);
                }
            }
        }

        if is_collectable {
            lua_state.gc_barrier_back(table_val.as_gc_ptr().unwrap());
        }
    }

    Ok(())
}
