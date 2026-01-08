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
    gc::CachedUpvalue,
    lua_value::{Chunk, LuaValue},
    lua_vm::{Instruction, LuaError, LuaResult, LuaState, OpCode},
    UpvalueId,
};
use std::rc::Rc;

use super::{
    closure_handler,
    helper::{buildhiddenargs, ivalue, setivalue, setnilvalue, ttisinteger},
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

/// GETVARG: R[A] := varargs[R[C]]
#[inline(always)]
pub fn exec_getvarg(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let _b = instr.get_b() as usize; // unused in Lua 5.5
    let c = instr.get_c() as usize;

    // Get nextraargs from CallInfo
    let call_info = lua_state.get_call_info(frame_idx);
    let nextra = call_info.nextraargs as usize;
    let func_id = call_info
        .func
        .as_function_id()
        .ok_or(LuaError::RuntimeError)?;

    // Get param_count from the function's chunk
    let param_count = {
        let vm = lua_state.vm_mut();
        let func_obj = vm
            .object_pool
            .get_function(func_id)
            .ok_or(LuaError::RuntimeError)?;
        let chunk = func_obj.data.chunk().ok_or(LuaError::RuntimeError)?;
        chunk.param_count
    };

    let stack = lua_state.stack_mut();
    let ra_idx = base + a;
    let rc = stack[base + c];

    // Check if R[C] is string "n" (get vararg count)
    if let Some(string_id) = rc.as_string_id() {
        let is_n = lua_state
            .vm_mut()
            .object_pool
            .get_string(string_id)
            .map(|s| s == "n")
            .unwrap_or(false);
        if is_n {
            // Return vararg count
            let stack = lua_state.stack_mut();
            setivalue(&mut stack[ra_idx], nextra as i64);
            *pc += 1;
            return Ok(());
        }
    }

    // Check if R[C] is an integer (vararg index, 1-based)
    if ttisinteger(&rc) {
        let index = ivalue(&rc);

        // Check if index is valid (1 <= index <= nextraargs)
        let stack = lua_state.stack_mut();
        if nextra > 0 && index >= 1 && (index as usize) <= nextra {
            // Get value from varargs at base + param_count
            let vararg_start = base + param_count;
            let src_val = stack[vararg_start + (index as usize) - 1];
            stack[ra_idx] = src_val;
        } else {
            // Out of bounds or no varargs: return nil
            setnilvalue(&mut stack[ra_idx]);
        }
    } else {
        // Not integer or "n": return nil
        let stack = lua_state.stack_mut();
        setnilvalue(&mut stack[ra_idx]);
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
