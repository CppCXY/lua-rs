// LuaTable - Rust优化的Lua Table实现
pub mod native_table;

use super::lua_value::LuaValue;
use crate::{LuaResult, TablePtr, lua_vm::LuaError};
use native_table::NativeTable;

/// Mask covering all TM flags — any bit set to 1 represents a cacheable TM.
/// With u32, we cover all 26 TmKind values (bits 0-25).
const MASK_FLAGS: u32 = (1u32 << 26) - 1;

pub struct LuaTable {
    meta: TablePtr,
    /// Bit-flag cache for absent metamethods (fasttm).
    /// Bit i set ⇒ TmKind(i) is known absent in this table (when used as a metatable).
    /// Covers all 26 TmKind values.
    flags: u32,
    pub(crate) impl_table: NativeTable,
}

impl LuaTable {
    /// 创建新table
    pub fn new(asize: u32, hsize: u32) -> Self {
        Self {
            meta: TablePtr::null(),
            flags: 0,
            impl_table: NativeTable::new(asize, hsize),
        }
    }

    /// Invalidate cached TM flags (called when new keys are inserted).
    /// Matches Lua 5.5's `invalidateTMcache(t)`: `t->flags &= ~maskflags`
    #[inline(always)]
    pub(crate) fn invalidate_tm_cache(&mut self) {
        self.flags &= !MASK_FLAGS;
    }

    /// fasttm: Check if TmKind `tm` is known absent from this table.
    /// Returns true if the metamethod is definitely NOT present (skip hash lookup).
    #[inline(always)]
    pub(crate) fn no_tm(&self, tm: u8) -> bool {
        self.flags & (1u32 << tm) != 0
    }

    /// Cache that TmKind `tm` is absent from this table.
    #[inline(always)]
    pub(crate) fn set_tm_absent(&mut self, tm: u8) {
        self.flags |= 1u32 << tm;
    }

    /// Get the raw metatable pointer (avoids LuaValue allocation for fasttm checks)
    #[inline(always)]
    pub(crate) fn meta_ptr(&self) -> TablePtr {
        self.meta
    }

    #[inline(always)]
    pub fn has_metatable(&self) -> bool {
        !self.meta.is_null()
    }

    pub fn get_metatable(&self) -> Option<LuaValue> {
        if self.meta.is_null() {
            None
        } else {
            Some(LuaValue::table(self.meta))
        }
    }

    pub(crate) fn set_metatable(&mut self, metatable: Option<LuaValue>) {
        if let Some(meta) = metatable {
            if let Some(table_ptr) = meta.as_table_ptr() {
                self.meta = table_ptr;
            } else {
                self.meta = TablePtr::null();
            }
        } else {
            self.meta = TablePtr::null();
        }
    }

    pub fn len(&self) -> usize {
        self.impl_table.len()
    }

    pub fn hash_size(&self) -> usize {
        self.impl_table.hash_size()
    }

    /// Current data memory footprint (array + hash allocations, not including GcTable header).
    /// Used by GC to track resize deltas.
    #[inline]
    pub fn compute_mem_size(&self) -> usize {
        self.impl_table.compute_mem_size()
    }

    pub fn is_array(&self) -> bool {
        // NativeTable always has both array and hash parts
        self.impl_table.hash_size() == 0
    }

    #[inline(always)]
    pub fn raw_geti(&self, key: i64) -> Option<LuaValue> {
        self.impl_table.get_int(key)
    }

    #[inline(always)]
    pub(crate) fn raw_seti(&mut self, key: i64, value: LuaValue) -> isize {
        self.impl_table.set_int(key, value)
    }

    #[inline(always)]
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        self.impl_table.raw_get(key)
    }

    /// return (new_key_inserted, mem_delta)
    #[inline(always)]
    pub(crate) fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> (bool, isize) {
        let (new_key, delta) = self.impl_table.raw_set(key, value);
        if new_key {
            // New key inserted — invalidate TM cache (this table might be a metatable)
            // Matches Lua 5.5: invalidateTMcache(t) in luaH_finishset
            self.invalidate_tm_cache();
        }
        (new_key, delta)
    }

    /// Returns Ok(Some((key, value))) for next entry, Ok(None) for end of table,
    /// or Err(()) for invalid key.
    pub fn next(&self, input_key: &LuaValue) -> Result<Option<(LuaValue, LuaValue)>, ()> {
        self.impl_table.next(input_key)
    }

    pub fn remove_array_at(&mut self, i: i64) -> LuaResult<LuaValue> {
        self.impl_table
            .remove_at(i)
            .ok_or(LuaError::IndexOutOfBounds)
    }

    pub fn iter_all(&self) -> Vec<(LuaValue, LuaValue)> {
        let mut result = Vec::new();
        self.impl_table.for_each_entry(|k, v| {
            result.push((k, v));
        });
        result
    }

    pub fn iter_keys(&self) -> Vec<LuaValue> {
        let mut result = Vec::new();
        self.impl_table.for_each_entry(|k, _v| {
            result.push(k);
        });
        result
    }

    /// GC-safe iteration: call f for each entry without allocating Vec
    /// This is used by GC to traverse table entries safely
    pub(crate) fn for_each_entry<F>(&self, f: F)
    where
        F: FnMut(LuaValue, LuaValue),
    {
        self.impl_table.for_each_entry(f);
    }
}
