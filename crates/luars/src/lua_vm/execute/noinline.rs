// ======================================================================
// Non-inlined metamethod fast-path helpers.
//
// These functions are HOT but intentionally #[inline(never)] to reduce
// register pressure and code size in the main dispatch loop.
// They are NOT #[cold] — the compiler still optimises them normally.
//
// Return an enum so the caller can decide which label to continue to.
// ======================================================================

use crate::{
    GcTable, LuaResult, TablePtr,
    lua_value::{LUA_VTABLE, LuaValue},
    lua_vm::{LuaState, TmKind, execute::cold},
};

// ── result types ─────────────────────────────────────────────────────

/// Outcome of a metatable __index lookup.
pub enum IndexResult {
    /// Value found (or nil for absent metamethod) — store in R[A] and `continue`.
    Found(LuaValue),
    /// Lua metamethod frame pushed — `continue 'startfunc`.
    CallMm,
    /// __index exists but is not a Lua function or is a table that missed —
    /// fall through to the cold recursive path.
    FallThrough,
}

/// Outcome of a binary-metamethod check on v1's metatable.
pub enum MmBinResult {
    /// Lua metamethod frame pushed — `continue 'startfunc`.
    CallMm,
    /// C metamethod called and result stored — `restore_state!(); continue`.
    Handled,
    /// No metamethod on v1 — fall through to slow path.
    FallThrough,
}

/// Outcome of a table `__len` metatable check.
pub enum LenResult {
    /// No __len present — use raw `table.len()`.
    RawLen,
    /// Lua metamethod frame pushed — `continue 'startfunc`.
    CallMm,
    /// __len is a C function — fall through to `handle_len`.
    FallThrough,
}

// ── __index metatable helpers ────────────────────────────────────────

/// Check metatable __index for a **short-string** key (GetField / Self_ / GetTabUp).
///
/// `meta` must be non-null (caller checks `meta_ptr().is_null()`).
#[inline(never)]
pub fn try_index_meta_str(
    lua_state: &mut LuaState,
    meta: TablePtr,
    obj: LuaValue,
    key: &LuaValue,
    frame_idx: usize,
) -> LuaResult<IndexResult> {
    let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
    const TM_INDEX_BIT: u8 = TmKind::Index as u8;
    if mt.no_tm(TM_INDEX_BIT) {
        return Ok(IndexResult::Found(LuaValue::nil()));
    }
    let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
    if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
        if mm.is_lua_function() {
            cold::push_lua_mm_frame(lua_state, mm, obj, *key, frame_idx)?;
            return Ok(IndexResult::CallMm);
        }
        if mm.tt == LUA_VTABLE {
            // Chain through __index tables (prototype chain optimization).
            // Instead of checking only one level and falling through to the
            // expensive cold path, iterate up to MAXTAGLOOP levels inline.
            let mut current_mm = mm;
            for _ in 0..crate::lua_vm::lua_limits::MAXTAGLOOP {
                let idx_table = unsafe { &*(current_mm.value.ptr as *const GcTable) };
                if let Some(val) = idx_table.data.impl_table.get_shortstr_fast(key) {
                    return Ok(IndexResult::Found(val));
                }
                // Not found — check if this __index table also has a metatable
                let next_meta = idx_table.data.meta_ptr();
                if next_meta.is_null() {
                    return Ok(IndexResult::Found(LuaValue::nil()));
                }
                let next_mt = unsafe { &mut (*next_meta.as_mut_ptr()).data };
                if next_mt.no_tm(TM_INDEX_BIT) {
                    return Ok(IndexResult::Found(LuaValue::nil()));
                }
                let next_event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
                match next_mt.impl_table.get_shortstr_fast(&next_event_key) {
                    Some(next_mm) => {
                        if next_mm.is_lua_function() {
                            // __index is a Lua function — push call frame
                            let idx_val = LuaValue {
                                value: current_mm.value,
                                tt: current_mm.tt,
                            };
                            cold::push_lua_mm_frame(lua_state, next_mm, idx_val, *key, frame_idx)?;
                            return Ok(IndexResult::CallMm);
                        }
                        if next_mm.tt == LUA_VTABLE {
                            current_mm = next_mm;
                            continue;
                        }
                        // __index is some other type — fall through to cold path
                        return Ok(IndexResult::FallThrough);
                    }
                    None => {
                        next_mt.set_tm_absent(TM_INDEX_BIT);
                        return Ok(IndexResult::Found(LuaValue::nil()));
                    }
                }
            }
        }
        Ok(IndexResult::FallThrough)
    } else {
        mt.set_tm_absent(TM_INDEX_BIT);
        Ok(IndexResult::Found(LuaValue::nil()))
    }
}

/// Check metatable __index for an **integer** key (GetI).
///
/// `meta` must be non-null.
#[inline(never)]
pub fn try_index_meta_int(
    lua_state: &mut LuaState,
    meta: TablePtr,
    obj: LuaValue,
    int_key: i64,
    frame_idx: usize,
) -> LuaResult<IndexResult> {
    let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
    const TM_INDEX_BIT: u8 = TmKind::Index as u8;
    if mt.no_tm(TM_INDEX_BIT) {
        return Ok(IndexResult::Found(LuaValue::nil()));
    }
    let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
    if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
        if mm.is_lua_function() {
            cold::push_lua_mm_frame(lua_state, mm, obj, LuaValue::integer(int_key), frame_idx)?;
            return Ok(IndexResult::CallMm);
        }
        if mm.tt == LUA_VTABLE {
            let idx_table = unsafe { &*(mm.value.ptr as *const GcTable) };
            if let Some(val) = idx_table.data.impl_table.fast_geti(int_key) {
                return Ok(IndexResult::Found(val));
            }
        }
        Ok(IndexResult::FallThrough)
    } else {
        mt.set_tm_absent(TM_INDEX_BIT);
        Ok(IndexResult::Found(LuaValue::nil()))
    }
}

/// Check metatable __index for a **generic** key (GetTable).
///
/// `meta` must be non-null.
#[inline(never)]
pub fn try_index_meta_generic(
    lua_state: &mut LuaState,
    meta: TablePtr,
    obj: LuaValue,
    key: LuaValue,
    frame_idx: usize,
) -> LuaResult<IndexResult> {
    let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
    const TM_INDEX_BIT: u8 = TmKind::Index as u8;
    if mt.no_tm(TM_INDEX_BIT) {
        return Ok(IndexResult::Found(LuaValue::nil()));
    }
    let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
    if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
        if mm.is_lua_function() {
            cold::push_lua_mm_frame(lua_state, mm, obj, key, frame_idx)?;
            return Ok(IndexResult::CallMm);
        }
        if mm.tt == LUA_VTABLE {
            let idx_table = unsafe { &*(mm.value.ptr as *const GcTable) };
            let idx_result = if key.ttisinteger() {
                idx_table.data.impl_table.fast_geti(key.ivalue())
            } else {
                idx_table.data.impl_table.raw_get(&key)
            };
            if let Some(val) = idx_result {
                return Ok(IndexResult::Found(val));
            }
        }
        Ok(IndexResult::FallThrough)
    } else {
        mt.set_tm_absent(TM_INDEX_BIT);
        Ok(IndexResult::Found(LuaValue::nil()))
    }
}

// ── __newindex metatable helper ──────────────────────────────────────

/// Check metatable __newindex — Lua function fast path.
///
/// `meta` must be non-null.
/// Returns `Ok(true)` if a Lua __newindex frame was pushed (`continue 'startfunc`).
#[inline(never)]
pub fn try_newindex_meta(
    lua_state: &mut LuaState,
    meta: TablePtr,
    obj: LuaValue,
    key: LuaValue,
    val: LuaValue,
    frame_idx: usize,
) -> LuaResult<bool> {
    let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
    const TM_NEWINDEX_BIT: u8 = TmKind::NewIndex as u8;
    if !mt.no_tm(TM_NEWINDEX_BIT) {
        let event_key = lua_state
            .vm_mut()
            .const_strings
            .get_tm_value(TmKind::NewIndex);
        if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key)
            && mm.is_lua_function()
        {
            cold::push_lua_newindex_frame(lua_state, mm, obj, key, val, frame_idx)?;
            return Ok(true);
        }
    }
    Ok(false)
}

// ── comparison metatable helper ──────────────────────────────────────

/// Check a table value's metatable for a comparison metamethod (__eq / __lt / __le).
///
/// `table_val.tt` must be `LUA_VTABLE` (caller checks).
/// `v1` and `v2` are the operands for `push_lua_mm_frame` (may differ from
/// `table_val` when GTI/GEI swap the operand order).
///
/// Returns `Ok(true)` if a Lua metamethod frame was pushed (`continue 'startfunc`).
#[inline(never)]
pub fn try_comp_meta_table(
    lua_state: &mut LuaState,
    table_val: LuaValue,
    v1: LuaValue,
    v2: LuaValue,
    tm: TmKind,
    frame_idx: usize,
) -> LuaResult<bool> {
    let table_gc = unsafe { &*(table_val.value.ptr as *const GcTable) };
    let meta = table_gc.data.meta_ptr();
    if meta.is_null() {
        return Ok(false);
    }
    let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
    let tm_bit = tm as u8;
    if mt.no_tm(tm_bit) {
        return Ok(false);
    }
    let event_key = lua_state.vm_mut().const_strings.get_tm_value(tm);
    if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
        if mm.is_lua_function() {
            cold::push_lua_mm_frame(lua_state, mm, v1, v2, frame_idx)?;
            return Ok(true);
        }
        Ok(false)
    } else {
        mt.set_tm_absent(tm_bit);
        Ok(false)
    }
}

// ── MmBin / MmBinI / MmBinK  metatable helper ───────────────────────

/// Fast path for MmBin* opcodes: check v1's metatable for the given metamethod.
///
/// `table_val.tt` must be `LUA_VTABLE` (caller checks `v1.ttistable()`).
/// `p1` and `p2` are the ordered operands for `push_lua_mm_frame` / `call_c_mm_bin`
/// (may differ from `table_val` when MmBinI/MmBinK swap the operands).
///
/// - Lua function → pushes frame, returns `CallMm`.
/// - C function   → calls it immediately, stores result, returns `Handled`.
/// - Not found    → returns `FallThrough`.
#[inline(never)]
pub fn try_mmbin_table_fast(
    lua_state: &mut LuaState,
    table_val: LuaValue,
    p1: LuaValue,
    p2: LuaValue,
    tm_idx: u8,
    result_reg: usize,
    frame_idx: usize,
) -> LuaResult<MmBinResult> {
    let table = unsafe { &mut *(table_val.value.ptr as *mut GcTable) };
    let meta = table.data.meta_ptr();
    if meta.is_null() {
        return Ok(MmBinResult::FallThrough);
    }
    let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
    if mt.no_tm(tm_idx) {
        return Ok(MmBinResult::FallThrough);
    }
    let tm_kind = unsafe { TmKind::from_u8_unchecked(tm_idx) };
    let event_key = lua_state.vm_mut().const_strings.get_tm_value(tm_kind);
    if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
        if mm.is_lua_function() {
            cold::push_lua_mm_frame(lua_state, mm, p1, p2, frame_idx)?;
            return Ok(MmBinResult::CallMm);
        }
        // C function metamethod
        cold::call_c_mm_bin(lua_state, mm, p1, p2, result_reg, frame_idx)?;
        Ok(MmBinResult::Handled)
    } else {
        mt.set_tm_absent(tm_idx);
        Ok(MmBinResult::FallThrough)
    }
}

// ── Len metatable helper ─────────────────────────────────────────────

/// Check a table's metatable for `__len`.
///
/// `meta` must be non-null.
///
/// - `RawLen`      — __len absent, use raw `table.len()`.
/// - `CallMm`      — Lua __len frame pushed.
/// - `FallThrough`  — __len is a C function, fall to `handle_len`.
#[inline(never)]
pub fn try_len_meta(
    lua_state: &mut LuaState,
    meta: TablePtr,
    rb: LuaValue,
    frame_idx: usize,
) -> LuaResult<LenResult> {
    let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
    const TM_LEN_BIT: u8 = TmKind::Len as u8;
    if mt.no_tm(TM_LEN_BIT) {
        return Ok(LenResult::RawLen);
    }
    let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Len);
    if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
        if mm.is_lua_function() {
            cold::push_lua_mm_frame(lua_state, mm, rb, rb, frame_idx)?;
            return Ok(LenResult::CallMm);
        }
        Ok(LenResult::FallThrough)
    } else {
        mt.set_tm_absent(TM_LEN_BIT);
        Ok(LenResult::RawLen)
    }
}
