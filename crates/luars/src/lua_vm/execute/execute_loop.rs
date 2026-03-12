use crate::{
    CallInfo, Instruction, LUA_MASKCALL, LUA_MASKCOUNT, LuaResult, LuaState, LuaValue, OpCode,
    lua_vm::{
        LuaError,
        call_info::call_status::{self, CIST_C, CIST_PENDING_FINISH},
        execute::{
            helper::{
                finishget, finishset, handle_pending_ops, setbfvalue, setbtvalue, setfltvalue,
                setivalue, setnilvalue, setobj2s, setobjs2s,
            },
            hook::{hook_check_instruction, hook_on_call},
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

        let base = ci.base;
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

        // MAINLOOP: Main instruction dispatch loop
        loop {
            let instr = unsafe { *code.get_unchecked(pc) }; // vmfetch
            pc += 1;

            if trap {
                hook_check_instruction(lua_state, pc, chunk, ci)?;
                updatetrap!();
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
                        ci.save_pc(pc);
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

                    ci.save_pc(pc);
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

                    let stack = lua_state.stack();
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

                    ci.save_pc(pc);
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
                        ci.save_pc(pc);
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
                    // luaV_fastget
                    if upval_value.is_table() {
                        let table = upval_value.hvalue_mut();
                        let native = &mut table.impl_table;
                        if native.has_hash() && native.set_shortstr_unchecked(&key, rc) {
                            if rc.is_collectable()
                                && let Some(gc_ptr) = upval_value.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                    }

                    ci.save_pc(pc);
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
                    let rc = k_val!(instr.get_c());
                    // if unsafe { (*ra_ptr).tt } == LUA_VTABLE {
                    //     let table_gc = unsafe { &mut *((*ra_ptr).value.ptr as *mut GcTable) };
                    //     let table_ref = &mut table_gc.data;
                    //     if !table_ref.has_metatable() {
                    //         // No metatable: try integer fast path first (t[i] = v)
                    //         if unsafe { pttisinteger(rb_ptr) } {
                    //             let ikey = unsafe { (*rb_ptr).value.i };
                    //             if unsafe { table_ref.impl_table.fast_seti_ptr(ikey, val_ptr) } {
                    //                 if unsafe { (*val_ptr).tt } & 0x40 != 0 {
                    //                     lua_state.gc_barrier_back(unsafe {
                    //                         (*ra_ptr).as_gc_ptr_table_unchecked()
                    //                     });
                    //                 }
                    //                 continue;
                    //             }
                    //             let val = unsafe { *val_ptr };
                    //             let delta = table_ref.impl_table.set_int_slow(ikey, val);
                    //             if delta != 0 {
                    //                 lua_state.gc_track_table_resize(
                    //                     unsafe { (*ra_ptr).as_table_ptr_unchecked() },
                    //                     delta,
                    //                 );
                    //             }
                    //             if val.is_collectable() {
                    //                 lua_state.gc_barrier_back(unsafe {
                    //                     (*ra_ptr).as_gc_ptr_table_unchecked()
                    //                 });
                    //             }
                    //             continue;
                    //         }
                    //         // Non-integer key: validate then raw_set
                    //         let rb = unsafe { *rb_ptr };
                    //         if rb.is_nil() {
                    //             return Err(cold::error_table_index_nil(lua_state));
                    //         }
                    //         if rb.ttisfloat() && rb.fltvalue().is_nan() {
                    //             return Err(cold::error_table_index_nan(lua_state));
                    //         }
                    //         let ra = unsafe { *ra_ptr };
                    //         let val = unsafe { *val_ptr };
                    //         lua_state.raw_set(&ra, rb, val);
                    //         continue;
                    //     }
                    //     // Has metatable: if integer key with existing non-nil value
                    //     // in array, __newindex is NOT consulted
                    //     if unsafe { pttisinteger(rb_ptr) } {
                    //         let val = unsafe { *val_ptr };
                    //         if table_ref
                    //             .impl_table
                    //             .fast_seti_existing(unsafe { (*rb_ptr).value.i }, val)
                    //         {
                    //             if val.is_collectable() {
                    //                 lua_state.gc_barrier_back(unsafe {
                    //                     (*ra_ptr).as_gc_ptr_table_unchecked()
                    //                 });
                    //             }
                    //             continue;
                    //         }
                    //     }
                    //     // Generic non-integer existing key check
                    //     let rb = unsafe { *rb_ptr };
                    //     if let Some(existing) = table_ref.impl_table.raw_get(&rb)
                    //         && !existing.is_nil()
                    //     {
                    //         let ra = unsafe { *ra_ptr };
                    //         let val = unsafe { *val_ptr };
                    //         lua_state.raw_set(&ra, rb, val);
                    //         continue;
                    //     }
                    //     // Noinline __newindex fast path
                    //     let meta = table_ref.meta_ptr();
                    //     if !meta.is_null() {
                    //         save_pc!();
                    //         let ra = unsafe { *ra_ptr };
                    //         let val = unsafe { *val_ptr };
                    //         if noinline::try_newindex_meta(lua_state, meta, ra, rb, val, frame_idx)?
                    //         {
                    //             continue 'startfunc;
                    //         }
                    //     }
                    // }

                    // // Cold: metatable __newindex chain or non-table
                    // save_pc!();
                    // let mut pc_idx = pc;
                    // table_ops::exec_settable(
                    //     lua_state,
                    //     instr,
                    //     constants,
                    //     base,
                    //     frame_idx,
                    //     &mut pc_idx,
                    // )?;
                    // pc = pc_idx;
                }
                _ => {}
            }
        }
    }
}
