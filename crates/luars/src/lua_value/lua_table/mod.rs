// LuaTable - Rust优化的Lua Table实现
pub mod native_table;

use super::lua_value::LuaValue;
use crate::{LuaResult, TablePtr, lua_vm::LuaError};
use native_table::NativeTable;

pub struct LuaTable {
    meta: TablePtr,
    pub(crate) impl_table: NativeTable,
}

impl LuaTable {
    /// 创建新table
    pub fn new(asize: u32, hsize: u32) -> Self {
        Self {
            meta: TablePtr::null(),
            impl_table: NativeTable::new(asize, hsize),
        }
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

    pub fn is_array(&self) -> bool {
        // NativeTable always has both array and hash parts
        self.impl_table.hash_size() == 0
    }

    pub fn raw_geti(&self, key: i64) -> Option<LuaValue> {
        self.impl_table.get_int(key)
    }

    pub(crate) fn raw_seti(&mut self, key: i64, value: LuaValue) {
        self.impl_table.set_int(key, value);
    }

    #[inline(always)]
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        self.impl_table.raw_get(key)
    }

    /// return true if new key inserted, false if updated existing key
    #[inline(always)]
    pub(crate) fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> bool {
        self.impl_table.raw_set(key, value)
    }

    pub fn next(&self, input_key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        self.impl_table.next(input_key)
    }

    /// Insert value at array index i (1-based)
    /// return true if new key inserted, false if updated existing key
    pub(crate) fn insert_array_at(&mut self, i: i64, value: LuaValue) -> LuaResult<bool> {
        Ok(self.impl_table.insert_at(i, value))
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
