/*----------------------------------------------------------------------
  Lua 5.5 VM Execution Engine

  Design Philosophy:
  1. **Slice-Based**: Code and constants accessed via `&[T]` slices with
     `noalias` guarantees — LLVM keeps slice base pointers in registers
     across function calls (raw pointers must be reloaded after `&mut` calls)
  2. **Minimal Indirection**: Use get_unchecked for stack access (no bounds checks)
  4. **CPU Register Optimization**: code, constants, pc, base, trap in CPU registers
  5. **Unsafe but Sound**: Use raw pointers with invariant guarantees for stack

  Key Invariants (maintained by caller):
  - Stack pointer valid throughout execution (no reallocation)
  - CallInfo valid and matches current frame
  - Chunk lifetime extends through execution
  - base + register < stack.len() (validated at call time)

  This leverages Rust's type system for LLVM optimization opportunities
----------------------------------------------------------------------*/

use crate::{
    CallInfo, Instruction, LUA_MASKCALL, LUA_MASKCOUNT, LUA_MASKLINE, LUA_MASKRET, LuaResult,
    LuaState, LuaValue, OpCode,
    lua_value::LUA_VNUMINT,
    lua_vm::{
        LuaError, TmKind,
        call_info::call_status::{CIST_C, CIST_CLSRET, CIST_PENDING_FINISH},
        execute::{
            call::{poscall, precall, pretailcall},
            closure::push_closure,
            concat::{concat, try_concat_pair_utf8},
            helper::{
                bin_tm_fallback, eq_fallback, error_div_by_zero, error_global, error_mod_by_zero,
                finishget_fallback, finishset_fallback, finishset_fallback_known_miss,
                float_for_loop, fltvalue, forprep, handle_pending_ops, ivalue, lua_fmod, lua_idiv,
                lua_imod, lua_shiftl, lua_shiftr, luai_numpow, objlen, order_tm_fallback,
                pfltvalue, pivalue, psetfltvalue, psetivalue, ptonumberns, pttisfloat,
                pttisinteger, return0_with_hook, return1_with_hook, setbfvalue, setbtvalue,
                setfltvalue, setivalue, setnilvalue, setobj2s, setobjs2s, tointeger, tointegerns,
                tonumberns, ttisfloat, ttisinteger, ttisstring, unary_tm_fallback,
            },
            hook::{hook_check_instruction, hook_on_call},
            number::{le_num, lt_num},
            vararg::{exec_varargprep, get_vararg, get_varargs},
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
        let mut ci = unsafe { &mut *ci_ptr };
        let call_status = ci.call_status;
        if call_status & (CIST_C | CIST_PENDING_FINISH) != 0 && handle_pending_ops(lua_state, ci)? {
            continue 'startfunc;
        }

        let mut base = ci.base;
        let pc_init = ci.pc as usize;
        let mut chunk = unsafe { &*ci.chunk_ptr };
        debug_assert!(lua_state.stack_len() >= base + chunk.max_stack_size + EXTRA_STACK);

        let mut code: &[Instruction] = &chunk.code;
        let mut constants: &[LuaValue] = &chunk.constants;
        let mut pc: usize = pc_init;

        if lua_state.hook_mask & LUA_MASKLINE != 0 {
            lua_state.oldpc = if pc_init > 0 {
                (pc_init - 1) as u32
            } else if chunk.is_vararg {
                0
            } else {
                u32::MAX
            };
        }

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
                    let a = instr.get_a();
                    let upval_value = *upval_value!(instr.get_b());
                    let key = k_val!(instr.get_c());
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    if upval_value.is_table() {
                        let table = upval_value.hvalue();
                        if !trap {
                            let next_instr = unsafe { *code.get_unchecked(pc) };
                            if next_instr.get_opcode() == OpCode::GetField
                                && next_instr.get_b() == a
                            {
                                let next_key = k_val!(next_instr.get_c());
                                debug_assert!(
                                    next_key.is_short_string(),
                                    "GetField key must be short string for fast path"
                                );

                                if let Some(outer) = table.impl_table.get_shortstr_fast(key) {
                                    if outer.is_table() {
                                        let inner_table = outer.hvalue();
                                        if inner_table.impl_table.has_hash() {
                                            let dest = unsafe {
                                                lua_state.stack_mut().as_mut_ptr().add(stack_id!(a))
                                            };
                                            if unsafe {
                                                inner_table
                                                    .impl_table
                                                    .get_shortstr_into(next_key, dest)
                                            } {
                                                pc += 1;
                                                continue;
                                            }
                                        }
                                    }

                                    setobj2s(lua_state, stack_id!(a), &outer);
                                    continue;
                                }
                            }
                        }

                        if table.impl_table.has_hash() {
                            let dest =
                                unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                            if unsafe { table.impl_table.get_shortstr_into(key, dest) } {
                                continue;
                            }
                        }
                    }
                    savestate!();
                    finishget_fallback(lua_state, ci, &upval_value, key, stack_id!(a))?;
                    updatetrap!();
                }
                OpCode::GetTable => {
                    // GETTABLE: R[A] := R[B][R[C]]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();

                    let rb = *unsafe { lua_state.stack().get_unchecked(stack_id!(b)) };

                    if rb.is_table() {
                        let table = rb.hvalue();
                        let rc_idx = stack_id!(c);
                        let rc_ptr = unsafe { lua_state.stack().as_ptr().add(rc_idx) };
                        let rc_tt = unsafe { (*rc_ptr).tt };
                        let dest = unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                        // Hot path 1: integer key → array fast path
                        if rc_tt == LUA_VNUMINT {
                            let key = unsafe { (*rc_ptr).value.i };
                            if unsafe { table.impl_table.fast_geti_into(key, dest) } {
                                continue;
                            }
                            if unsafe { table.impl_table.get_int_from_hash_into(key, dest) } {
                                continue;
                            }
                        }
                        // Hot path 2: short string key → hash fast path (zero-copy)
                        else if unsafe { (*rc_ptr).is_short_string() }
                            && table.impl_table.has_hash()
                            && unsafe { table.impl_table.get_shortstr_into(&*rc_ptr, dest) }
                        {
                            continue;
                        }
                        let rc = unsafe { *rc_ptr };
                        // Cold path: other key types, hash fallback for integers
                        if let Some(val) = table.impl_table.raw_get(&rc) {
                            setobj2s(lua_state, stack_id!(a), &val);
                            continue;
                        }

                        savestate!();
                        finishget_fallback(lua_state, ci, &rb, &rc, stack_id!(a))?;
                        updatetrap!();
                        continue;
                    }

                    let rc = *unsafe { lua_state.stack().get_unchecked(stack_id!(c)) };

                    // Metamethod / non-table fallback
                    savestate!();
                    finishget_fallback(lua_state, ci, &rb, &rc, stack_id!(a))?;
                    updatetrap!();
                }
                OpCode::GetI => {
                    // GETI: R[A] := R[B][C] (integer key)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let rc = instr.get_c() as i64;
                    let rb = *stack_val!(b);
                    if rb.is_table() {
                        let table = rb.hvalue();
                        let dest = unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                        // fast_geti: try array part first
                        let found = unsafe { table.impl_table.fast_geti_into(rc, dest) };
                        if found {
                            continue;
                        }
                        // fallback: direct integer hash lookup (no float/array re-check)
                        let found = unsafe { table.impl_table.get_int_from_hash_into(rc, dest) };
                        if found {
                            continue;
                        }
                    }

                    savestate!();
                    finishget_fallback(lua_state, ci, &rb, &LuaValue::integer(rc), stack_id!(a))?;
                    updatetrap!();
                }
                OpCode::GetField => {
                    // GETFIELD: R[A] := R[B][K[C]:string]
                    let rb = *stack_val!(instr.get_b());
                    let key = k_val!(instr.get_c());
                    debug_assert!(
                        key.is_short_string(),
                        "GetField key must be short string for fast path"
                    );
                    if rb.is_table() {
                        let table = rb.hvalue();
                        if table.impl_table.has_hash() {
                            let dest = unsafe {
                                lua_state
                                    .stack_mut()
                                    .as_mut_ptr()
                                    .add(stack_id!(instr.get_a()))
                            };
                            if unsafe { table.impl_table.get_shortstr_into(key, dest) } {
                                continue;
                            }
                        }
                    }
                    savestate!();
                    finishget_fallback(lua_state, ci, &rb, key, stack_id!(instr.get_a()))?;
                    updatetrap!();
                }
                OpCode::SetTabUp => {
                    // UpValue[A][K[B]:shortstring] := RK(C)
                    let upval_value = *upval_value!(instr.get_a());
                    let key = k_val!(instr.get_b());
                    let rc = if instr.get_k() {
                        *k_val!(instr.get_c())
                    } else {
                        *unsafe { lua_state.stack().get_unchecked(stack_id!(instr.get_c())) }
                    };
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    let mut known_newindex_miss = false;
                    if upval_value.is_table() {
                        let table = upval_value.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let pset_result = table.impl_table.pset_shortstr(key, rc);
                            let (new_key, delta) =
                                table.impl_table.finish_shortstr_set(key, rc, pset_result);
                            if new_key {
                                table.invalidate_tm_cache();
                            }
                            if delta != 0 {
                                lua_state.gc_track_table_resize(
                                    unsafe { upval_value.as_table_ptr_unchecked() },
                                    delta,
                                );
                            }
                            if rc.is_collectable() {
                                lua_state.gc_barrier_back(unsafe {
                                    upval_value.as_gc_ptr_table_unchecked()
                                });
                            }
                            continue;
                        } else {
                            if table.impl_table.set_existing_shortstr(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(unsafe {
                                        upval_value.as_gc_ptr_table_unchecked()
                                    });
                                }
                                continue;
                            }
                            known_newindex_miss = true;
                        }
                    }

                    savestate!();
                    if known_newindex_miss {
                        finishset_fallback_known_miss(lua_state, ci, &upval_value, key, rc)?;
                    } else {
                        finishset_fallback(lua_state, ci, &upval_value, key, rc)?;
                    }
                    updatetrap!();
                }
                OpCode::SetTable => {
                    // SETTABLE: R[A][R[B]] := RK(C)
                    let ra = *stack_val!(instr.get_a());
                    let rb = *stack_val!(instr.get_b());
                    let rc = if instr.get_k() {
                        *k_val!(instr.get_c())
                    } else {
                        *stack_val!(instr.get_c())
                    };

                    // Hot path: table + integer key in array range, no __newindex
                    let mut known_newindex_miss = false;
                    if ra.is_table() && rb.ttisinteger() {
                        let table = ra.hvalue_mut();
                        let key = rb.ivalue();
                        let table_ptr = unsafe { ra.as_table_ptr_unchecked() };
                        let gc_ptr = if rc.is_collectable() {
                            Some(unsafe { ra.as_gc_ptr_table_unchecked() })
                        } else {
                            None
                        };
                        if table.impl_table.set_existing_int(key, rc) {
                            if let Some(gc_ptr) = gc_ptr {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }

                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            if table.impl_table.fast_seti(key, rc) {
                                if let Some(gc_ptr) = gc_ptr {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }

                            let delta = table.impl_table.set_int_slow(key, rc);
                            if delta != 0 {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if let Some(gc_ptr) = gc_ptr {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        } else {
                            known_newindex_miss = true;
                        }
                    }

                    // Slow path: shortstr, generic key, non-table, or metamethod
                    if ra.is_table() && !known_newindex_miss {
                        let table_ptr = unsafe { ra.as_table_ptr_unchecked() };
                        let table = ra.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let mut hres = false;
                            if rb.is_short_string() {
                                let pset_result = table.impl_table.pset_shortstr(&rb, rc);
                                let (new_key, delta) =
                                    table.impl_table.finish_shortstr_set(&rb, rc, pset_result);
                                if new_key {
                                    table.invalidate_tm_cache();
                                }
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(table_ptr, delta);
                                }
                                hres = true;
                            } else if !rb.is_nil() && !rb.ttisinteger() {
                                let (_new_key, delta) = table.impl_table.raw_set(&rb, rc);
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(table_ptr, delta);
                                }
                                hres = true;
                            }

                            if hres {
                                if rc.is_collectable() || rb.is_collectable() {
                                    lua_state
                                        .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                                }
                                continue;
                            }
                        } else if rb.is_short_string() {
                            if table.impl_table.set_existing_shortstr(&rb, rc) {
                                if rc.is_collectable() || rb.is_collectable() {
                                    lua_state
                                        .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                                }
                                continue;
                            }
                            known_newindex_miss = true;
                        }
                    }
                    savestate!();
                    if known_newindex_miss {
                        finishset_fallback_known_miss(lua_state, ci, &ra, &rb, rc)?;
                    } else {
                        finishset_fallback(lua_state, ci, &ra, &rb, rc)?;
                    }
                    updatetrap!();
                }
                OpCode::SetI => {
                    // SETI: R[A][B] := RK(C) (integer key)
                    let ra = stack_val!(instr.get_a());
                    let b = instr.get_b() as i64;
                    let rc = if instr.get_k() {
                        *k_val!(instr.get_c())
                    } else {
                        *stack_val!(instr.get_c())
                    };
                    let mut known_newindex_miss = false;

                    // Hot path: table with no __newindex metamethod, key in array range
                    if ra.is_table() {
                        let table = ra.hvalue_mut();
                        let table_ptr = unsafe { ra.as_table_ptr_unchecked() };
                        let gc_ptr = if rc.is_collectable() {
                            Some(unsafe { ra.as_gc_ptr_table_unchecked() })
                        } else {
                            None
                        };
                        if table.impl_table.set_existing_int(b, rc) {
                            if let Some(gc_ptr) = gc_ptr {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }

                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            if table.impl_table.fast_seti(b, rc) {
                                if let Some(gc_ptr) = gc_ptr {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }

                            let delta = table.impl_table.set_int_slow(b, rc);
                            if delta != 0 {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if let Some(gc_ptr) = gc_ptr {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        } else {
                            known_newindex_miss = true;
                        }
                    }
                    let ra = *ra;
                    let rb = LuaValue::integer(b);
                    savestate!();
                    if known_newindex_miss {
                        finishset_fallback_known_miss(lua_state, ci, &ra, &rb, rc)?;
                    } else {
                        finishset_fallback(lua_state, ci, &ra, &rb, rc)?;
                    }
                    updatetrap!();
                }
                OpCode::SetField => {
                    // SETFIELD: R[A][K[B]:string] := RK(C)
                    let ra = *stack_val!(instr.get_a());
                    let key = k_val!(instr.get_b());
                    let rc = if instr.get_k() {
                        *k_val!(instr.get_c())
                    } else {
                        *stack_val!(instr.get_c())
                    };
                    debug_assert!(
                        key.is_short_string(),
                        "SetField key must be short string for fast path"
                    );
                    let mut known_newindex_miss = false;
                    if ra.is_table() {
                        let table = ra.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let pset_result = table.impl_table.pset_shortstr(key, rc);
                            let (new_key, delta) =
                                table.impl_table.finish_shortstr_set(key, rc, pset_result);
                            if new_key {
                                table.invalidate_tm_cache();
                            }
                            if delta != 0 {
                                lua_state.gc_track_table_resize(
                                    unsafe { ra.as_table_ptr_unchecked() },
                                    delta,
                                );
                            }
                            if rc.is_collectable() {
                                lua_state
                                    .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                            }
                            continue;
                        } else {
                            if table.impl_table.set_existing_shortstr(key, rc) {
                                if rc.is_collectable() {
                                    lua_state
                                        .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                                }
                                continue;
                            }
                            known_newindex_miss = true;
                        }
                    }
                    let rb = *key;
                    savestate!();
                    if known_newindex_miss {
                        finishset_fallback_known_miss(lua_state, ci, &ra, &rb, rc)?;
                    } else {
                        finishset_fallback(lua_state, ci, &ra, &rb, rc)?;
                    }
                    updatetrap!();
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
                    let rb = *stack_val!(instr.get_b());
                    let key = k_val!(instr.get_c());

                    debug_assert!(
                        key.is_short_string(),
                        "Self key must be short string for fast path"
                    );
                    setobj2s(lua_state, stack_id!(a + 1), &rb);
                    // Fast path: rb is a table with hash part
                    if rb.ttistable() {
                        let table = rb.hvalue();
                        if table.impl_table.has_hash() {
                            let dest =
                                unsafe { lua_state.stack_mut().as_mut_ptr().add(stack_id!(a)) };
                            if unsafe { table.impl_table.get_shortstr_into(key, dest) } {
                                continue;
                            }
                        }
                    }

                    savestate!();
                    finishget_fallback(lua_state, ci, &rb, key, stack_id!(a))?;
                    updatetrap!();
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
                                return Err(error_div_by_zero(lua_state));
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
                                return Err(error_mod_by_zero(lua_state));
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
                                return Err(error_mod_by_zero(lua_state));
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
                                return Err(error_div_by_zero(lua_state));
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
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let ra = *stack_val!(a);
                    let rb = *stack_val!(b);
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let result_reg = (base + pi.get_a() as usize) as u32;

                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };

                    savestate!();
                    bin_tm_fallback(lua_state, ci, ra, rb, result_reg, a as u32, b as u32, tm)?;
                    updatetrap!();
                }
                OpCode::MmBinI => {
                    let a = instr.get_a() as usize;
                    let imm = instr.get_sb();
                    let flip = instr.get_k();

                    let ra = stack_val!(a);
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let result_reg = (base + pi.get_a() as usize) as u32;

                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };
                    let rb = LuaValue::integer(imm as i64);
                    let r = if flip { (rb, *ra) } else { (*ra, rb) };
                    savestate!();
                    bin_tm_fallback(lua_state, ci, r.0, r.1, result_reg, a as u32, a as u32, tm)?;
                    updatetrap!();
                }
                OpCode::MmBinK => {
                    let ra = *stack_val!(instr.get_a());
                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                    let imm = *k_val!(instr.get_b());
                    let tm = unsafe { TmKind::from_u8_unchecked(instr.get_c() as u8) };
                    let flip = instr.get_k();
                    let result_reg = (base + pi.get_a() as usize) as u32;

                    let a_reg = instr.get_a();
                    savestate!();
                    let r = if flip { (imm, ra) } else { (ra, imm) };
                    bin_tm_fallback(lua_state, ci, r.0, r.1, result_reg, a_reg, a_reg, tm)?;
                    updatetrap!();
                }
                OpCode::Unm => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = *stack_val!(b);

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
                            unary_tm_fallback(lua_state, ci, rb, result_reg, TmKind::Unm)?;
                            updatetrap!();
                        }
                    }
                }
                OpCode::BNot => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = *stack_val!(b);

                    let mut ib = 0i64;
                    if tointegerns(&rb, &mut ib) {
                        setivalue(stack_val_mut!(a), !ib);
                    } else {
                        savestate!();
                        let result_reg = stack_id!(a);
                        unary_tm_fallback(lua_state, ci, rb, result_reg, TmKind::Bnot)?;
                        updatetrap!();
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
                    savestate!();
                    objlen(lua_state, stack_id!(a), rb)?;
                }
                OpCode::Concat => {
                    let a = instr.get_a();
                    let n = instr.get_b();

                    if n == 2 {
                        let left = *stack_val!(a);
                        let right = *stack_val!(a + 1);
                        ci.save_pc(pc);

                        if let Some(result) = try_concat_pair_utf8(lua_state, left, right)? {
                            *stack_val_mut!(a) = result;
                            updatetrap!();

                            let top = lua_state.get_top();
                            lua_state.check_gc_in_loop(ci, pc, top, &mut trap);
                            continue;
                        }
                    }

                    let concat_top = base + (a + n) as usize;
                    lua_state.set_top_raw(concat_top);

                    // ProtectNT
                    ci.save_pc(pc);
                    match concat(lua_state, n as usize) {
                        Ok(()) => {}
                        Err(LuaError::Yield) => {
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
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
                    ci.save_pc(pc); // save PC so get_local_var_name finds the variable name
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
                    let cond = eq_fallback(lua_state, ci, ra, rb)?;
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
                        } else if ra.is_number() && rb.is_number() {
                            lt_num(ra, rb)
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
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Lt)?;
                            updatetrap!();
                            result
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
                        } else if ra.is_number() && rb.is_number() {
                            le_num(ra, rb)
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
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Le)?;
                            updatetrap!();
                            result
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
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = unsafe {
                        let ra_ptr = lua_state.stack_mut().as_mut_ptr().add(base + a);

                        if pttisinteger(ra_ptr) {
                            pivalue(ra_ptr) < im as i64
                        } else if pttisfloat(ra_ptr) {
                            pfltvalue(ra_ptr) < im as f64
                        } else {
                            let va = *ra_ptr;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Lt)?;
                            updatetrap!();
                            result
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
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = unsafe {
                        let ra_ptr = lua_state.stack_mut().as_mut_ptr().add(base + a);

                        if pttisinteger(ra_ptr) {
                            pivalue(ra_ptr) <= im as i64
                        } else if pttisfloat(ra_ptr) {
                            pfltvalue(ra_ptr) <= im as f64
                        } else {
                            let va = *ra_ptr;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Le)?;
                            updatetrap!();
                            result
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
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = unsafe {
                        let ra_ptr = lua_state.stack_mut().as_mut_ptr().add(base + a);

                        if pttisinteger(ra_ptr) {
                            pivalue(ra_ptr) > im as i64
                        } else if pttisfloat(ra_ptr) {
                            pfltvalue(ra_ptr) > im as f64
                        } else {
                            let va = *ra_ptr;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            // GtI: a > b ≡ b < a → swap args, use Lt
                            let result = order_tm_fallback(lua_state, ci, vb, va, TmKind::Lt)?;
                            updatetrap!();
                            result
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
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = unsafe {
                        let ra_ptr = lua_state.stack_mut().as_mut_ptr().add(base + a);

                        if pttisinteger(ra_ptr) {
                            pivalue(ra_ptr) >= im as i64
                        } else if pttisfloat(ra_ptr) {
                            pfltvalue(ra_ptr) >= im as f64
                        } else {
                            let va = *ra_ptr;
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            // GeI: a >= b ≡ b <= a → swap args, use Le
                            let result = order_tm_fallback(lua_state, ci, vb, va, TmKind::Le)?;
                            updatetrap!();
                            result
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
                    // l_isfalse: nil or false => truthy = !nil && !false
                    let cond = !ra.is_nil() && !ra.ttisfalse();

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
                OpCode::Call => {
                    let a = instr.get_a();
                    let b = instr.get_b() as usize;
                    let nresults = instr.get_c() as i32 - 1;
                    if b != 0 {
                        lua_state.set_top_raw(stack_id!(a) + b);
                    }
                    ci.save_pc(pc);
                    if precall(lua_state, stack_id!(a), nresults)? {
                        // Lua call: new frame pushed
                        continue 'startfunc;
                    }
                    // C call completed
                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::TailCall => {
                    let a = instr.get_a();
                    let mut b = instr.get_b() as usize;
                    let func_idx = stack_id!(a);
                    // let nparams1 = instr.get_c() as usize;
                    if b != 0 {
                        lua_state.set_top_raw(func_idx + b);
                    } else {
                        b = lua_state.get_top() - func_idx;
                    }
                    ci.save_pc(pc);
                    if instr.get_k() {
                        lua_state.close_upvalues(base);
                    }
                    if pretailcall(lua_state, ci, func_idx, b)? {
                        // Lua tail call: CI reused in place
                        continue 'startfunc;
                    }
                    // C tail call completed
                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::Return => {
                    // return R[A], ..., R[A+B-2]   (lvm.c:1763-1783)
                    let a_pos = stack_id!(instr.get_a());
                    let mut n;

                    // Check if resuming after a yield inside __close during return
                    if ci.call_status & CIST_CLSRET != 0 {
                        // Resuming from yield-in-close: use saved nres and skip close_all
                        // (close_all already ran; remaining TBCs were closed on resume)
                        ci.call_status &= !CIST_CLSRET;
                        n = ci.saved_nres();

                        // Save pc first so re-yield points to RETURN again
                        ci.save_pc(pc);

                        // Continue closing remaining TBC variables (if any)
                        match lua_state.close_all(base) {
                            Ok(()) => {}
                            Err(LuaError::Yield) => {
                                ci.call_status |= CIST_CLSRET;
                                ci.pc -= 1;
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        n = instr.get_b() as i32 - 1;
                        if n < 0 {
                            n = (lua_state.get_top() - a_pos) as i32;
                        }

                        ci.save_pc(pc);
                        if instr.get_k() {
                            // May have open upvalues / TBC variables
                            ci.set_saved_nres(n);
                            if lua_state.get_top() < ci.top as usize {
                                lua_state.set_top_raw(ci.top as usize);
                            }
                            match lua_state.close_all(base) {
                                Ok(()) => {}
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_CLSRET;
                                    ci.pc -= 1;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    }

                    lua_state.set_top_raw(a_pos + n as usize);
                    poscall(lua_state, ci, n as usize, pc)?;
                    // updatetrap!();
                    continue 'startfunc; // goto ret
                }
                OpCode::Return0 => {
                    // return (no values)
                    if lua_state.hook_mask & (LUA_MASKRET | LUA_MASKLINE) != 0 {
                        return0_with_hook(lua_state, ci, stack_id!(instr.get_a()), pc)?;
                        continue 'startfunc;
                    }

                    // Inlined fast path: no hook, no moveresults overhead
                    let nresults = ci.nresults();
                    let res = ci.base - ci.func_offset as usize;
                    lua_state.pop_call_frame();
                    lua_state.set_top_raw(res);
                    // nil-fill if caller wanted results
                    if nresults > 0 {
                        unsafe {
                            let sp = lua_state.stack_mut().as_mut_ptr();
                            for i in 0..nresults as usize {
                                *sp.add(res + i) = LuaValue::nil();
                            }
                        }
                        lua_state.set_top_raw(res + nresults as usize);
                    }

                    // "goto returning" — skip full startfunc reload
                    let new_depth = lua_state.call_depth();
                    if new_depth <= target_depth {
                        return Ok(());
                    }
                    let new_fi = new_depth - 1;
                    let ci_ptr = unsafe { lua_state.get_call_info_ptr(new_fi) } as *mut CallInfo;
                    ci = unsafe { &mut *ci_ptr };
                    // If caller has pending metamethod finish (yield in __index etc.),
                    // must go through startfunc to run handle_pending_ops.
                    if ci.call_status & CIST_PENDING_FINISH != 0 {
                        continue 'startfunc;
                    }
                    base = ci.base;
                    pc = ci.pc as usize;
                    chunk = unsafe { &*ci.chunk_ptr };
                    code = &chunk.code;
                    constants = &chunk.constants;
                    trap = lua_state.hook_mask != 0;
                    continue; // inner dispatch loop
                }
                OpCode::Return1 => {
                    // return R[A]  (single value)
                    if lua_state.hook_mask & (LUA_MASKRET | LUA_MASKLINE) != 0 {
                        return1_with_hook(lua_state, ci, stack_id!(instr.get_a()), pc)?;
                        continue 'startfunc;
                    }

                    // Inlined fast path — raw pointer for single copy
                    let nresults = ci.nresults();
                    let res = ci.base - ci.func_offset as usize;
                    lua_state.pop_call_frame();
                    if nresults == 0 {
                        // Caller wants no results
                        lua_state.set_top_raw(res);
                    } else {
                        // Copy the single result value using raw pointer
                        unsafe {
                            let sp = lua_state.stack_mut().as_mut_ptr();
                            let val = *sp.add(base + instr.get_a() as usize);
                            *sp.add(res) = val;
                        }
                        lua_state.set_top_raw(res + 1);
                        // nil-fill if caller wanted more than 1
                        if nresults > 1 {
                            unsafe {
                                let sp = lua_state.stack_mut().as_mut_ptr();
                                for i in 1..nresults as usize {
                                    *sp.add(res + i) = LuaValue::nil();
                                }
                            }
                            lua_state.set_top_raw(res + nresults as usize);
                        }
                    }

                    // "goto returning" — C Lua optimization: avoid full startfunc reload.
                    // Instead of `continue 'startfunc` (which re-checks depth, CIST_C,
                    // oldpc, call hook), directly reload caller's frame state and
                    // continue the dispatch loop.
                    let new_depth = lua_state.call_depth();
                    if new_depth <= target_depth {
                        return Ok(());
                    }
                    let new_fi = new_depth - 1;
                    let ci_ptr = unsafe { lua_state.get_call_info_ptr(new_fi) } as *mut CallInfo;
                    ci = unsafe { &mut *ci_ptr };
                    // If caller has pending metamethod finish (yield in __index etc.),
                    // must go through startfunc to run handle_pending_ops.
                    if ci.call_status & CIST_PENDING_FINISH != 0 {
                        continue 'startfunc;
                    }
                    base = ci.base;
                    pc = ci.pc as usize;
                    chunk = unsafe { &*ci.chunk_ptr };
                    code = &chunk.code;
                    constants = &chunk.constants;
                    trap = lua_state.hook_mask != 0;
                    continue; // inner dispatch loop — skip startfunc overhead
                }
                OpCode::ForLoop => {
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    unsafe {
                        let ra = lua_state.stack_mut().as_mut_ptr().add(base + a);
                        // Check if integer loop (tag of step at ra+1)
                        if pttisinteger(ra.add(1)) {
                            // Integer loop (most common for numeric loops)
                            // ra: counter (count of iterations left)
                            // ra+1: step
                            // ra+2: control variable (idx)
                            let count = pivalue(ra) as u64;
                            if count > 0 {
                                // More iterations
                                let step = pivalue(ra.add(1));
                                let idx = pivalue(ra.add(2));

                                // Update counter (decrement) - only write value, tag unchanged
                                (*ra).value.i = count as i64 - 1;
                                // Update control variable: idx += step - only write value
                                (*ra.add(2)).value.i = idx.wrapping_add(step);

                                // Jump back
                                pc -= bx;
                            }
                            // else: counter expired, exit loop
                        } else if float_for_loop(lua_state, base + a) {
                            // Float loop with non-integer step
                            // Jump back if loop continues
                            pc -= bx;
                        }
                    }

                    updatetrap!();
                }
                OpCode::ForPrep => {
                    let a = instr.get_a();
                    savestate!();
                    if forprep(lua_state, stack_id!(a))? {
                        // Skip the loop body: jump forward past FORLOOP
                        pc += instr.get_bx() as usize + 1;
                    }
                }
                OpCode::TForPrep => {
                    // Prepare generic for loop — inline (for loop related)
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = base + a;

                    // Swap control and closing variables
                    stack.swap(ra + 3, ra + 2);

                    // Mark ra+2 as to-be-closed if not nil
                    lua_state.mark_tbc(ra + 2)?;

                    pc += bx;
                }
                OpCode::TForCall => {
                    // Generic for loop call — matches C Lua's OP_TFORCALL.
                    // Copy iterator+state+control to ra+3..ra+5, then precall.
                    let a = instr.get_a() as usize;
                    let c = instr.get_c() as usize;
                    let ra = base + a;
                    let func_idx = ra + 3;
                    unsafe {
                        let stack = lua_state.stack_mut();
                        *stack.get_unchecked_mut(ra + 5) = *stack.get_unchecked(ra + 3);
                        *stack.get_unchecked_mut(ra + 4) = *stack.get_unchecked(ra + 1);
                        *stack.get_unchecked_mut(ra + 3) = *stack.get_unchecked(ra);
                    }
                    lua_state.set_top_raw(func_idx + 3); // func + 2 args
                    ci.save_pc(pc);
                    if precall(lua_state, func_idx, c as i32)? {
                        // Lua iterator: new frame pushed
                        continue 'startfunc;
                    }
                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::TForLoop => {
                    // Generic for loop test
                    // If ra+3 (control variable) != nil then continue loop (jump back)
                    // After TForPrep swap: ra+2=closing(TBC), ra+3=control
                    // TFORCALL places first result at ra+3, automatically updating control
                    // Check if ra+3 (control value from iterator) is not nil
                    if !stack_val!(instr.get_a() + 3).is_nil() {
                        // Continue loop: jump back
                        pc -= instr.get_bx() as usize;
                    }
                    // else: exit loop (control variable is nil)
                }
                OpCode::SetList => {
                    let a = instr.get_a();
                    let mut n = instr.get_vb() as usize;
                    let stack_idx = instr.get_vc() as usize;
                    let mut last = stack_idx;
                    if n == 0 {
                        n = lua_state.get_top() - stack_id!(a) - 1; // adjust n based on top if vb=0
                    } else {
                        lua_state.set_top_raw(ci.top as usize);
                    }
                    last += n;
                    if instr.get_k() {
                        let next_instr = unsafe { *code.get_unchecked(pc) };
                        debug_assert!(next_instr.get_opcode() == OpCode::ExtraArg);
                        pc += 1; // Consume EXTRAARG
                        let extra = next_instr.get_ax() as usize;
                        // Add extra to starting index
                        last += extra * (1 << Instruction::SIZE_V_C);
                    }
                    let ra = *stack_val!(a);
                    let h = ra.hvalue_mut();
                    if last > h.impl_table.asize as usize {
                        h.impl_table.resize_array(last as u32);
                    }

                    let impl_table = &mut h.impl_table;
                    let stack_ptr = lua_state.stack().as_ptr();
                    let mut is_collectable = false;
                    // Port of C Lua's SETLIST loop (lvm.c):
                    //   for (; n > 0; n--) { val = s2v(ra+n); obj2arr(h, last, val); last--; }
                    // Reads n values from stack[ra+n..ra+1], writes to table[last..last-n+1]
                    let mut write_idx = last;
                    for i in (1..=n).rev() {
                        let val = unsafe { *stack_ptr.add(stack_id!(a) + i) };
                        if val.iscollectable() {
                            is_collectable = true;
                        }
                        unsafe {
                            impl_table.write_array(write_idx as i64, val);
                        }
                        write_idx -= 1;
                    }

                    if is_collectable {
                        lua_state.gc_barrier_back(unsafe { ra.as_gc_ptr_unchecked() });
                    }
                }
                OpCode::Closure => {
                    let a = instr.get_a() as usize;
                    let proto_idx = instr.get_bx() as usize;
                    savestate!();
                    let upvalue_ptrs = unsafe {
                        let lf: *const _ = ci.func.as_lua_function_unchecked();
                        (&*lf).upvalues()
                    };
                    push_closure(lua_state, base, a, proto_idx, chunk, upvalue_ptrs)?;

                    lua_state.check_gc_in_loop(ci, pc, base + a + 1, &mut trap);
                }
                OpCode::Vararg => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let n = instr.get_c() as i32 - 1;
                    let vatab = if instr.get_k() { b as i32 } else { -1 };

                    savestate!();
                    match get_varargs(lua_state, ci, base, a, b, vatab, n, chunk) {
                        Ok(()) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::GetVarg => {
                    let a = stack_id!(instr.get_a());
                    let c = stack_id!(instr.get_c());
                    get_vararg(lua_state, ci, base, a, c)?;
                }
                OpCode::ErrNNil => {
                    let a = instr.get_a();
                    let ra = stack_val!(a);

                    if !ra.is_nil() {
                        let bx = instr.get_bx() as usize;
                        let global_name = if bx > 0 && bx - 1 < constants.len() {
                            if let Some(s) = constants[bx - 1].as_str() {
                                s.to_string()
                            } else {
                                "?".to_string()
                            }
                        } else {
                            "?".to_string()
                        };

                        savestate!();
                        return Err(error_global(lua_state, &global_name));
                    }
                }
                OpCode::VarargPrep => {
                    exec_varargprep(lua_state, ci, chunk, &mut base)?;
                    // After varargprep, hook call if hooks are active
                    let hook_mask = lua_state.hook_mask;
                    if hook_mask != 0 {
                        let call_status = ci.call_status;
                        hook_on_call(lua_state, hook_mask, call_status, chunk)?;
                        if hook_mask & LUA_MASKLINE != 0 {
                            lua_state.oldpc = u32::MAX; // force line event on next instruction
                        }
                    }
                }
                OpCode::ExtraArg => {
                    // Extra argument for previous opcode
                    // This instruction should never be executed directly
                    // It's always consumed by the previous instruction (NEWTABLE, SETLIST, etc.)
                    // If we reach here, it's a compiler error
                    debug_assert!(false, "ExtraArg should never be executed directly");
                }
            }
        }
    }
}
