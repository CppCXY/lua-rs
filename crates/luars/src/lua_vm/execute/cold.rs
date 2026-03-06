// ======================================================================
// Cold opcode handlers - extracted to reduce lua_execute code size.
// Each #[cold] #[inline(never)] function gets its OWN small stack frame,
// keeping the main dispatch loop's frame tight for L1 icache + stack.
// ======================================================================

use crate::{
    GcTable, Instruction, LuaResult, LuaValue, OpCode,
    gc::UpvaluePtr,
    lua_vm::{
        LuaError, LuaState, TmKind,
        execute::{
            helper::{self, ivalue, setfltvalue, setivalue, setnilvalue, tonumberns, ttisinteger},
            metamethod,
        },
    },
    stdlib::basic::parse_number::parse_lua_number,
};

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

    // Copy values for potential error messages (avoids borrow conflict)
    let limit_val = stack[ra + 1];
    let step_val = stack[ra + 2];
    let init_val = stack[ra];

    if !tonumberns(&limit_val, &mut limit) {
        let t = crate::stdlib::debug::objtypename(lua_state, &limit_val);
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error(format!("bad 'for' limit (number expected, got {})", t)));
    }
    if !tonumberns(&step_val, &mut step) {
        let t = crate::stdlib::debug::objtypename(lua_state, &step_val);
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error(format!("bad 'for' step (number expected, got {})", t)));
    }
    if !tonumberns(&init_val, &mut init) {
        let t = crate::stdlib::debug::objtypename(lua_state, &init_val);
        lua_state.set_frame_pc(frame_idx, *pc as u32);
        return Err(lua_state.error(format!(
            "bad 'for' initial value (number expected, got {})",
            t
        )));
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
                if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
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
        // Try trait-based __len for userdata first
        if rb.ttisfulluserdata()
            && let Some(ud) = rb.as_userdata_mut()
            && let Some(udv) = ud.get_trait().lua_len()
        {
            let result = crate::lua_value::udvalue_to_lua_value(lua_state, udv)?;
            lua_state.stack_mut()[*base + a] = result;
            return Ok(());
        }
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
            return Err(crate::stdlib::debug::typeerror(
                lua_state,
                &rb,
                "get length of",
            ));
        }
    }
    Ok(())
}

/// Cold path: try __call metamethod on a table value.
/// Returns Ok(true) if the __call was handled (new Lua frame pushed),
/// Ok(false) if __call was not found or is not a Lua function (fall through to handle_call).
/// The caller should `continue 'startfunc` on Ok(true) to reload all context.
#[cold]
#[inline(never)]
pub fn handle_call_metamethod(
    lua_state: &mut LuaState,
    func: LuaValue,
    func_idx: usize,
    b: usize,
    c: usize,
) -> LuaResult<bool> {
    let table = unsafe { &mut *(func.value.ptr as *mut GcTable) };
    let meta = table.data.meta_ptr();
    if meta.is_null() {
        return Ok(false);
    }
    let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
    const TM_CALL_BIT: u8 = TmKind::Call as u8;
    if mt.no_tm(TM_CALL_BIT) {
        return Ok(false);
    }
    let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Call);
    let mm = match mt.impl_table.get_shortstr_fast(&event_key) {
        Some(mm) => mm,
        None => {
            // __call absent — cache it
            mt.set_tm_absent(TM_CALL_BIT);
            return Ok(false);
        }
    };
    if !mm.is_lua_function() {
        // __call is not a Lua function — fall to cold handle_call path
        return Ok(false);
    }

    // Compute nargs
    let nargs = if b != 0 {
        b - 1
    } else {
        let current_top = lua_state.get_top();
        if current_top > func_idx + 1 {
            current_top - func_idx - 1
        } else {
            0
        }
    };

    // Shift arguments right by 1 using unsafe ptr copy
    // Layout: [func, arg1, ..., argN]  →  [mm, func, arg1, ..., argN]
    unsafe {
        let sp = lua_state.stack_mut().as_mut_ptr();
        let src = sp.add(func_idx + 1);
        std::ptr::copy(src, src.add(1), nargs);
        *src = func; // original callable → 1st arg
        *sp.add(func_idx) = mm; // metamethod → func slot
    }

    let new_nargs = nargs + 1;
    let nresults = if c == 0 { -1 } else { (c - 1) as i32 };

    // For b==0 case, stack_top needs +1 for shifted arg
    if b == 0 {
        let top = lua_state.get_top();
        lua_state.set_top_raw(top + 1);
    }

    let lua_func = unsafe { mm.as_lua_function_unchecked() };
    let new_chunk = lua_func.chunk();

    lua_state.push_lua_frame(
        &mm,
        func_idx + 1,
        new_nargs,
        nresults,
        new_chunk.param_count,
        new_chunk.max_stack_size,
        new_chunk as *const _,
    )?;

    // Track __call depth for debug.getinfo extraargs
    {
        use crate::lua_vm::call_info::call_status;
        let new_fi = lua_state.call_depth() - 1;
        let ci = lua_state.get_call_info_mut(new_fi);
        ci.call_status = call_status::set_ccmt_count(ci.call_status, 1);
    }

    Ok(true)
}

/// Cold error: attempt to divide by zero (IDIV)
#[cold]
#[inline(never)]
pub fn error_div_by_zero(lua_state: &mut LuaState) -> LuaError {
    lua_state.error("attempt to divide by zero".to_string())
}

/// Cold error: attempt to perform 'n%0' (MOD)
#[cold]
#[inline(never)]
pub fn error_mod_by_zero(lua_state: &mut LuaState) -> LuaError {
    lua_state.error("attempt to perform 'n%0'".to_string())
}

/// Cold error: table index is nil
#[cold]
#[inline(never)]
pub fn error_table_index_nil(lua_state: &mut LuaState) -> LuaError {
    lua_state.error("table index is nil".to_string())
}

/// Cold error: table index is NaN
#[cold]
#[inline(never)]
pub fn error_table_index_nan(lua_state: &mut LuaState) -> LuaError {
    lua_state.error("table index is NaN".to_string())
}

/// Cold error: unexpected EXTRAARG instruction
#[cold]
#[inline(never)]
pub fn error_unexpected_extraarg(lua_state: &mut LuaState) -> LuaError {
    lua_state.error("unexpected EXTRAARG instruction".to_string())
}

/// Cold helper: register-register comparison metamethod fallback (used by exec_lt, exec_le)
#[cold]
#[inline(never)]
pub fn cmp_reg_metamethod(
    lua_state: &mut LuaState,
    va: LuaValue,
    vb: LuaValue,
    tm: TmKind,
    frame_idx: usize,
    pc: usize,
    base: usize,
) -> LuaResult<bool> {
    lua_state.set_frame_pc(frame_idx, pc as u32);
    match super::metamethod::try_comp_tm(lua_state, va, vb, tm) {
        Ok(Some(result)) => {
            let new_base = lua_state.get_frame_base(frame_idx);
            if new_base != base {
                return Err(lua_state.error("base changed in comparison".to_string()));
            }
            Ok(result)
        }
        Ok(None) => Err(crate::stdlib::debug::ordererror(lua_state, &va, &vb)),
        Err(LuaError::Yield) => {
            use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
            let ci = lua_state.get_call_info_mut(frame_idx);
            ci.call_status |= CIST_PENDING_FINISH;
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Cold helper: immediate comparison metamethod fallback (used by exec_lti, exec_lei, exec_gti, exec_gei)
#[cold]
#[inline(never)]
pub fn cmp_imm_metamethod(
    lua_state: &mut LuaState,
    ra_val: LuaValue,
    im: i32,
    isf: bool,
    tm: TmKind,
    swap: bool,
    frame_idx: usize,
    pc: usize,
) -> LuaResult<bool> {
    let imm_val = if isf {
        LuaValue::float(im as f64)
    } else {
        LuaValue::integer(im as i64)
    };
    let (va, vb) = if swap {
        (imm_val, ra_val)
    } else {
        (ra_val, imm_val)
    };
    lua_state.set_frame_pc(frame_idx, pc as u32);
    match super::metamethod::try_comp_tm(lua_state, va, vb, tm) {
        Ok(Some(result)) => Ok(result),
        Ok(None) => Err(crate::stdlib::debug::ordererror(lua_state, &va, &vb)),
        Err(LuaError::Yield) => {
            use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
            let ci = lua_state.get_call_info_mut(frame_idx);
            ci.call_status |= CIST_PENDING_FINISH;
            Err(LuaError::Yield)
        }
        Err(e) => Err(e),
    }
}

/// Cold helper: push a non-recursive Lua metamethod frame and mark caller PENDING_FINISH.
/// Used by MmBin and Len opcodes. Returns Ok(()) on success; caller should `continue 'startfunc`.
#[cold]
#[inline(never)]
pub fn push_lua_mm_frame(
    lua_state: &mut LuaState,
    mm: LuaValue,
    v1: LuaValue,
    v2: LuaValue,
    frame_idx: usize,
) -> LuaResult<()> {
    let func_pos = lua_state.current_frame_top_unchecked();
    let top = lua_state.get_top();
    if top != func_pos {
        lua_state.set_top_raw(func_pos);
    }
    unsafe {
        let sp = lua_state.stack_mut().as_mut_ptr();
        *sp.add(func_pos) = mm;
        *sp.add(func_pos + 1) = v1;
        *sp.add(func_pos + 2) = v2;
    }
    lua_state.set_top_raw(func_pos + 3);

    let lua_func = unsafe { mm.as_lua_function_unchecked() };
    let chunk_mm = lua_func.chunk();
    lua_state.push_lua_frame(
        &mm,
        func_pos + 1,
        2,
        1,
        chunk_mm.param_count,
        chunk_mm.max_stack_size,
        chunk_mm as *const _,
    )?;
    {
        use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
        let ci = lua_state.get_call_info_mut(frame_idx);
        ci.call_status |= CIST_PENDING_FINISH;
    }
    Ok(())
}

/// Cold helper: push a non-recursive Lua __newindex metamethod frame (3 args, 0 results).
/// Used by SetField/SetI/SetTable/SetTabUp opcodes.
/// Caller should `continue 'startfunc`.
#[cold]
#[inline(never)]
pub fn push_lua_newindex_frame(
    lua_state: &mut LuaState,
    mm: LuaValue,
    obj: LuaValue,
    key: LuaValue,
    val: LuaValue,
    frame_idx: usize,
) -> LuaResult<()> {
    let func_pos = lua_state.current_frame_top_unchecked();
    let top = lua_state.get_top();
    if top != func_pos {
        lua_state.set_top_raw(func_pos);
    }
    unsafe {
        let sp = lua_state.stack_mut().as_mut_ptr();
        *sp.add(func_pos) = mm;
        *sp.add(func_pos + 1) = obj;
        *sp.add(func_pos + 2) = key;
        *sp.add(func_pos + 3) = val;
    }
    lua_state.set_top_raw(func_pos + 4);

    let lua_func = unsafe { mm.as_lua_function_unchecked() };
    let chunk_mm = lua_func.chunk();
    lua_state.push_lua_frame(
        &mm,
        func_pos + 1,
        3,
        0,
        chunk_mm.param_count,
        chunk_mm.max_stack_size,
        chunk_mm as *const _,
    )?;
    {
        use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
        let ci = lua_state.get_call_info_mut(frame_idx);
        ci.call_status |= CIST_PENDING_FINISH;
    }
    Ok(())
}

/// Cold helper: Try to push a non-recursive comparison metamethod frame.
/// Looks up metamethod from both operands' metatables (v1 first, then v2).
/// If found and is a Lua function, pushes the frame and returns Ok(true).
/// Otherwise returns Ok(false) so the caller can fall through to the recursive path.
#[cold]
#[inline(never)]
pub fn try_push_comp_mm_frame(
    lua_state: &mut LuaState,
    v1: LuaValue,
    v2: LuaValue,
    tm: TmKind,
    frame_idx: usize,
) -> LuaResult<bool> {
    let mm = helper::get_binop_metamethod(lua_state, &v1, &v2, tm);
    if let Some(mm) = mm
        && mm.is_lua_function()
    {
        push_lua_mm_frame(lua_state, mm, v1, v2, frame_idx)?;
        return Ok(true);
    }
    Ok(false)
}

/// Cold helper: Try to push a non-recursive __eq metamethod frame.
/// Only for same-type tables or userdata. Returns Ok(true) if frame pushed.
#[cold]
#[inline(never)]
pub fn try_push_eq_mm_frame(
    lua_state: &mut LuaState,
    t1: LuaValue,
    t2: LuaValue,
    frame_idx: usize,
) -> LuaResult<bool> {
    let mm = helper::get_binop_metamethod(lua_state, &t1, &t2, TmKind::Eq);
    if let Some(mm) = mm
        && mm.is_lua_function()
    {
        push_lua_mm_frame(lua_state, mm, t1, t2, frame_idx)?;
        return Ok(true);
    }
    Ok(false)
}

/// Cold helper: Try to push a non-recursive unary metamethod frame (for UNM, BNOT).
/// Looks up metamethod from the operand's metatable.
/// If found and is a Lua function, pushes the frame (mm, operand, operand) and returns Ok(true).
#[cold]
#[inline(never)]
pub fn try_push_unary_mm_frame(
    lua_state: &mut LuaState,
    operand: LuaValue,
    tm: TmKind,
    frame_idx: usize,
) -> LuaResult<bool> {
    let mm = helper::get_metamethod_event(lua_state, &operand, tm);
    if let Some(mm) = mm
        && mm.is_lua_function()
    {
        push_lua_mm_frame(lua_state, mm, operand, operand, frame_idx)?;
        return Ok(true);
    }
    Ok(false)
}

/// Cold helper: handle C function metamethod in MmBin.
/// Calls the metamethod and stores the result in the target register.
#[cold]
#[inline(never)]
pub fn call_c_mm_bin(
    lua_state: &mut LuaState,
    mm: LuaValue,
    v1: LuaValue,
    v2: LuaValue,
    result_reg: usize,
    frame_idx: usize,
) -> LuaResult<()> {
    let result = match super::metamethod::call_tm_res(lua_state, mm, v1, v2) {
        Ok(r) => r,
        Err(LuaError::Yield) => {
            use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
            let ci = lua_state.get_call_info_mut(frame_idx);
            ci.pending_finish_get = result_reg as i32;
            ci.call_status |= CIST_PENDING_FINISH;
            return Err(LuaError::Yield);
        }
        Err(e) => return Err(e),
    };
    let cur_base = lua_state.get_frame_base(frame_idx);
    unsafe {
        *lua_state
            .stack_mut()
            .get_unchecked_mut(cur_base + result_reg) = result;
    }
    Ok(())
}

/// Integer for-loop preparation — extracted to reduce main loop code size.
/// ForPrep only executes once per loop, so function call overhead is negligible.
/// Extracting this prevents LLVM from clobbering r12/r15 in the main dispatch.
#[inline(never)]
pub fn handle_forprep_int(
    lua_state: &mut LuaState,
    ra: usize,
    bx: usize,
    frame_idx: usize,
    pc: &mut usize,
) -> LuaResult<()> {
    let stack = lua_state.stack_mut();

    // Convert string values to numbers (Lua 5.5 allows string for-loop params)
    for offset in 0..3 {
        let val = unsafe { *stack.get_unchecked(ra + offset) };
        if val.is_string() {
            let num = parse_lua_number(val.as_str().unwrap_or(""));
            if !num.is_nil() {
                unsafe { *stack.get_unchecked_mut(ra + offset) = num };
            }
        }
    }

    if ttisinteger(unsafe { stack.get_unchecked(ra) })
        && ttisinteger(unsafe { stack.get_unchecked(ra + 2) })
    {
        // Integer loop (init and step are integers)
        let init = ivalue(unsafe { stack.get_unchecked(ra) });
        let step = ivalue(unsafe { stack.get_unchecked(ra + 2) });

        if step == 0 {
            lua_state.set_frame_pc(frame_idx, *pc as u32);
            return Err(lua_state.error("'for' step is zero".to_string()));
        }

        // forlimit: convert limit to integer per C Lua 5.5 logic
        let (limit, should_skip) = 'forlimit: {
            // Try integer limit directly
            if ttisinteger(unsafe { stack.get_unchecked(ra + 1) }) {
                let lim = ivalue(unsafe { stack.get_unchecked(ra + 1) });
                let skip = if step > 0 { init > lim } else { init < lim };
                break 'forlimit (lim, skip);
            }
            // Try converting to float (handles float and string)
            let mut flimit = 0.0;
            let limit_val = unsafe { *stack.get_unchecked(ra + 1) };
            if !tonumberns(&limit_val, &mut flimit) {
                lua_state.set_frame_pc(frame_idx, *pc as u32);
                return Err(cold_error_for_bad_limit(lua_state, &limit_val));
            }
            // Try rounding the float to integer
            let nl = if step < 0 {
                flimit.ceil()
            } else {
                flimit.floor()
            };
            // Check if the rounded float fits in i64
            if nl >= (i64::MIN as f64) && nl < -(i64::MIN as f64) && !nl.is_nan() {
                let lim = nl as i64;
                let skip = if step > 0 { init > lim } else { init < lim };
                break 'forlimit (lim, skip);
            }
            // Float is out of integer range — use C Lua overflow logic
            if flimit > 0.0 {
                if step < 0 {
                    break 'forlimit (0, true);
                }
                break 'forlimit (i64::MAX, false);
            } else {
                if step > 0 {
                    break 'forlimit (0, true);
                }
                break 'forlimit (i64::MIN, false);
            }
        };

        let stack = lua_state.stack_mut();
        if should_skip {
            *pc += bx + 1;
        } else {
            let count = if step > 0 {
                ((limit as u64).wrapping_sub(init as u64)) / (step as u64)
            } else {
                let step_abs = if step == i64::MIN {
                    i64::MAX as u64 + 1
                } else {
                    (-step) as u64
                };
                ((init as u64).wrapping_sub(limit as u64)) / step_abs
            };

            setivalue(unsafe { stack.get_unchecked_mut(ra) }, count as i64);
            setivalue(unsafe { stack.get_unchecked_mut(ra + 1) }, step);
            setivalue(unsafe { stack.get_unchecked_mut(ra + 2) }, init);
        }
    } else {
        // Float loop — delegate to existing handler
        handle_forprep_float(lua_state, ra, bx, frame_idx, pc)?;
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn cold_error_for_bad_limit(lua_state: &mut LuaState, limit_val: &LuaValue) -> LuaError {
    let t = crate::stdlib::debug::objtypename(lua_state, limit_val);
    lua_state.error(format!("bad 'for' limit (number expected, got {})", t))
}

/// Cold GC barrier for SETUPVAL — keeps the barrier code out of the hot dispatch path.
/// Only called when the assigned value is collectable.
#[cold]
#[inline(never)]
pub fn setupval_gc_barrier(lua_state: &mut LuaState, upval_ptr: UpvaluePtr, value: LuaValue) {
    if let Some(gc_ptr) = value.as_gc_ptr() {
        lua_state.gc_barrier(upval_ptr, gc_ptr);
    }
}
