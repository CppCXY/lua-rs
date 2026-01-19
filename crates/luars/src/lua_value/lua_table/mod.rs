// LuaTable - Rust优化的Lua Table实现
mod hash_table;
mod type_array;
mod value_array;

use super::lua_value::LuaValue;
use crate::{
    GcId, GcObjectKind, LuaResult, TablePtr,
    lua_value::lua_table::{hash_table::LuaHashTable, value_array::LuaValueArray},
    lua_vm::LuaError,
};

pub struct LuaTable {
    meta: TablePtr,

    pub(crate) impl_table: LuaTableDetail,
}

impl LuaTable {
    /// 创建新table
    pub fn new(asize: u32, hsize: u32) -> Self {
        let impl_table = if hsize == 0 {
            LuaTableDetail::ValueArray(LuaValueArray::new(asize as usize))
        } else {
            LuaTableDetail::HashTable(LuaHashTable::new(hsize as usize))
        };

        Self {
            meta: TablePtr::null(),
            impl_table,
        }
    }

    #[inline(always)]
    pub fn has_metatable(&self) -> bool {
        !self.meta.is_null()
    }
    pub fn get_metatable(&self) -> Option<TablePtr> {
        if self.meta.is_null() {
            None
        } else {
            Some(self.meta)
        }
    }

    pub fn set_metatable(&mut self, metatable: Option<LuaValue>) {
        
    }

    pub fn len(&self) -> usize {
        match &self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.len(),
            LuaTableDetail::ValueArray(arr) => arr.len(),
            LuaTableDetail::HashTable(map) => map.len(),
        }
    }

    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        match &self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.get_int(key),
            LuaTableDetail::ValueArray(arr) => arr.get_int(key),
            LuaTableDetail::HashTable(map) => map.get_int(key),
        }
    }

    #[allow(unused)]
    fn migrate_to_value_array(&mut self) {
        let len = self.len();
        let old_impl = std::mem::replace(
            &mut self.impl_table,
            LuaTableDetail::ValueArray(LuaValueArray::new(len)),
        );

        // if let LuaTableDetail::ValueArray(new_arr) = &mut self.impl_table {
        //     match old_impl {
        //         LuaTableDetail::TypedArray(old_arr) => {
        //             let tt = old_arr.tt;
        //             new_arr.array.resize(
        //                 len,
        //                 LuaValue {
        //                     tt,
        //                     value: Value::nil(),
        //                 },
        //             );

        //             for i in 0..len {
        //                 if let Some(v) = old_arr.get_int((i + 1) as i64) {
        //                     new_arr.array[i] = v;
        //                 }
        //             }
        //         }
        //         _ => {}
        //     }
        // }
    }

    fn migrate_to_hash_table(&mut self) {
        let len = self.len();
        // 预留 2x 容量，避免频繁扩容
        let capacity = (len * 2).max(32);
        let old_impl = std::mem::replace(
            &mut self.impl_table,
            LuaTableDetail::HashTable(LuaHashTable::new(capacity)),
        );

        if let LuaTableDetail::HashTable(new_map) = &mut self.impl_table {
            match old_impl {
                // LuaTableDetail::TypedArray(old_arr) => {
                //     for i in 0..len {
                //         if let Some(v) = old_arr.get_int((i + 1) as i64) {
                //             new_map.set_int((i + 1) as i64, v);
                //         }
                //     }
                // }
                LuaTableDetail::ValueArray(old_arr) => {
                    for i in 0..len {
                        if let Some(v) = old_arr.get_int((i + 1) as i64) {
                            new_map.set_int((i + 1) as i64, v);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        let r = match &mut self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.set_int(key, value),
            LuaTableDetail::ValueArray(arr) => arr.set_int(key, value),
            LuaTableDetail::HashTable(map) => map.set_int(key, value),
        };

        match r {
            LuaInsertResult::Success | LuaInsertResult::Failure => {}
            LuaInsertResult::NeedConvertToValueArray => {
                self.migrate_to_value_array();
                if let LuaTableDetail::ValueArray(arr) = &mut self.impl_table {
                    arr.set_int(key, value);
                }
            }
            LuaInsertResult::NeedConvertToHashTable => {
                self.migrate_to_hash_table();
                if let LuaTableDetail::HashTable(map) = &mut self.impl_table {
                    map.set_int(key, value);
                }
            }
        }
    }

    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        match &self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.raw_get(key),
            LuaTableDetail::ValueArray(arr) => arr.raw_get(key),
            LuaTableDetail::HashTable(map) => map.raw_get(key),
        }
    }

    pub fn raw_set(&mut self, key: &LuaValue, value: LuaValue) {
        let r = match &mut self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.raw_set(key, value),
            LuaTableDetail::ValueArray(arr) => arr.raw_set(key, value),
            LuaTableDetail::HashTable(map) => map.raw_set(key, value),
        };

        match r {
            LuaInsertResult::Success | LuaInsertResult::Failure => {}
            LuaInsertResult::NeedConvertToValueArray => {
                self.migrate_to_value_array();
                if let LuaTableDetail::ValueArray(arr) = &mut self.impl_table {
                    arr.raw_set(key, value);
                }
            }
            LuaInsertResult::NeedConvertToHashTable => {
                self.migrate_to_hash_table();
                if let LuaTableDetail::HashTable(map) = &mut self.impl_table {
                    map.raw_set(key, value);
                }
            }
        }
    }

    pub fn next(&self, input_key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        match &self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.next(input_key),
            LuaTableDetail::ValueArray(arr) => arr.next(input_key),
            LuaTableDetail::HashTable(map) => map.next(input_key),
        }
    }

    pub fn insert_array_at(&mut self, i: i64, value: LuaValue) -> LuaResult<()> {
        let index = (i - 1) as usize;
        let r = match &mut self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.insert_at(index, value),
            LuaTableDetail::ValueArray(arr) => arr.insert_at(index, value),
            LuaTableDetail::HashTable(map) => map.insert_at(index, value),
        };

        match r {
            LuaInsertResult::Success => {}
            LuaInsertResult::Failure => return Err(LuaError::IndexOutOfBounds),
            LuaInsertResult::NeedConvertToValueArray => {
                self.migrate_to_value_array();
                if let LuaTableDetail::ValueArray(arr) = &mut self.impl_table {
                    arr.insert_at(index, value);
                }
            }
            LuaInsertResult::NeedConvertToHashTable => {
                self.migrate_to_hash_table();
                if let LuaTableDetail::HashTable(map) = &mut self.impl_table {
                    map.insert_at(index, value);
                }
            }
        }
        Ok(())
    }

    pub fn remove_array_at(&mut self, i: i64) -> LuaResult<LuaValue> {
        let index = (i - 1) as usize;
        match &mut self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.remove_at(index),
            LuaTableDetail::ValueArray(arr) => arr.remove_at(index),
            LuaTableDetail::HashTable(map) => map.remove_at(index),
        }
    }

    pub fn iter_all(&self) -> Vec<(LuaValue, LuaValue)> {
        let mut result = Vec::new();
        match &self.impl_table {
            // LuaTableDetail::TypedArray(ar) => {
            //     let tt = ar.tt;
            //     for i in 0..ar.array.len() {
            //         let value = LuaValue {
            //             value: ar.array[i],
            //             tt,
            //         };
            //         let key = LuaValue::integer((i + 1) as i64);
            //         result.push((key, value));
            //     }
            // }
            LuaTableDetail::ValueArray(ar) => {
                for i in 0..ar.array.len() {
                    let value = ar.array[i];
                    let key = LuaValue::integer((i + 1) as i64);
                    result.push((key, value));
                }
            }
            LuaTableDetail::HashTable(t) => {
                // 使用 next 方法遵循接口遍历
                let mut key = LuaValue::nil();
                while let Some((k, v)) = t.next(&key) {
                    result.push((k, v));
                    key = k;
                }
            }
        }

        result
    }
}

pub trait LuaTableImpl {
    fn get_int(&self, key: i64) -> Option<LuaValue>;

    fn set_int(&mut self, key: i64, value: LuaValue) -> LuaInsertResult;

    fn raw_get(&self, key: &LuaValue) -> Option<LuaValue>;

    fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> LuaInsertResult;

    fn insert_at(&mut self, index: usize, value: LuaValue) -> LuaInsertResult;

    fn remove_at(&mut self, index: usize) -> LuaResult<LuaValue>;

    fn next(&self, input_key: &LuaValue) -> Option<(LuaValue, LuaValue)>;

    fn len(&self) -> usize;
}

impl LuaTable {
    /// Remove entries with dead (collectible) keys or values
    /// Used by weak table cleanup during GC
    /// - weak_keys: if true, remove entries whose keys are dead GC objects
    /// - weak_values: if true, remove entries whose values are dead GC objects
    /// - is_dead: closure to check if a GcId is dead
    pub fn remove_weak_entries_with_checker<F>(
        &mut self,
        weak_keys: bool,
        weak_values: bool,
        mut is_dead: F,
    ) where
        F: FnMut(crate::gc::GcId) -> bool,
    {
        // Collect all keys to remove
        let mut keys_to_remove = Vec::new();

        // Iterate over all entries
        let entries = self.iter_all();
        for (key, value) in entries {
            let mut should_remove = false;

            // Check if key should cause removal (for weak keys)
            if weak_keys {
                if let Some(gc_id) = Self::value_to_gc_id(&key) {
                    if is_dead(gc_id) {
                        should_remove = true;
                    }
                }
            }

            // Check if value should cause removal (for weak values)
            if !should_remove && weak_values {
                if let Some(gc_id) = Self::value_to_gc_id(&value) {
                    if is_dead(gc_id) {
                        should_remove = true;
                    }
                }
            }

            if should_remove {
                keys_to_remove.push(key);
            }
        }

        // Remove marked keys
        for key in keys_to_remove {
            self.raw_set(&key, LuaValue::nil());
        }
    }

    /// Convert LuaValue to GcId for dead object checking
    fn value_to_gc_id(value: &LuaValue) -> Option<crate::gc::GcId> {
        use crate::gc::GcId;
        use crate::lua_value::LuaValueKind;

        match value.kind() {
            LuaValueKind::String => value.as_string_id().map(GcId::StringId),
            LuaValueKind::Table => value.as_table_id().map(GcId::TableId),
            LuaValueKind::Function => value.as_function_id().map(GcId::FunctionId),
            LuaValueKind::Thread => value.as_thread_id().map(GcId::ThreadId),
            LuaValueKind::Userdata => value.as_userdata_id().map(GcId::UserdataId),
            _ => None,
        }
    }
}

pub enum LuaTableDetail {
    // TypedArray(LuaTypedArray),
    ValueArray(LuaValueArray),
    HashTable(LuaHashTable),
}

pub enum LuaInsertResult {
    Success,
    NeedConvertToValueArray,
    NeedConvertToHashTable,
    Failure,
}

#[cfg(test)]
mod test {

    #[test]
    fn test_table_set_get() {
        // let mut table = LuaTable::new(0, 0);
        // let mut pool = ObjectPool::new();
        // let s = pool.create_string("hello").0;
        // table.set_int(1, LuaValue::integer(42));
        // table.set_int(2, LuaValue::string(s));
        // table.raw_set(&LuaValue::string(s), LuaValue::integer(100));

        // assert_eq!(table.get_int(1).unwrap().as_integer().unwrap(), 42);
        // assert_eq!(table.get_int(2).unwrap(), LuaValue::string(s));
        // assert_eq!(
        //     table
        //         .raw_get(&LuaValue::string(s))
        //         .unwrap()
        //         .as_integer()
        //         .unwrap(),
        //     100
        // );
    }
}
