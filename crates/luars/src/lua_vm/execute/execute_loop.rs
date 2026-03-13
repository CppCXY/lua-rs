use crate::{
    CallInfo, Instruction, LUA_MASKCALL, LUA_MASKCOUNT, LuaResult, LuaState, LuaValue, OpCode,
    lua_vm::{
        LuaError, TmKind,
        call_info::call_status::{CIST_C, CIST_PENDING_FINISH},
        execute::{
            cold::{self},
            concat::concat,
            helper::{
                equalobj, finishget, finishset, fltvalue, handle_pending_ops, ivalue, lua_fmod,
                lua_idiv, lua_imod, lua_shiftl, lua_shiftr, luai_numpow, objlen, pfltvalue,
                pivalue, psetfltvalue, psetivalue, ptonumberns, pttisfloat, pttisinteger,
                setbfvalue, setbtvalue, setfltvalue, setivalue, setnilvalue, setobj2s, setobjs2s,
                tointeger, tointegerns, tonumberns, ttisfloat, ttisinteger, ttisstring,
            },
            hook::{hook_check_instruction, hook_on_call},
            metamethod::{try_bin_tm, try_comp_tm, try_unary_tm},
            number::{le_num, lt_num},
        },
        lua_limits::EXTRA_STACK,
    },
};

/// Execute until call depth reaches target_depth
/// Used for protected calls (pcall) to execute only the called function
/// without affecting caller frames
///
/// NOTE: n_ccalls tracking is NOT done here (unlike the wrapper approach).
/// Instead, each recursive CALL SITE (metamethods, pcall, resume, __close)
/// increments/decrements n_ccalls around its call to lua_execute, mirroring
/// Lua 5.5's luaD_call pattern.
#[allow(unused)]
pub fn lua_execute(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    // STARTFUNC: Function context switching point (like Lua C's startfunc label)
    'startfunc: loop {
        // Check if we've returned past target depth.
        let current_depth = lua_state.call_depth();
        if current_depth <= target_depth {
            return Ok(());
        }

        let frame_idx = current_depth - 1;
        let ci_ptr = unsafe { lua_state.get_call_info_ptr(frame_idx) } as *mut CallInfo;
        let ci = unsafe { &mut *ci_ptr };
        let call_status = ci.call_status;
        if call_status & (CIST_C | CIST_PENDING_FINISH) != 0
            && handle_pending_ops(lua_state, frame_idx)?
        {
            continue 'startfunc;
        }

        let mut base = ci.base;
        let pc_init = ci.pc as usize;
        let chunk = unsafe { &*ci.chunk_ptr };
        debug_assert!(lua_state.stack_len() >= base + chunk.max_stack_size + EXTRA_STACK);

        let code: &[Instruction] = &chunk.code;
        let constants: &[LuaValue] = &chunk.constants;
        let mut pc: usize = pc_init;

        lua_state.oldpc = if pc_init > 0 {
            (pc_init - 1) as u32
        } else if chunk.is_vararg {
            0
        } else {
            u32::MAX
        };

        // CALL HOOK: fire when entering a new Lua function (pc == 0)
        let mut trap = lua_state.hook_mask != 0;
        if pc == 0 && trap {
            let hook_mask = lua_state.hook_mask;
            if hook_mask & LUA_MASKCALL != 0 && lua_state.allow_hook {
                hook_on_call(lua_state, hook_mask, call_status, chunk)?;
            }
            if hook_mask & LUA_MASKCOUNT != 0 {
                lua_state.hook_count = lua_state.base_hook_count;
            }
        }

        macro_rules! stack_id {
            ($a:expr) => {
                base + $a as usize
            };
        }

        macro_rules! stack_val_mut {
            ($a:expr) => {
                unsafe { lua_state.stack_mut().get_unchecked_mut(stack_id!($a)) }
            };
        }

        macro_rules! stack_val {
            ($a:expr) => {
                unsafe { lua_state.stack().get_unchecked(stack_id!($a)) }
            };
        }

        macro_rules! k_val {
            ($a:expr) => {
                unsafe { constants.get_unchecked($a as usize) }
            };
        }

        macro_rules! upval_value {
            ($b:expr) => {
                unsafe { *ci.upvalue_ptrs.add($b as usize) }
                    .as_ref()
                    .data
                    .get_value_ref()
            };
        }

        macro_rules! updatetrap {
            () => {
                trap = lua_state.hook_mask != 0;
            };
        }

        macro_rules! updatebase {
            () => {
                base = ci.base;
            };
        }

        macro_rules! savestate {
            () => {
                ci.save_pc(pc);
                lua_state.set_top_raw(ci.top as usize);
            };
        }

        // MAINLOOP: Main instruction dispatch loop
        loop {
            let instr = unsafe { *code.get_unchecked(pc) }; // vmfetch
            pc += 1;

            if trap {
                trap = hook_check_instruction(lua_state, pc, chunk, ci)?;
                updatebase!();
            }

            match instr.get_opcode() {
                OpCode::Move => {
                    // R[A] := R[B]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    setobjs2s(lua_state, stack_id!(a), stack_id!(b));
                }
                OpCode::LoadI => {
                    // R[A] := sBx
                    let a = instr.get_a();
                    let sbx = instr.get_sbx();
                    setivalue(stack_val_mut!(a), sbx as i64);
                }
                OpCode::LoadF => {
                    // R[A] := (float)sBx
                    let a = instr.get_a();
                    let sbx = instr.get_sbx();
                    setfltvalue(stack_val_mut!(a), sbx as f64);
                }
                OpCode::LoadK => {
                    // R[A] := K[Bx]
                    let a = instr.get_a();
                    let bx = instr.get_bx();
                    setobj2s(lua_state, stack_id!(a), k_val!(bx));
                }
                OpCode::LoadKX => {
                    // R[A] := K[extra arg]
                    let a = instr.get_a();
                    let next_instr = unsafe { *code.get_unchecked(pc) };
                    debug_assert_eq!(next_instr.get_opcode(), OpCode::ExtraArg);
                    let rb = next_instr.get_ax();
                    pc += 1;
                    setobj2s(lua_state, stack_id!(a), k_val!(rb));
                }
                OpCode::LoadFalse => {
                    // R[A] := false
                    let a = instr.get_a();
                    setbfvalue(stack_val_mut!(a));
                }
                OpCode::LFalseSkip => {
                    // R[A] := false; pc++
                    let a = instr.get_a();
                    setbfvalue(stack_val_mut!(a));
                    pc += 1; // Skip next instruction
                }
                OpCode::LoadTrue => {
                    // R[A] := true
                    let a = instr.get_a();
                    setbtvalue(stack_val_mut!(a));
                }
                OpCode::LoadNil => {
                    // R[A], R[A+1], ..., R[A+B] := nil
                    let mut a = instr.get_a();
                    let mut b = instr.get_b();
                    loop {
                        setnilvalue(stack_val_mut!(a));
                        if b == 0 {
                            break;
                        }
                        b -= 1;
                        a += 1;
                    }
                }
                OpCode::GetUpval => {
                    // R[A] := UpValue[B]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    setobj2s(lua_state, stack_id!(a), upval_value!(b));
                }
                OpCode::SetUpval => {
                    // UpValue[B] := R[A]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    unsafe {
                        let upvalue_ptr = *ci.upvalue_ptrs.add(b as usize);
                        let value = lua_state.stack().get_unchecked(base + a as usize);
                        upvalue_ptr.as_mut_ref().data.set_value(*value);

                        // GC barrier (only for collectable values)
                        if value.is_collectable()
                            && let Some(gc_ptr) = value.as_gc_ptr()
                        {
                            lua_state.gc_barrier(upvalue_ptr, gc_ptr);
                        }
                    }
                }
                OpCode::GetTabUp => {
                    // R[A] := UpValue[B][K[C]:shortstring]
                    let upval_value = upval_value!(instr.get_b()).clone();
                    let key = k_val!(instr.get_c());
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    // luaV_fastget
                    let result = if upval_value.is_table() {
                        let table = upval_value.hvalue();
                        table.impl_table.get_shortstr_unchecked(key)
                    } else {
                        None
                    };

                    if let Some(value) = result {
                        setobj2s(lua_state, stack_id!(instr.get_a()), &value);
                    } else {
                        // Protect(luaV_finishget(L, upval, rc, ra, tag));
                        savestate!();
                        match finishget(lua_state, &upval_value, key) {
                            Ok(result) => {
                                updatetrap!();
                                setobj2s(
                                    lua_state,
                                    stack_id!(instr.get_a()),
                                    &result.unwrap_or(LuaValue::nil()),
                                );
                            }
                            Err(LuaError::Yield) => {
                                ci.pending_finish_get = stack_id!(instr.get_a()) as i32;
                                ci.call_status |= CIST_PENDING_FINISH;
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                OpCode::GetTable => {
                    // GETTABLE: R[A] := R[B][R[C]]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();

                    let stack = lua_state.stack();
                    let rb = unsafe { stack.get_unchecked(stack_id!(b)) }.clone();
                    let rc = unsafe { stack.get_unchecked(stack_id!(c)) }.clone();
                    if rc.ttisinteger() && rb.is_table() {
                        // fast_geti
                        let table = rb.hvalue();
                        unsafe {
                            table.impl_table.fast_geti_into(
                                rc.ivalue(),
                                lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)),
                            );
                        }
                        continue;
                    } else if let Some(table) = rb.as_table() {
                        // fast_get with non-integer key
                        if let Some(val) = table.impl_table.raw_get(&rc) {
                            setobj2s(lua_state, stack_id!(a), &val);
                            continue;
                        }
                    }

                    savestate!();
                    let result = match finishget(lua_state, &rb, &rc) {
                        Ok(result) => {
                            updatetrap!();
                            result
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = a as i32;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    };

                    setobj2s(lua_state, stack_id!(a), &result.unwrap_or(LuaValue::nil()));
                }
                OpCode::GetI => {
                    // GETI: R[A] := R[B][C] (integer key)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let rc = instr.get_c() as i64;
                    let rb = stack_val!(b).clone();
                    if rb.is_table() {
                        // fast_geti
                        let table = rb.hvalue();
                        unsafe {
                            table.impl_table.fast_geti_into(
                                rc,
                                lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)),
                            );
                        }
                        continue;
                    }

                    savestate!();
                    let result = match finishget(lua_state, &rb, &LuaValue::integer(rc)) {
                        Ok(result) => {
                            updatetrap!();
                            result
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = a as i32;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    };

                    setobj2s(lua_state, stack_id!(a), &result.unwrap_or(LuaValue::nil()));
                }
                OpCode::GetField => {
                    // GETFIELD: R[A] := R[B][K[C]:string]
                    let rb = stack_val!(instr.get_b()).clone();
                    let key = k_val!(instr.get_c());
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    // luaV_fastget
                    let result = if rb.is_table() {
                        let table = rb.hvalue();
                        table.impl_table.get_shortstr_unchecked(key)
                    } else {
                        None
                    };

                    if let Some(value) = result {
                        setobj2s(lua_state, stack_id!(instr.get_a()), &value);
                    } else {
                        // Protect(luaV_finishget(L, upval, rc, ra, tag));
                        savestate!();
                        match finishget(lua_state, &rb, key) {
                            Ok(result) => {
                                updatetrap!();
                                setobj2s(
                                    lua_state,
                                    stack_id!(instr.get_a()),
                                    &result.unwrap_or(LuaValue::nil()),
                                );
                            }
                            Err(LuaError::Yield) => {
                                ci.pending_finish_get = stack_id!(instr.get_a()) as i32;
                                ci.call_status |= CIST_PENDING_FINISH;
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                OpCode::SetTabUp => {
                    // UpValue[A][K[B]:shortstring] := RK(C)
                    let upval_value = upval_value!(instr.get_b()).clone();
                    let key = k_val!(instr.get_b());
                    let rc = unsafe { lua_state.stack().get_unchecked(stack_id!(instr.get_c())) }
                        .clone();
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    let mut hres = false;
                    // luaV_fastget
                    if upval_value.is_table() {
                        let table = upval_value.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let native = &mut table.impl_table;
                            if native.has_hash() && native.set_shortstr_unchecked(&key, rc) {
                                hres = true;
                            }
                        }
                    }

                    if hres {
                        if rc.is_collectable()
                            && let Some(gc_ptr) = upval_value.as_gc_ptr()
                        {
                            lua_state.gc_barrier_back(gc_ptr);
                        }
                        continue;
                    }

                    savestate!();
                    match finishset(lua_state, &upval_value, &key, rc) {
                        Ok(_) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = -2;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::SetTable => {
                    // SETTABLE: R[A][R[B]] := RK(C)
                    let ra = stack_val!(instr.get_a());
                    let rb = stack_val!(instr.get_b());
                    let rc = k_val!(instr.get_c()).clone();

                    if ra.is_table() {
                        let mut hres = false;
                        let table = ra.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let native = &mut table.impl_table;
                            if rb.ttisinteger() {
                                if native.fast_seti(rb.ivalue(), rc) {
                                    hres = true;
                                }
                            } else if rb.is_short_string()
                                && native.has_hash()
                                && native.set_shortstr_unchecked(rb, rc)
                            {
                                hres = true;
                            } else {
                                let rb_clone = rb.clone();
                                if native.has_hash() && native.raw_set(&rb_clone, rc).0 {
                                    hres = true;
                                }
                            }
                        }

                        if hres {
                            if rc.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                    }
                    let ra = ra.clone();
                    let rb = rb.clone();
                    savestate!();
                    match finishset(lua_state, &ra, &rb, rc) {
                        Ok(_) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = -2;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::SetI => {
                    // SETI: R[A][B] := RK(C) (integer key)
                    // Pointer-based: avoid 16B LuaValue copies on fast path
                    let ra = stack_val!(instr.get_a());
                    let b = instr.get_b() as i64;
                    let rc = k_val!(instr.get_c()).clone();

                    if ra.is_table() {
                        let mut hres = false;
                        let table = ra.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let native = &mut table.impl_table;
                            if native.fast_seti(b, rc) {
                                hres = true;
                            }
                        }

                        if hres {
                            if rc.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                    }
                    let ra = ra.clone();
                    let rb = LuaValue::integer(b);
                    savestate!();
                    match finishset(lua_state, &ra, &rb, rc) {
                        Ok(_) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = -2;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::SetField => {
                    // SETFIELD: R[A][K[B]:string] := RK(C)
                    let ra = stack_val!(instr.get_a());
                    let rb = k_val!(instr.get_b());
                    let rc = k_val!(instr.get_c()).clone();
                    debug_assert!(
                        rb.is_short_string(),
                        "SetField key must be short string for fast path"
                    );
                    if ra.is_table() {
                        let mut hres = false;
                        let table = ra.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let native = &mut table.impl_table;
                            if native.has_hash() && native.set_shortstr_unchecked(rb, rc) {
                                hres = true;
                            }
                        }

                        if hres {
                            if rc.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                    }
                    let ra = ra.clone();
                    let rb = rb.clone();
                    savestate!();
                    match finishset(lua_state, &ra, &rb, rc) {
                        Ok(_) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = -2;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::NewTable => {
                    // R[A] := {} (new table) — table ops should be inlined
                    let a = instr.get_a();
                    let mut vb = instr.get_vb();
                    let mut vc = instr.get_vc();
                    let k = instr.get_k();

                    vb = if vb > 0 {
                        if vb > 31 { 0 } else { 1 << (vb - 1) }
                    } else {
                        0
                    };

                    if k {
                        let extra_instr = unsafe { *code.get_unchecked(pc) };
                        if extra_instr.get_opcode() == OpCode::ExtraArg {
                            vc += extra_instr.get_ax() * 1024;
                        }
                    }

                    pc += 1; // skip EXTRAARG

                    let value = lua_state.create_table(vc as usize, vb as usize)?;
                    setobj2s(lua_state, stack_id!(a), &value);

                    let new_top = base + a as usize + 1;
                    // ci.save_pc(pc);
                    // lua_state.set_top_raw(new_top);
                    // lua_state.check_gc()?;
                    // let frame_top = ci.top;
                    // lua_state.set_top_raw(frame_top as usize);
                    lua_state.check_gc_in_loop(ci, pc, new_top, &mut trap);
                }
                OpCode::Self_ => {
                    // SELF: R[A+1] := R[B]; R[A] := R[B][K[C]:string]
                    let a = instr.get_a();
                    let rb = stack_val!(instr.get_b()).clone();
                    let key = k_val!(instr.get_c());

                    debug_assert!(
                        key.is_short_string(),
                        "Self key must be short string for fast path"
                    );
                    setobj2s(lua_state, stack_id!(a + 1), &rb);
                    // Fast path: rb is a table
                    if rb.ttistable() {
                        let table = rb.hvalue();
                        if let Some(val) = table.impl_table.get_shortstr_fast(key) {
                            setobj2s(lua_state, stack_id!(a), &val);
                            continue;
                        }
                    }

                    savestate!();
                    match finishget(lua_state, &rb, key) {
                        Ok(result) => {
                            updatetrap!();
                            setobj2s(lua_state, stack_id!(a), &result.unwrap_or(LuaValue::nil()));
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = a as i32;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    };
                }
                OpCode::Add => {
                    // op_arith(L, l_addi, luai_numadd)
                    // R[A] := R[B] + R[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_add(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) + pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 + n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::AddI => {
                    // op_arithI(L, l_addi, luai_numadd)
                    // R[A] := R[B] + sC
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let sc = instr.get_sc();

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        // Fast path: integer (most common)
                        if pttisinteger(v1_ptr) {
                            let iv1 = pivalue(v1_ptr);
                            psetivalue(ra_ptr, iv1.wrapping_add(sc as i64));
                            pc += 1; // Skip metamethod on success
                        }
                        // Slow path: float
                        else if pttisfloat(v1_ptr) {
                            let nb = pfltvalue(v1_ptr);
                            psetfltvalue(ra_ptr, nb + (sc as f64));
                            pc += 1; // Skip metamethod on success
                        }
                        // else: fall through to MMBINI (next instruction)
                    }
                }
                OpCode::Sub => {
                    // op_arith(L, l_subi, luai_numsub)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_sub(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) - pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 - n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Mul => {
                    // op_arith(L, l_muli, luai_nummul)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_mul(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) * pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 * n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Div => {
                    // op_arithf(L, luai_numdiv) - 浮点除法
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) / pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 / n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::IDiv => {
                    // op_arith(L, luaV_idiv, luai_numidiv) - 整数除法
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_idiv(i1, i2));
                                pc += 1;
                            } else {
                                ci.save_pc(pc);
                                return Err(cold::error_div_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, (pfltvalue(v1_ptr) / pfltvalue(v2_ptr)).floor());
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, (n1 / n2).floor());
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Mod => {
                    // op_arith(L, luaV_mod, luaV_modf)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_imod(i1, i2));
                                pc += 1;
                            } else {
                                ci.save_pc(pc);
                                return Err(cold::error_mod_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, lua_fmod(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, lua_fmod(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Pow => {
                    // op_arithf(L, luai_numpow)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, luai_numpow(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, luai_numpow(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::AddK => {
                    // op_arithK(L, l_addi, luai_numadd)
                    // R[A] := R[B] + K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_add(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) + pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 + n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::SubK => {
                    // R[A] := R[B] - K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_sub(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) - pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 - n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::MulK => {
                    // R[A] := R[B] * K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_mul(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) * pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 * n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::ModK => {
                    // R[A] := R[B] % K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_imod(i1, i2));
                                pc += 1;
                            } else {
                                ci.save_pc(pc);
                                return Err(cold::error_mod_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, lua_fmod(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, lua_fmod(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::PowK => {
                    // R[A] := R[B] ^ K[C] (always float)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, luai_numpow(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, luai_numpow(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::DivK => {
                    // R[A] := R[B] / K[C] (float division)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) / pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 / n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::IDivK => {
                    // R[A] := R[B] // K[C] (floor division)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_idiv(i1, i2));
                                pc += 1;
                            } else {
                                ci.save_pc(pc);
                                return Err(cold::error_div_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, (pfltvalue(v1_ptr) / pfltvalue(v2_ptr)).floor());
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, (n1 / n2).floor());
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::BAndK => {
                    // R[A] := R[B] & K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = k_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 & i2);
                    }
                }
                OpCode::BOrK => {
                    // R[A] := R[B] | K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = k_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 | i2);
                    }
                }
                OpCode::BXorK => {
                    // R[A] := R[B] ^ K[C] (bitwise xor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = k_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 ^ i2);
                    }
                }
                OpCode::BAnd => {
                    // op_bitwise(L, l_band)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 & i2);
                    }
                }
                OpCode::BOr => {
                    // op_bitwise(L, l_bor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 | i2);
                    }
                }
                OpCode::BXor => {
                    // op_bitwise(L, l_bxor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), i1 ^ i2);
                    }
                }
                OpCode::Shl => {
                    // op_bitwise(L, luaV_shiftl)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), lua_shiftl(i1, i2));
                    }
                }
                OpCode::Shr => {
                    // op_bitwise(L, luaV_shiftr)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = stack_val!(b);
                    let v2 = stack_val!(c);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(stack_val_mut!(a), lua_shiftr(i1, i2));
                    }
                }
                OpCode::ShlI => {
                    // R[A] := sC << R[B]
                    // Note: In Lua 5.5, SHLI is immediate << register (not register << immediate)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc(); // shift amount from immediate

                    let rb = stack_val!(b);

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftl(ic, ib): shift ic left by ib
                        setivalue(stack_val_mut!(a), lua_shiftl(ic as i64, ib));
                    }
                    // else: metamethod
                }
                OpCode::ShrI => {
                    // R[A] := R[B] >> sC
                    // Logical right shift (Lua 5.5: luaV_shiftr)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc(); // shift amount

                    let rb = stack_val!(b);

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftr(ib, ic) = luaV_shiftl(ib, -ic)
                        setivalue(stack_val_mut!(a), lua_shiftr(ib, ic as i64));
                    }
                    // else: metamethod
                }
                OpCode::MmBin => {
                    // Call metamethod over R[A] and R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let ra = *stack_val!(a);
                    let rb = *stack_val!(b);
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let result_reg = pi.get_a();

                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };

                    savestate!();
                    // Call metamethod (may change stack/base)
                    match try_bin_tm(lua_state, ra, rb, result_reg, tm) {
                        Ok(_) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = result_reg as i32;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    };
                }
                OpCode::MmBinI => {
                    // Call metamethod over R[A] and immediate sB
                    let a = instr.get_a() as usize;
                    let imm = instr.get_sb();
                    let flip = instr.get_k();

                    let ra = stack_val!(a);
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let result_reg = pi.get_a();

                    // Get tag method — unchecked since compiler guarantees valid TmKind in MMBIN instruction
                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };
                    let rb = LuaValue::integer(imm as i64);
                    let r = if flip { (rb, *ra) } else { (*ra, rb) };
                    savestate!();
                    // Call metamethod (may change stack/base)
                    match try_bin_tm(lua_state, r.0, r.1, result_reg, tm) {
                        Ok(_) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = result_reg as i32;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    };
                }
                OpCode::MmBinK => {
                    let ra = *stack_val!(instr.get_a());
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let imm = *k_val!(instr.get_b());
                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };
                    let flip = instr.get_k();
                    let result_reg = pi.get_a();

                    savestate!();
                    let r = if flip { (imm, ra) } else { (ra, imm) };
                    // Call metamethod (may change stack/base)
                    match try_bin_tm(lua_state, r.0, r.1, result_reg, tm) {
                        Ok(_) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.pending_finish_get = result_reg as i32;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    };
                }
                OpCode::Unm => {
                    // 取负: -value
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = stack_val!(b).clone();

                    if ttisinteger(&rb) {
                        let ib = ivalue(&rb);
                        setivalue(stack_val_mut!(a), ib.wrapping_neg());
                    } else {
                        let mut nb = 0.0;
                        if tonumberns(&rb, &mut nb) {
                            setfltvalue(stack_val_mut!(a), -nb);
                        } else {
                            savestate!();
                            let result_reg = stack_id!(a);
                            // Call metamethod (may change stack/base)
                            match try_unary_tm(lua_state, rb, result_reg, TmKind::Unm) {
                                Ok(_) => {
                                    updatetrap!();
                                }
                                Err(LuaError::Yield) => {
                                    ci.pending_finish_get = result_reg as i32;
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            };
                        }
                    }
                }
                OpCode::BNot => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = stack_val!(b).clone();

                    let mut ib = 0i64;
                    if tointegerns(&rb, &mut ib) {
                        setivalue(stack_val_mut!(a), !ib);
                    } else {
                        // Try non-recursive __bnot for tables/userdata
                        savestate!();
                        let result_reg = stack_id!(a);
                        // Fall through to recursive path (C function mm or error)
                        match try_unary_tm(lua_state, rb, result_reg, TmKind::Bnot) {
                            Ok(_) => {
                                updatetrap!();
                            }
                            Err(LuaError::Yield) => {
                                ci.pending_finish_get = result_reg as i32;
                                ci.call_status |= CIST_PENDING_FINISH;
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                OpCode::Not => {
                    // R[A] := not R[B]
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = stack_val!(b);
                    if rb.ttisfalse() || rb.is_nil() {
                        setbtvalue(stack_val_mut!(a));
                    } else {
                        setbfvalue(stack_val_mut!(a));
                    }
                }
                OpCode::Len => {
                    // HOT PATH: inline table length for no-metatable case
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let rb = *stack_val!(b);
                    objlen(lua_state, stack_id!(a), rb)?;
                }
                OpCode::Concat => {
                    let a = instr.get_a();
                    let n = instr.get_b();
                    let concat_top = base + (a + n) as usize;
                    lua_state.set_top_raw(concat_top);

                    // ProtectNT
                    ci.save_pc(pc);
                    concat(lua_state, n as usize)?;
                    updatetrap!();

                    let top = lua_state.get_top();
                    lua_state.check_gc_in_loop(ci, pc, top, &mut trap);
                }
                OpCode::Close => {
                    let a = instr.get_a();
                    let close_from = stack_id!(a);

                    ci.save_pc(pc);
                    match lua_state.close_all(close_from) {
                        Ok(()) => {}
                        Err(LuaError::Yield) => {
                            ci.pc -= 1;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::Tbc => {
                    // Mark variable as to-be-closed
                    let a = instr.get_a();
                    lua_state.mark_tbc(stack_id!(a))?;
                }
                OpCode::Jmp => {
                    let sj = instr.get_sj();
                    pc = (pc as isize + sj as isize) as usize;
                    updatetrap!();
                }
                OpCode::Eq => {
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let ra = *stack_val!(a);
                    let rb = *stack_val!(b);
                    savestate!();
                    let cond = match equalobj(lua_state, ra, rb) {
                        Ok(eq) => eq,
                        Err(LuaError::Yield) => {
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    };
                    updatetrap!();
                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::Lt => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = unsafe { stack.get_unchecked(stack_id!(a)) };
                        let rb = unsafe { stack.get_unchecked(stack_id!(b)) };

                        if ttisinteger(ra) && ttisinteger(rb) {
                            ivalue(ra) < ivalue(rb)
                        } else if (ra.is_number() && rb.is_number()) {
                            lt_num(&ra, &rb)
                        } else if ttisstring(ra) && ttisstring(rb) {
                            // contain binary string comparison
                            // String comparison
                            let sa = ra.as_str_bytes();
                            let sb = rb.as_str_bytes();

                            if let (Some(sa), Some(sb)) = (sa, sb) {
                                sa < sb
                            } else {
                                false
                            }
                        } else {
                            let va = *ra;
                            let vb = *rb;
                            savestate!();
                            match try_comp_tm(lua_state, va, vb, TmKind::Lt) {
                                Ok(Some(result)) => {
                                    updatetrap!();
                                    result
                                }
                                Ok(None) => {
                                    return Err(crate::stdlib::debug::ordererror(
                                        lua_state, &va, &vb,
                                    ));
                                }
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }

                OpCode::Le => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = unsafe { stack.get_unchecked(stack_id!(a)) };
                        let rb = unsafe { stack.get_unchecked(stack_id!(b)) };

                        if ttisinteger(ra) && ttisinteger(rb) {
                            ivalue(ra) <= ivalue(rb)
                        } else if (ra.is_number() && rb.is_number()) {
                            le_num(&ra, &rb)
                        } else if ttisstring(ra) && ttisstring(rb) {
                            // contain binary string comparison
                            // String comparison
                            let sa = ra.as_str_bytes();
                            let sb = rb.as_str_bytes();

                            if let (Some(sa), Some(sb)) = (sa, sb) {
                                sa <= sb
                            } else {
                                false
                            }
                        } else {
                            let va = *ra;
                            let vb = *rb;
                            savestate!();
                            match try_comp_tm(lua_state, va, vb, TmKind::Le) {
                                Ok(Some(result)) => {
                                    updatetrap!();
                                    result
                                }
                                Ok(None) => {
                                    return Err(crate::stdlib::debug::ordererror(
                                        lua_state, &va, &vb,
                                    ));
                                }
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::EqK => {
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let k = instr.get_k();

                    let ra = stack_val!(a);
                    let rb = k_val!(b);
                    // Raw equality (no metamethods for constants)
                    let cond = ra == rb;
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }

                OpCode::EqI => {
                    let a = instr.get_a();
                    let im = instr.get_sb();
                    let ra = stack_val!(a);
                    let cond = if ttisinteger(ra) {
                        ivalue(ra) == im as i64
                    } else if ttisfloat(ra) {
                        fltvalue(ra) == im as f64
                    } else {
                        false
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }

                OpCode::LtI => {
                    let a = instr.get_a();
                    let im = instr.get_sb();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = unsafe { stack.get_unchecked(stack_id!(a)) };

                        if ttisinteger(ra) {
                            ivalue(ra) < im as i64
                        } else if ttisfloat(ra) {
                            fltvalue(ra) < im as f64
                        } else {
                            let va = *ra;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            match try_comp_tm(lua_state, va, vb, TmKind::Lt) {
                                Ok(Some(result)) => {
                                    updatetrap!();
                                    result
                                }
                                Ok(None) => {
                                    return Err(crate::stdlib::debug::ordererror(
                                        lua_state, &va, &vb,
                                    ));
                                }
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::LeI => {
                    let a = instr.get_a();
                    let im = instr.get_sb();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = unsafe { stack.get_unchecked(stack_id!(a)) };

                        if ttisinteger(ra) {
                            ivalue(ra) <= im as i64
                        } else if ttisfloat(ra) {
                            fltvalue(ra) <= im as f64
                        } else {
                            let va = *ra;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            match try_comp_tm(lua_state, va, vb, TmKind::Le) {
                                Ok(Some(result)) => {
                                    updatetrap!();
                                    result
                                }
                                Ok(None) => {
                                    return Err(crate::stdlib::debug::ordererror(
                                        lua_state, &va, &vb,
                                    ));
                                }
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }

                OpCode::GtI => {
                    let a = instr.get_a();
                    let im = instr.get_sb();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = unsafe { stack.get_unchecked(stack_id!(a)) };

                        if ttisinteger(ra) {
                            ivalue(ra) > im as i64
                        } else if ttisfloat(ra) {
                            fltvalue(ra) > im as f64
                        } else {
                            let va = *ra;
                            let vb = LuaValue::integer(im as i64);
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            match try_comp_tm(lua_state, vb, va, TmKind::Lt) {
                                Ok(Some(result)) => {
                                    updatetrap!();
                                    result
                                }
                                Ok(None) => {
                                    return Err(crate::stdlib::debug::ordererror(
                                        lua_state, &va, &vb,
                                    ));
                                }
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }

                OpCode::GeI => {
                    let a = instr.get_a();
                    let im = instr.get_sb();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = unsafe { stack.get_unchecked(stack_id!(a)) };

                        if ttisinteger(ra) {
                            ivalue(ra) >= im as i64
                        } else if ttisfloat(ra) {
                            fltvalue(ra) >= im as f64
                        } else {
                            let va = *ra;
                            let vb = LuaValue::integer(im as i64);
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            match try_comp_tm(lua_state, vb, va, TmKind::Le) {
                                Ok(Some(result)) => {
                                    updatetrap!();
                                    result
                                }
                                Ok(None) => {
                                    return Err(crate::stdlib::debug::ordererror(
                                        lua_state, &va, &vb,
                                    ));
                                }
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::Test => {
                    let a = instr.get_a();
                    let ra = stack_val!(a);
                    // l_isfalse: nil or false
                    let cond = !ra.is_nil() || ra.ttisfalse();

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = unsafe { *code.get_unchecked(pc) };
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }

                OpCode::TestSet => {
                    // if (l_isfalse(R[B]) == k) then pc++ else R[A] := R[B]; donextjump
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let k = instr.get_k();

                    let rb = *stack_val!(b);
                    let cond = rb.is_nil() || rb.ttisfalse();
                    if cond == k {
                        pc += 1; // Condition failed - skip next instruction (JMP)
                    } else {
                        // Condition succeeded - copy value and EXECUTE next instruction (must be JMP)
                        setobj2s(lua_state, stack_id!(a), &rb);
                        // donextjump: fetch and execute next JMP instruction
                        let next_instr = unsafe { *code.get_unchecked(pc) };
                        debug_assert!(next_instr.get_opcode() == OpCode::Jmp);
                        pc += 1; // Move past the JMP instruction
                        let sj = next_instr.get_sj();
                        pc = (pc as isize + sj as isize) as usize; // Execute the jump
                        updatetrap!();
                    }
                }
                _ => {}
            }
        }
    }
}
