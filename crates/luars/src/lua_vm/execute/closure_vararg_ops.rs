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
    UpvalueId,
    gc::CachedUpvalue,
    lua_value::{Chunk, LuaValue},
    lua_vm::{Instruction, LuaResult, LuaState, OpCode},
};
use std::rc::Rc;

use super::{
    closure_handler,
    helper::{buildhiddenargs, setnilvalue},
};

/// CLOSURE: R[A] := closure(KPROTO[Bx])
#[inline(always)]
pub fn exec_closure(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    chunk: &Rc<Chunk>,
    upvalue_ptrs: &[CachedUpvalue],
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let bx = instr.get_bx() as usize;

    // Extract upvalue IDs from cached upvalues for closure creation
    let parent_upvalue_ids: Vec<UpvalueId> = upvalue_ptrs.iter().map(|cu| cu.id).collect();

    // Create closure from child prototype
    closure_handler::handle_closure(lua_state, base, a, bx, chunk, &parent_upvalue_ids)?;

    Ok(())
}

/// VARARG: R[A], ..., R[A+C-2] = varargs
#[inline(always)]
pub fn exec_vararg(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    chunk: &Rc<Chunk>,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let k = instr.get_k();

    // wanted = number of results wanted (C-1), -1 means all
    let wanted = if c == 0 { -1 } else { (c - 1) as i32 };

    // Get nextraargs from CallInfo (set by VarargPrep)
    let call_info = lua_state.get_call_info(frame_idx);
    let nargs = call_info.nextraargs as usize;

    // Check if using vararg table (k flag set)
    let vatab = if k { b as i32 } else { -1 };

    // Calculate how many to copy
    let touse = if wanted < 0 {
        nargs // Get all
    } else {
        if (wanted as usize) > nargs {
            nargs
        } else {
            wanted as usize
        }
    };

    // Always update stack_top and frame.top to accommodate the results
    let new_top = base + a + touse;
    lua_state.set_top(new_top);
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

        // Collect values first to avoid borrow issues
        let stack = lua_state.stack_mut();
        let mut values = Vec::with_capacity(touse);
        for i in 0..touse {
            values.push(stack[vararg_start + i]);
        }
        for (i, val) in values.into_iter().enumerate() {
            stack[ra + i] = val;
        }
    } else {
        // Get from vararg table at R[B] - OPTIMIZED: Direct pointer access
        let table_val = {
            let stack = lua_state.stack_mut();
            stack[base + b]
        };

        if let Some(table) = table_val.as_table_mut() {
            // Direct pointer access
            let mut values = Vec::with_capacity(touse);
            for i in 0..touse {
                let val = table.get_int((i + 1) as i64).unwrap_or(LuaValue::nil());
                values.push(val);
            }
            // Now write to stack
            let stack = lua_state.stack_mut();
            for (i, val) in values.into_iter().enumerate() {
                stack[ra + i] = val;
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
#[inline(always)]
pub fn exec_varargprep(
    lua_state: &mut LuaState,
    frame_idx: usize,
    chunk: &Rc<Chunk>,
    base: &mut usize,
) -> LuaResult<()> {
    // Calculate total arguments and extra arguments
    let call_info = lua_state.get_call_info(frame_idx);
    let func_pos = call_info.base - 1;
    let stack_top = lua_state.get_top();

    // Total arguments = stack_top - func_pos - 1 (exclude function itself)
    let totalargs = if stack_top > func_pos {
        stack_top - func_pos - 1
    } else {
        0
    };

    let nfixparams = chunk.param_count;
    let nextra = if totalargs > nfixparams {
        totalargs - nfixparams
    } else {
        0
    };

    // Store nextra in CallInfo for later use by VARARG/GETVARG
    let call_info = lua_state.get_call_info_mut(frame_idx);
    call_info.nextraargs = nextra as i32;

    // Implement buildhiddenargs if there are extra args
    if nextra > 0 {
        // Ensure stack has enough space
        let required_size = func_pos + totalargs + 1 + nfixparams + chunk.max_stack_size;
        if lua_state.stack_len() < required_size {
            lua_state.grow_stack(required_size)?;
        }

        let new_base = buildhiddenargs(lua_state, frame_idx, chunk, totalargs, nfixparams, nextra)?;
        *base = new_base;
    }

    Ok(())
}

/// SETLIST: R[A][vC+i] := R[A+i], 1 <= i <= vB
#[inline(always)]
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
    if k {
        if *pc < code.len() {
            let extra_instr = code[*pc];

            if extra_instr.get_opcode() == OpCode::ExtraArg {
                *pc += 1; // Consume EXTRAARG
                let extra = extra_instr.get_ax() as usize;
                // Add extra to starting index
                vc += extra * 1024;
            }
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

    if let Some(table) = table_val.as_table_mut() {
        // Direct pointer access - no object_pool needed
        unsafe {
            let stack_ptr = lua_state.stack_mut().as_mut_ptr();

            // Set elements: table[vc+i] = R[A+i] for i=1..vb
            for i in 1..=vb {
                let val = stack_ptr.add(base + a + i);
                let index = (vc + i) as i64;
                table.set_int(index, *val);
            }
        }
    }

    Ok(())
}
