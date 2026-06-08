/*----------------------------------------------------------------------
  Lua 5.5 VM Table Operations

  Extracted from execute_loop.rs to separate table GET/SET/CREATE
  operations from the main dispatch loop.

  Design:
  1. Each opcode is an #[inline(always)] function that takes &mut
     references to shared VM state (base_stk, pc, trap).
  2. Functions return LuaResult<()> — the caller always does `continue`
     after a successful invocation.
  3. The savestate/syncbase/updatetrap macros are replaced with direct
     code since the functions have &mut access to all state.
  4. This mirrors Lua 5.5's C code organization (lvm.c delegates table
     access to ltable.c functions), while maintaining zero-overhead
     inlining via #[inline(always)].
----------------------------------------------------------------------*/

use crate::{
    Instruction, LuaResult, LuaValue, OpCode,
    gc::TablePtr,
    lua_vm::{
        LuaState, StkId, TmKind,
        call_info::CallInfo,
        execute::{
            helper::{
                finishget_fallback, finishset_fallback, instr_at, k_val,
                self_shortstr_index_chain_fast,
            },
            metamethod::call_newindex_tm_fast,
        },
    },
};

// ── GET operations ──────────────────────────────────────────────

/// GetTabUp: R[A] := UpValue[B][K[C]:shortstring]
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(crate) fn op_get_tabup(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    code: &[Instruction],
    constants: &[LuaValue],
) -> LuaResult<()> {
    let a = instr.get_a();
    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(instr.get_b() as usize) };
    let upval_value = upvalue_ptr.as_ref().data.get_value_ref();
    let key = k_val(constants, instr.get_c());
    let dest = (*base_stk).offset(a as usize);
    debug_assert!(
        key.is_short_string(),
        "GetTabUp key must be short string for fast path"
    );

    if upval_value.is_table() {
        let table = upval_value.hvalue();
        if !*trap {
            let next_instr = instr_at(code, *pc);
            if next_instr.get_opcode() == OpCode::GetField && next_instr.get_b() == a {
                let next_key = k_val(constants, next_instr.get_c());
                debug_assert!(
                    next_key.is_short_string(),
                    "GetField key must be short string for fast path"
                );

                if let Some(outer) = table.impl_table.get_shortstr_fast(key) {
                    if outer.is_table() {
                        let inner_table = outer.hvalue();
                        if inner_table.impl_table.has_hash()
                            && inner_table
                                .impl_table
                                .get_shortstr_into(next_key, dest.as_ptr())
                        {
                            *pc += 1;
                            return Ok(());
                        }
                    }

                    (*base_stk).offset(a as usize).write(&outer);
                    return Ok(());
                }
            }
        }

        if table.impl_table.has_hash() && table.impl_table.get_shortstr_into(key, dest.as_ptr()) {
            return Ok(());
        }
    }

    ci.save_pc(*pc);
    lua_state.set_top_raw(ci.top as usize);
    let upval_value = *upval_value;
    finishget_fallback(lua_state, ci, &upval_value, key, dest)?;
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

/// GetTable: R[A] := R[B][R[C]]
#[inline(always)]
pub(crate) fn op_get_table(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    _pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    _constants: &[LuaValue],
) -> LuaResult<()> {
    let a = instr.get_a();
    let b = instr.get_b();
    let c = instr.get_c();

    let base = *base_stk;
    let rb = base.offset(b as usize);
    let rc = base.offset(c as usize);
    let dest = base.offset(a as usize);
    if rb.is_table() {
        let table = rb.hvalue();

        // Hot path 1: integer key → array fast path
        if rc.is_integer() {
            let key = rc.ivalue();
            if table.impl_table.fast_geti_into(key, dest.as_ptr()) {
                return Ok(());
            }
            if table.impl_table.get_int_from_hash_into(key, dest.as_ptr()) {
                return Ok(());
            }
        }
        // Hot path 2: short string key → hash fast path (zero-copy)
        else if rc.is_short_string()
            && table.impl_table.has_hash()
            && table
                .impl_table
                .get_shortstr_into(rc.get_ref(), dest.as_ptr())
        {
            return Ok(());
        }
        // Cold path: other key types, hash fallback for integers
        if let Some(val) = table.impl_table.raw_get(rc.get_ref()) {
            dest.write(&val);
            return Ok(());
        }

        ci.save_pc(*_pc);
        lua_state.set_top_raw(ci.top as usize);
        finishget_fallback(lua_state, ci, rb.get_ref(), rc.get_ref(), dest)?;
        *base_stk = ci.base_stk;
        #[cfg(not(feature = "sandbox"))]
        {
            *trap = lua_state.hook_mask != 0;
        }
        #[cfg(feature = "sandbox")]
        {
            *trap = lua_state.has_active_instruction_watch();
        }
        return Ok(());
    }

    // Metamethod / non-table fallback
    ci.save_pc(*_pc);
    lua_state.set_top_raw(ci.top as usize);
    finishget_fallback(lua_state, ci, rb.get_ref(), rc.get_ref(), dest)?;
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

/// GetI: R[A] := R[B][C] (integer key)
#[inline(always)]
pub(crate) fn op_get_i(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    _pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    _constants: &[LuaValue],
) -> LuaResult<()> {
    let a = instr.get_a();
    let b = instr.get_b();
    let rc = instr.get_c() as i64;
    let base = *base_stk;
    let rb = base.offset(b as usize);
    let dest = base.offset(a as usize);
    if rb.is_table() {
        let table = rb.hvalue();

        // fast_geti: try array part first
        let found = table.impl_table.fast_geti_into(rc, dest.as_ptr());
        if found {
            return Ok(());
        }
        // fallback: direct integer hash lookup (no float/array re-check)
        let found = table.impl_table.get_int_from_hash_into(rc, dest.as_ptr());
        if found {
            return Ok(());
        }
    }

    ci.save_pc(*_pc);
    lua_state.set_top_raw(ci.top as usize);
    finishget_fallback(lua_state, ci, rb.get_ref(), &LuaValue::integer(rc), dest)?;
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

/// GetField: R[A] := R[B][K[C]:string]
#[inline(always)]
pub(crate) fn op_get_field(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    _pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    constants: &[LuaValue],
) -> LuaResult<()> {
    let base = *base_stk;
    let rb = base.offset(instr.get_b() as usize);
    let key = k_val(constants, instr.get_c());
    debug_assert!(
        key.is_short_string(),
        "GetField key must be short string for fast path"
    );
    if rb.is_table() {
        let table = rb.hvalue();
        if table.impl_table.has_hash() {
            let dest = base.offset(instr.get_a() as usize);
            if table.impl_table.get_shortstr_into(key, dest.as_ptr()) {
                return Ok(());
            }
        }
    }
    ci.save_pc(*_pc);
    lua_state.set_top_raw(ci.top as usize);
    let rb = rb.get();
    finishget_fallback(lua_state, ci, &rb, key, base.offset(instr.get_a() as usize))?;
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

// ── SET operations ──────────────────────────────────────────────

/// SetTabUp: UpValue[A][K[B]:shortstring] := RK(C)
#[inline(always)]
pub(crate) fn op_set_tabup(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    constants: &[LuaValue],
) -> LuaResult<()> {
    let a = instr.get_a();
    let b = instr.get_b();
    let c = instr.get_c();
    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(a as usize) };
    let upval_value = upvalue_ptr.as_ref().data.get_value_ref();
    let key = k_val(constants, b);
    debug_assert!(
        key.is_short_string(),
        "GetTabUp key must be short string for fast path"
    );
    let mut known_newindex_miss = false;
    let mut meta = TablePtr::null();
    if upval_value.is_table() {
        let table = upval_value.hvalue_mut();
        let table_ptr = upval_value.table_ptr_raw();
        let gc_ptr = upval_value.as_gc_ptr_unchecked();
        meta = table.meta_ptr();
        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
            let (new_key, delta, is_collectable) = if instr.get_k() {
                let rc = *k_val(constants, c);
                let pset_result = table.impl_table.pset_shortstr(key, rc);
                let (new_key, delta) = table.impl_table.finish_shortstr_set(key, rc, pset_result);
                (new_key, delta, rc.is_collectable())
            } else {
                let rc = (*base_stk).offset(c as usize);
                let pset_result = table.impl_table.pset_shortstr(key, rc.get());
                let (new_key, delta) =
                    table
                        .impl_table
                        .finish_shortstr_set(key, rc.get(), pset_result);
                (new_key, delta, rc.is_collectable())
            };
            if new_key {
                table.invalidate_tm_cache();
            }
            if delta != 0 {
                lua_state.gc_track_table_resize(table_ptr, delta);
            }
            if is_collectable {
                lua_state.gc_barrier_back(gc_ptr);
            }
            return Ok(());
        } else {
            let rc = if instr.get_k() {
                *k_val(constants, c)
            } else {
                (*base_stk).offset(c as usize).get()
            };
            if table.impl_table.set_existing_shortstr(key, rc) {
                if rc.is_collectable() {
                    lua_state.gc_barrier_back(gc_ptr);
                }
                return Ok(());
            }
            known_newindex_miss = true;
        }
    }

    let upval_value = *upval_value;
    let rc = if instr.get_k() {
        *k_val(constants, c)
    } else {
        (*base_stk).offset(c as usize).get()
    };
    ci.save_pc(*pc);
    lua_state.set_top_raw(ci.top as usize);
    if known_newindex_miss {
        if call_newindex_tm_fast(lua_state, ci, upval_value, meta, *key, rc)? {
            *base_stk = ci.base_stk;
            #[cfg(not(feature = "sandbox"))]
            {
                *trap = lua_state.hook_mask != 0;
            }
            #[cfg(feature = "sandbox")]
            {
                *trap = lua_state.has_active_instruction_watch();
            }
            return Ok(());
        }
        finishset_fallback(lua_state, ci, &upval_value, key, rc, true)?;
    } else {
        finishset_fallback(lua_state, ci, &upval_value, key, rc, false)?;
    }
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

/// SetTable: R[A][R[B]] := RK(C)
#[inline(always)]
pub(crate) fn op_set_table(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    constants: &[LuaValue],
) -> LuaResult<()> {
    let a = instr.get_a();
    let b = instr.get_b();
    let c = instr.get_c();

    let base = *base_stk;
    let ra = base.offset(a as usize);
    let rb = base.offset(b as usize);

    // Hot path: table + integer key in array range, no __newindex
    if ra.is_table() && rb.is_integer() {
        let table = ra.hvalue_mut();
        let key = rb.ivalue();
        let meta = table.meta_ptr();
        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
            if !instr.get_k() {
                let rc = base.offset(c as usize);
                if table.impl_table.fast_seti(key, rc.get()) {
                    if rc.is_collectable() {
                        lua_state.gc_barrier_back(ra.as_gc_ptr());
                    }
                    return Ok(());
                }

                let rc = rc.get();
                let delta = table.impl_table.set_int_slow(key, rc);
                if delta != 0 {
                    lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
                }
                if rc.is_collectable() {
                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                }
                return Ok(());
            }

            let rc = *k_val(constants, c);
            if table.impl_table.fast_seti(key, rc) {
                if rc.is_collectable() {
                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                }
                return Ok(());
            }

            let delta = table.impl_table.set_int_slow(key, rc);
            if delta != 0 {
                lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
            }
            if rc.is_collectable() {
                lua_state.gc_barrier_back(ra.as_gc_ptr());
            }
            return Ok(());
        } else {
            let rc = if instr.get_k() {
                *k_val(constants, c)
            } else {
                base.offset(c as usize).get()
            };
            if table.impl_table.set_existing_int(key, rc) {
                if rc.is_collectable() {
                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                }
                return Ok(());
            }
            // Fall through to finishset fallback (known miss)
            let rc = if instr.get_k() {
                *k_val(constants, c)
            } else {
                base.offset(c as usize).get()
            };
            ci.save_pc(*pc);
            lua_state.set_top_raw(ci.top as usize);
            if call_newindex_tm_fast(lua_state, ci, ra.get(), meta, rb.get(), rc)? {
                *base_stk = ci.base_stk;
                #[cfg(not(feature = "sandbox"))]
                {
                    *trap = lua_state.hook_mask != 0;
                }
                #[cfg(feature = "sandbox")]
                {
                    *trap = lua_state.has_active_instruction_watch();
                }
                return Ok(());
            }
            finishset_fallback(lua_state, ci, ra.get_ref(), rb.get_ref(), rc, true)?;
            *base_stk = ci.base_stk;
            #[cfg(not(feature = "sandbox"))]
            {
                *trap = lua_state.hook_mask != 0;
            }
            #[cfg(feature = "sandbox")]
            {
                *trap = lua_state.has_active_instruction_watch();
            }
            return Ok(());
        }
    }

    // Slow path: shortstr, generic key, non-table, or metamethod
    if ra.is_table() {
        let table = ra.hvalue_mut();
        let meta = table.meta_ptr();
        if (meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()))
            && rb.is_short_string()
        {
            let key = rb.get_ref();
            let (new_key, delta, needs_barrier) = if instr.get_k() {
                let rc = *k_val(constants, c);
                let pset_result = table.impl_table.pset_shortstr(key, rc);
                let (new_key, delta) = table.impl_table.finish_shortstr_set(key, rc, pset_result);
                (new_key, delta, rc.is_collectable() || key.is_collectable())
            } else {
                let rc = base.offset(c as usize);
                let pset_result = table.impl_table.pset_shortstr(key, rc.get());
                let (new_key, delta) =
                    table
                        .impl_table
                        .finish_shortstr_set(key, rc.get(), pset_result);
                (new_key, delta, (rc.is_collectable() || rb.is_collectable()))
            };
            if new_key {
                table.invalidate_tm_cache();
            }
            if delta != 0 {
                lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
            }
            if needs_barrier {
                lua_state.gc_barrier_back(ra.as_gc_ptr());
            }
            return Ok(());
        }
    }

    let rc = if instr.get_k() {
        *k_val(constants, c)
    } else {
        base.offset(c as usize).get()
    };
    if ra.is_table() {
        let table = ra.hvalue_mut();
        let meta = table.meta_ptr();
        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
            if !rb.is_nil() && !rb.is_integer() {
                let (_new_key, delta) = table.impl_table.raw_set(rb.get_ref(), rc);
                if delta != 0 {
                    lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
                }
                if rc.is_collectable() || rb.is_collectable() {
                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                }
                return Ok(());
            }
        } else if rb.is_short_string() {
            if table.impl_table.set_existing_shortstr(rb.get_ref(), rc) {
                if rc.is_collectable() || rb.is_collectable() {
                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                }
                return Ok(());
            }
            ci.save_pc(*pc);
            lua_state.set_top_raw(ci.top as usize);
            if call_newindex_tm_fast(lua_state, ci, ra.get(), meta, rb.get(), rc)? {
                *base_stk = ci.base_stk;
                #[cfg(not(feature = "sandbox"))]
                {
                    *trap = lua_state.hook_mask != 0;
                }
                #[cfg(feature = "sandbox")]
                {
                    *trap = lua_state.has_active_instruction_watch();
                }
                return Ok(());
            }
            finishset_fallback(lua_state, ci, ra.get_ref(), rb.get_ref(), rc, true)?;
            *base_stk = ci.base_stk;
            #[cfg(not(feature = "sandbox"))]
            {
                *trap = lua_state.hook_mask != 0;
            }
            #[cfg(feature = "sandbox")]
            {
                *trap = lua_state.has_active_instruction_watch();
            }
            return Ok(());
        }
    }
    ci.save_pc(*pc);
    lua_state.set_top_raw(ci.top as usize);
    finishset_fallback(lua_state, ci, ra.get_ref(), rb.get_ref(), rc, false)?;
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

/// SetI: R[A][B] := RK(C) (integer key)
#[inline(always)]
pub(crate) fn op_set_i(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    constants: &[LuaValue],
) -> LuaResult<()> {
    let base = *base_stk;
    let ra = base.offset(instr.get_a() as usize);
    let b = instr.get_b() as i64;
    let c = instr.get_c();

    // Hot path: table with no __newindex metamethod, key in array range
    if ra.is_table() {
        let table = ra.hvalue_mut();
        let table_ptr = ra.as_table_ptr();
        let gc_ptr = ra.as_gc_ptr();
        let meta = table.meta_ptr();
        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
            if !instr.get_k() {
                let rc = base.offset(c as usize);
                if table.impl_table.fast_seti(b, rc.get()) {
                    if rc.is_collectable() {
                        lua_state.gc_barrier_back(gc_ptr);
                    }
                    return Ok(());
                }

                let delta = table.impl_table.set_int_slow(b, rc.get());
                if delta != 0 {
                    lua_state.gc_track_table_resize(table_ptr, delta);
                }
                if rc.is_collectable() {
                    lua_state.gc_barrier_back(gc_ptr);
                }
                return Ok(());
            }

            let rc = *k_val(constants, c);
            if table.impl_table.fast_seti(b, rc) {
                if rc.is_collectable() {
                    lua_state.gc_barrier_back(gc_ptr);
                }
                return Ok(());
            }

            let delta = table.impl_table.set_int_slow(b, rc);
            if delta != 0 {
                lua_state.gc_track_table_resize(table_ptr, delta);
            }
            if rc.is_collectable() {
                lua_state.gc_barrier_back(gc_ptr);
            }
            return Ok(());
        } else {
            let rc = if instr.get_k() {
                *k_val(constants, c)
            } else {
                base.offset(c as usize).get()
            };
            if table.impl_table.set_existing_int(b, rc) {
                if rc.is_collectable() {
                    lua_state.gc_barrier_back(gc_ptr);
                }
                return Ok(());
            }
            // Fall through to finishset fallback (known miss)
            let rb = LuaValue::integer(b);
            ci.save_pc(*pc);
            lua_state.set_top_raw(ci.top as usize);
            if call_newindex_tm_fast(lua_state, ci, ra.get(), meta, rb, rc)? {
                *base_stk = ci.base_stk;
                #[cfg(not(feature = "sandbox"))]
                {
                    *trap = lua_state.hook_mask != 0;
                }
                #[cfg(feature = "sandbox")]
                {
                    *trap = lua_state.has_active_instruction_watch();
                }
                return Ok(());
            }
            finishset_fallback(lua_state, ci, ra.get_ref(), &rb, rc, true)?;
            *base_stk = ci.base_stk;
            #[cfg(not(feature = "sandbox"))]
            {
                *trap = lua_state.hook_mask != 0;
            }
            #[cfg(feature = "sandbox")]
            {
                *trap = lua_state.has_active_instruction_watch();
            }
            return Ok(());
        }
    }
    let rc = if instr.get_k() {
        *k_val(constants, c)
    } else {
        base.offset(c as usize).get()
    };
    let rb = LuaValue::integer(b);
    ci.save_pc(*pc);
    lua_state.set_top_raw(ci.top as usize);
    finishset_fallback(lua_state, ci, ra.get_ref(), &rb, rc, false)?;
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

/// SetField: R[A][K[B]:string] := RK(C)
#[inline(always)]
pub(crate) fn op_set_field(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    constants: &[LuaValue],
) -> LuaResult<()> {
    let a = instr.get_a();
    let b = instr.get_b();
    let c = instr.get_c();
    let ra = (*base_stk).offset(a as usize);
    let key = k_val(constants, b);
    debug_assert!(
        key.is_short_string(),
        "SetField key must be short string for fast path"
    );
    let mut known_newindex_miss = false;
    let mut meta = TablePtr::null();
    if ra.is_table() {
        let table = ra.hvalue_mut();
        meta = table.meta_ptr();
        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
            let (new_key, delta, is_collectable) = if instr.get_k() {
                let rc = *k_val(constants, c);
                let pset_result = table.impl_table.pset_shortstr(key, rc);
                let (new_key, delta) = table.impl_table.finish_shortstr_set(key, rc, pset_result);
                (new_key, delta, rc.is_collectable())
            } else {
                let rc = (*base_stk).offset(c as usize);
                let pset_result = table.impl_table.pset_shortstr(key, rc.get());
                let (new_key, delta) =
                    table
                        .impl_table
                        .finish_shortstr_set(key, rc.get(), pset_result);
                (new_key, delta, rc.is_collectable())
            };
            if new_key {
                table.invalidate_tm_cache();
            }
            if delta != 0 {
                lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
            }
            if is_collectable {
                lua_state.gc_barrier_back(ra.as_gc_ptr());
            }
            return Ok(());
        } else {
            let rc = if instr.get_k() {
                *k_val(constants, c)
            } else {
                (*base_stk).offset(c as usize).get()
            };
            if table.impl_table.set_existing_shortstr(key, rc) {
                if rc.is_collectable() {
                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                }
                return Ok(());
            }
            known_newindex_miss = true;
        }
    }
    let rc = if instr.get_k() {
        *k_val(constants, c)
    } else {
        (*base_stk).offset(c as usize).get()
    };
    ci.save_pc(*pc);
    lua_state.set_top_raw(ci.top as usize);
    if known_newindex_miss {
        if call_newindex_tm_fast(lua_state, ci, ra.get(), meta, *key, rc)? {
            *base_stk = ci.base_stk;
            #[cfg(not(feature = "sandbox"))]
            {
                *trap = lua_state.hook_mask != 0;
            }
            #[cfg(feature = "sandbox")]
            {
                *trap = lua_state.has_active_instruction_watch();
            }
            return Ok(());
        }
        finishset_fallback(lua_state, ci, ra.get_ref(), key, rc, true)?;
    } else {
        finishset_fallback(lua_state, ci, ra.get_ref(), key, rc, false)?;
    }
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

// ── Table creation / bulk operations ────────────────────────────

/// NewTable: R[A] := {} (create new table)
#[inline(always)]
pub(crate) fn op_new_table(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    code: &[Instruction],
) -> LuaResult<()> {
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
        let extra_instr = instr_at(code, *pc);
        if extra_instr.get_opcode() == OpCode::ExtraArg {
            vc += extra_instr.get_ax() * 1024;
        }
    }

    *pc += 1; // skip EXTRAARG

    let value = lua_state.create_table(vc as usize, vb as usize)?;
    (*base_stk).offset(a as usize).write(&value);

    let new_top = ci.base + a as usize + 1;
    lua_state.check_gc_in_loop(*pc, new_top, trap);
    *base_stk = ci.base_stk;
    Ok(())
}

/// Self_: R[A+1] := R[B]; R[A] := R[B][K[C]:string]
#[inline(always)]
pub(crate) fn op_self(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    _pc: &mut usize,
    trap: &mut bool,
    instr: Instruction,
    constants: &[LuaValue],
) -> LuaResult<()> {
    let a = instr.get_a();
    let base = *base_stk;
    let rb = base.offset(instr.get_b() as usize).get();
    let key = k_val(constants, instr.get_c());

    debug_assert!(
        key.is_short_string(),
        "Self key must be short string for fast path"
    );
    base.offset(a as usize + 1).write(&rb);
    // Fast path: rb is a table with hash part
    if rb.ttistable() {
        let table = rb.hvalue();
        if table.impl_table.has_hash() {
            let dest = base.offset(a as usize);
            if table.impl_table.get_shortstr_into(key, dest.as_ptr()) {
                return Ok(());
            }
        }
        if self_shortstr_index_chain_fast(lua_state, &rb, key, base.offset(a as usize)) {
            *base_stk = ci.base_stk;
            #[cfg(not(feature = "sandbox"))]
            {
                *trap = lua_state.hook_mask != 0;
            }
            #[cfg(feature = "sandbox")]
            {
                *trap = lua_state.has_active_instruction_watch();
            }
            return Ok(());
        }
    }

    ci.save_pc(*_pc);
    lua_state.set_top_raw(ci.top as usize);
    finishget_fallback(lua_state, ci, &rb, key, base.offset(a as usize))?;
    *base_stk = ci.base_stk;
    #[cfg(not(feature = "sandbox"))]
    {
        *trap = lua_state.hook_mask != 0;
    }
    #[cfg(feature = "sandbox")]
    {
        *trap = lua_state.has_active_instruction_watch();
    }
    Ok(())
}

/// SetList: R[A][(C-1)*FPF+i] := R(A+i), 1 <= i <= B
#[inline(always)]
pub(crate) fn op_set_list(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    _trap: &mut bool,
    instr: Instruction,
    code: &[Instruction],
) -> LuaResult<()> {
    let a = instr.get_a();
    let mut n = instr.get_vb() as usize;
    let stack_idx = instr.get_vc() as usize;
    let mut last = stack_idx;
    let a_pos = ci.base + a as usize;
    if n == 0 {
        n = lua_state.get_top() - a_pos - 1; // adjust n based on top if vb=0
    } else {
        lua_state.set_top_raw(ci.top as usize);
    }
    last += n;
    if instr.get_k() {
        let next_instr = instr_at(code, *pc);
        debug_assert!(next_instr.get_opcode() == OpCode::ExtraArg);
        *pc += 1; // Consume EXTRAARG
        let extra = next_instr.get_ax() as usize;
        // Add extra to starting index
        last += extra * (1 << Instruction::SIZE_V_C);
    }
    let ra = (*base_stk).offset(a as usize).get();
    let h = ra.hvalue_mut();
    if last > h.impl_table.asize as usize {
        h.impl_table.resize_array(last as u32);
    }

    let impl_table = &mut h.impl_table;
    let stack_base = lua_state.stack().as_ptr();
    let mut is_collectable = false;
    // Port of C Lua's SETLIST loop (lvm.c):
    //   for (; n > 0; n--) { val = s2v(ra+n); obj2arr(h, last, val); last--; }
    // Reads n values from stack[ra+n..ra+1], writes to table[last..last-n+1]
    let mut write_idx = last;
    for i in (1..=n).rev() {
        let val = unsafe { *stack_base.add(a_pos + i) };
        if val.iscollectable() {
            is_collectable = true;
        }

        impl_table.write_array(write_idx as i64, val);
        write_idx -= 1;
    }

    if is_collectable {
        lua_state.gc_barrier_back(ra.as_gc_ptr_unchecked());
    }
    Ok(())
}
