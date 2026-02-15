// ======================================================================
// Cold opcode handlers - extracted to reduce lua_execute code size.
// Each #[cold] #[inline(never)] function gets its OWN small stack frame,
// keeping the main dispatch loop's frame tight for L1 icache + stack.
// ======================================================================

use crate::{
    Instruction, LuaResult, LuaValue, OpCode,
    lua_vm::{
        LuaState, TmKind,
        execute::{
            helper::{self, ivalue, setfltvalue, setivalue, setnilvalue, tonumberns, ttisinteger},
            metamethod,
        },
    },
};

#[cold]
#[inline(never)]
pub fn handle_loadkx(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    code: &[Instruction],
    constants: &[LuaValue],
    pc: &mut usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;

    if *pc >= code.len() {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error("LOADKX: missing EXTRAARG".to_string()));
    }

    let extra = code[*pc];
    *pc += 1;

    if extra.get_opcode() != OpCode::ExtraArg {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error("LOADKX: expected EXTRAARG".to_string()));
    }

    let ax = extra.get_ax() as usize;
    if ax >= constants.len() {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error(format!("LOADKX: invalid constant index {}", ax)));
    }

    let value = constants[ax];
    let stack = lua_state.stack_mut();
    stack[base + a] = value;
    Ok(())
}

#[cold]
#[inline(never)]
pub fn handle_close(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    pc: usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let close_from = base + a;

    lua_state.set_frame_pc(frame_idx, pc as u32);
    match lua_state.close_all(close_from) {
        Ok(()) => Ok(()),
        Err(crate::lua_vm::LuaError::Yield) => {
            lua_state.get_call_info_mut(frame_idx).pc -= 1;
            Err(crate::lua_vm::LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

#[cold]
#[inline(never)]
pub fn handle_getvarg(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let c = instr.get_c() as usize;

    let call_info = lua_state.get_call_info(frame_idx);
    let nextra = call_info.nextraargs as usize;

    let stack = lua_state.stack_mut();
    let ra_idx = base + a;
    let rc = stack[base + c];

    if let Some(s) = rc.as_str() {
        if s == "n" {
            let stack = lua_state.stack_mut();
            setivalue(&mut stack[ra_idx], nextra as i64);
        } else {
            let stack = lua_state.stack_mut();
            setnilvalue(&mut stack[ra_idx]);
        }
    } else if ttisinteger(&rc) {
        let n = ivalue(&rc);
        let stack = lua_state.stack_mut();
        if nextra > 0 && n >= 1 && (n as usize) <= nextra {
            let slot = (base - 1) - nextra + (n as usize) - 1;
            stack[ra_idx] = stack[slot];
        } else {
            setnilvalue(&mut stack[ra_idx]);
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
                stack[ra_idx] = stack[slot];
            } else {
                setnilvalue(&mut stack[ra_idx]);
            }
        } else {
            let stack = lua_state.stack_mut();
            setnilvalue(&mut stack[ra_idx]);
        }
    } else {
        let stack = lua_state.stack_mut();
        setnilvalue(&mut stack[ra_idx]);
    }
    Ok(())
}

#[cold]
#[inline(never)]
pub fn handle_errnil(
    lua_state: &mut LuaState,
    instr: crate::lua_vm::Instruction,
    base: usize,
    constants: &[LuaValue],
    frame_idx: usize,
    pc: usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let bx = instr.get_bx() as usize;

    let stack = lua_state.stack_mut();
    let ra = &stack[base + a];

    if !ra.is_nil() {
        let global_name = if bx > 0 && bx - 1 < constants.len() {
            if let Some(s) = constants[bx - 1].as_str() {
                s.to_string()
            } else {
                "?".to_string()
            }
        } else {
            "?".to_string()
        };

        lua_state.set_frame_pc(frame_idx, pc as u32);
        return Err(lua_state.error(format!("global '{}' already defined", global_name)));
    }
    Ok(())
}

/// Float for-loop preparation — cold path (integer path is inlined)
#[cold]
#[inline(never)]
pub fn handle_forprep_float(
    lua_state: &mut LuaState,
    ra: usize,
    bx: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let stack = lua_state.stack_mut();
    let mut init = 0.0;
    let mut limit = 0.0;
    let mut step = 0.0;

    if !tonumberns(&stack[ra + 1], &mut limit) {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error("'for' limit must be a number".to_string()));
    }
    if !tonumberns(&stack[ra + 2], &mut step) {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error("'for' step must be a number".to_string()));
    }
    if !tonumberns(&stack[ra], &mut init) {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error("'for' initial value must be a number".to_string()));
    }

    if step == 0.0 {
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error("'for' step is zero".to_string()));
    }

    let should_skip = if step > 0.0 {
        limit < init
    } else {
        init < limit
    };

    if should_skip {
        *pc += bx + 1;
    } else {
        setfltvalue(&mut stack[ra], limit);
        setfltvalue(&mut stack[ra + 1], step);
        setfltvalue(&mut stack[ra + 2], init);
    }
    Ok(())
}

#[cold]
#[inline(never)]
pub fn handle_len(
    lua_state: &mut LuaState,
    instr: crate::lua_vm::Instruction,
    base: &mut usize,
    frame_idx: usize,
    pc: usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;

    let rb = lua_state.stack_mut()[*base + b];

    if let Some(s) = rb.as_str() {
        let len = s.len();
        setivalue(&mut lua_state.stack_mut()[*base + a], len as i64);
    } else if let Some(bytes) = rb.as_binary() {
        let len = bytes.len();
        setivalue(&mut lua_state.stack_mut()[*base + a], len as i64);
    } else if let Some(table) = rb.as_table_mut() {
        let meta = table.meta_ptr();
        if !meta.is_null() {
            let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
            const TM_LEN_BIT: u8 = TmKind::Len as u8;
            if !mt.no_tm(TM_LEN_BIT) {
                let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Len);
                if let Some(mm) = mt.raw_get(&event_key) {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    let result = match metamethod::call_tm_res(lua_state, mm, rb, rb) {
                        Ok(r) => r,
                        Err(crate::lua_vm::LuaError::Yield) => {
                            use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                            let ci = lua_state.get_call_info_mut(frame_idx);
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(crate::lua_vm::LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    };
                    *base = lua_state.get_frame_base(frame_idx);
                    lua_state.stack_mut()[*base + a] = result;
                } else {
                    mt.set_tm_absent(TM_LEN_BIT);
                    setivalue(&mut lua_state.stack_mut()[*base + a], table.len() as i64);
                }
            } else {
                setivalue(&mut lua_state.stack_mut()[*base + a], table.len() as i64);
            }
        } else {
            setivalue(&mut lua_state.stack_mut()[*base + a], table.len() as i64);
        }
    } else {
        if let Some(mm) = helper::get_metamethod_event(lua_state, &rb, TmKind::Len) {
            lua_state.set_frame_pc(frame_idx, pc as u32);
            let result = match metamethod::call_tm_res(lua_state, mm, rb, rb) {
                Ok(r) => r,
                Err(crate::lua_vm::LuaError::Yield) => {
                    use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                    let ci = lua_state.get_call_info_mut(frame_idx);
                    ci.call_status |= CIST_PENDING_FINISH;
                    return Err(crate::lua_vm::LuaError::Yield);
                }
                Err(e) => return Err(e),
            };
            *base = lua_state.get_frame_base(frame_idx);
            lua_state.stack_mut()[*base + a] = result;
        } else {
            return Err(lua_state.error(format!(
                "attempt to get length of a {} value",
                rb.type_name()
            )));
        }
    }
    Ok(())
}
