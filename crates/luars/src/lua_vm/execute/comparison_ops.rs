/*----------------------------------------------------------------------
  Comparison Operations Module - Extracted from main execution loop
  
  This module contains comparison instructions with metamethod support:
  - Eq, Lt, Le (register-register comparisons)
  - EqK, EqI, LtI, LeI, GtI, GeI (constant/immediate comparisons)
  
  These operations can trigger metamethods and have complex logic,
  so extracting them reduces main loop size.
----------------------------------------------------------------------*/

use crate::{
    lua_value::LuaValue,
    lua_vm::{Instruction, LuaResult, LuaState},
};

use super::{
    helper::{fltvalue_ref, ivalue_ref, tonumberns_ref, ttisfloat_ref, ttisinteger_ref, ttisstring_ref},
    metamethod::{self, TmKind},
};

/// EQ: if ((R[A] == R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_eq(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<bool> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let k = instr.get_k();

    let (ra, rb) = {
        let stack = lua_state.stack_mut();
        (stack[base + a], stack[base + b])
    };

    // Save PC before potential metamethod call
    lua_state.set_frame_pc(frame_idx, *pc as u32);
    let cond = metamethod::equalobj(lua_state, ra, rb)?;
    
    // Verify base hasn't changed
    let new_base = lua_state.get_frame_base(frame_idx);
    if new_base != base {
        return Err(lua_state.error("base changed in EQ".to_string()));
    }

    if cond != k {
        *pc += 1; // Condition failed - skip next instruction
    }
    Ok(true)
}

/// LT: if ((R[A] < R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_lt(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<bool> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let k = instr.get_k();

    let cond = {
        let stack = lua_state.stack_mut();
        let ra = &stack[base + a];
        let rb = &stack[base + b];

        if ttisinteger_ref(ra) && ttisinteger_ref(rb) {
            ivalue_ref(ra) < ivalue_ref(rb)
        } else if (ttisinteger_ref(ra) || ttisfloat_ref(ra)) && (ttisinteger_ref(rb) || ttisfloat_ref(rb)) {
            let mut na = 0.0;
            let mut nb = 0.0;
            tonumberns_ref(ra, &mut na);
            tonumberns_ref(rb, &mut nb);
            na < nb
        } else if ttisstring_ref(ra) && ttisstring_ref(rb) {
            // String comparison
            let sid_a = ra.tsvalue();
            let sid_b = rb.tsvalue();
            let _ = stack; // Release stack borrow

            let pool = &lua_state.vm_mut().object_pool;
            if let (Some(sa), Some(sb)) = (pool.get_string(sid_a), pool.get_string(sid_b)) {
                sa < sb
            } else {
                false
            }
        } else {
            // Try metamethod
            let va = *ra;
            let vb = *rb;

            lua_state.set_frame_pc(frame_idx, *pc as u32);
            let result = match metamethod::try_comp_tm(lua_state, va, vb, TmKind::Lt)? {
                Some(result) => result,
                None => {
                    return Err(lua_state
                        .error("attempt to compare non-comparable values".to_string()));
                }
            };
            
            let new_base = lua_state.get_frame_base(frame_idx);
            if new_base != base {
                return Err(lua_state.error("base changed in LT".to_string()));
            }
            
            result
        }
    };

    if cond != k {
        *pc += 1;
    }
    Ok(true)
}

/// LE: if ((R[A] <= R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_le(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<bool> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let k = instr.get_k();

    let cond = {
        let stack = lua_state.stack_mut();
        let ra = &stack[base + a];
        let rb = &stack[base + b];

        if ttisinteger_ref(ra) && ttisinteger_ref(rb) {
            ivalue_ref(ra) <= ivalue_ref(rb)
        } else if (ttisinteger_ref(ra) || ttisfloat_ref(ra)) && (ttisinteger_ref(rb) || ttisfloat_ref(rb)) {
            let mut na = 0.0;
            let mut nb = 0.0;
            tonumberns_ref(ra, &mut na);
            tonumberns_ref(rb, &mut nb);
            na <= nb
        } else if ttisstring_ref(ra) && ttisstring_ref(rb) {
            // String comparison
            let sid_a = ra.tsvalue();
            let sid_b = rb.tsvalue();

            let pool = &lua_state.vm_mut().object_pool;
            if let (Some(sa), Some(sb)) = (pool.get_string(sid_a), pool.get_string(sid_b)) {
                sa <= sb
            } else {
                false
            }
        } else {
            // Try metamethod
            let va = *ra;
            let vb = *rb;

            lua_state.set_frame_pc(frame_idx, *pc as u32);
            let result = match metamethod::try_comp_tm(lua_state, va, vb, TmKind::Le)? {
                Some(result) => result,
                None => {
                    return Err(lua_state
                        .error("attempt to compare non-comparable values".to_string()));
                }
            };
            
            let new_base = lua_state.get_frame_base(frame_idx);
            if new_base != base {
                return Err(lua_state.error("base changed in LE".to_string()));
            }
            
            result
        }
    };

    if cond != k {
        *pc += 1;
    }
    Ok(true)
}

/// EQK: if ((R[A] == K[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_eqk(
    lua_state: &mut LuaState,
    instr: Instruction,
    constants: &[LuaValue],
    base: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let k = instr.get_k();

    let stack = lua_state.stack_mut();
    let ra = stack[base + a];
    let kb = constants.get(b).unwrap();

    // Raw equality (no metamethods for constants)
    let cond = ra == *kb;
    if cond != k {
        *pc += 1;
    }
    Ok(())
}

/// EQI: if ((R[A] == sB) ~= k) then pc++
#[inline(always)]
pub fn exec_eqi(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let sb = instr.get_sb();
    let k = instr.get_k();

    let stack = lua_state.stack_mut();
    let ra = &stack[base + a];

    let cond = if ttisinteger_ref(ra) {
        ivalue_ref(ra) == (sb as i64)
    } else if ttisfloat_ref(ra) {
        fltvalue_ref(ra) == (sb as f64)
    } else {
        false
    };

    if cond != k {
        *pc += 1;
    }
    Ok(())
}

/// LTI: if ((R[A] < sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lti(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let im = instr.get_sb();
    let k = instr.get_k();

    let stack = lua_state.stack_mut();
    let ra = &stack[base + a];

    let cond = if ttisinteger_ref(ra) {
        ivalue_ref(ra) < (im as i64)
    } else if ttisfloat_ref(ra) {
        fltvalue_ref(ra) < (im as f64)
    } else {
        false
    };

    if cond != k {
        *pc += 1;
    }
    Ok(())
}

/// LEI: if ((R[A] <= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lei(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let im = instr.get_sb();
    let k = instr.get_k();

    let stack = lua_state.stack_mut();
    let ra = &stack[base + a];

    let cond = if ttisinteger_ref(ra) {
        ivalue_ref(ra) <= (im as i64)
    } else if ttisfloat_ref(ra) {
        fltvalue_ref(ra) <= (im as f64)
    } else {
        false
    };

    if cond != k {
        *pc += 1;
    }
    Ok(())
}

/// GTI: if ((R[A] > sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gti(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let im = instr.get_sb();
    let k = instr.get_k();

    let stack = lua_state.stack_mut();
    let ra = &stack[base + a];

    let cond = if ttisinteger_ref(ra) {
        ivalue_ref(ra) > (im as i64)
    } else if ttisfloat_ref(ra) {
        fltvalue_ref(ra) > (im as f64)
    } else {
        false
    };

    if cond != k {
        *pc += 1;
    }
    Ok(())
}

/// GEI: if ((R[A] >= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gei(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let im = instr.get_sb();
    let k = instr.get_k();

    let stack = lua_state.stack_mut();
    let ra = &stack[base + a];

    let cond = if ttisinteger_ref(ra) {
        ivalue_ref(ra) >= (im as i64)
    } else if ttisfloat_ref(ra) {
        fltvalue_ref(ra) >= (im as f64)
    } else {
        false
    };

    if cond != k {
        *pc += 1;
    }
    Ok(())
}
