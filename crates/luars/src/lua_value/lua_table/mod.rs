// LuaTable - Rust优化的Lua Table实现
mod hash_table;
mod type_array;
mod value_array;

use super::lua_value::LuaValue;
use crate::{
    LuaResult, TablePtr,
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
        match &self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.len(),
            LuaTableDetail::ValueArray(arr) => arr.len(),
            LuaTableDetail::HashTable(map) => map.len(),
        }
    }

    pub fn hash_size(&self) -> usize {
        match &self.impl_table {
            // LuaTableDetail::TypedArray(_) => 0,
            LuaTableDetail::ValueArray(_) => 0,
            LuaTableDetail::HashTable(map) => map.hash_size(),
        }
    }

    pub fn is_array(&self) -> bool {
        match &self.impl_table {
            // LuaTableDetail::TypedArray(_) => true,
            LuaTableDetail::ValueArray(_) => true,
            LuaTableDetail::HashTable(_) => false,
        }
    }

    pub fn raw_geti(&self, key: i64) -> Option<LuaValue> {
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

    pub(crate) fn raw_seti(&mut self, key: i64, value: LuaValue) {
        let r = match &mut self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.set_int(key, value),
            LuaTableDetail::ValueArray(arr) => arr.set_int(key, value),
            LuaTableDetail::HashTable(map) => map.set_int(key, value),
        };

        match r {
            LuaInsertResult::Update
            | LuaInsertResult::NewKeyInserted
            | LuaInsertResult::Failure => {}
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

    /// return true if new key inserted, false if updated existing key
    pub(crate) fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> bool {
        let r = match &mut self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.raw_set(key, value),
            LuaTableDetail::ValueArray(arr) => arr.raw_set(key, value),
            LuaTableDetail::HashTable(map) => map.raw_set(key, value),
        };

        match r {
            LuaInsertResult::Update | LuaInsertResult::Failure => false,
            LuaInsertResult::NewKeyInserted => true,
            LuaInsertResult::NeedConvertToValueArray => {
                self.migrate_to_value_array();
                if let LuaTableDetail::ValueArray(arr) = &mut self.impl_table {
                    arr.raw_set(key, value);
                }

                true
            }
            LuaInsertResult::NeedConvertToHashTable => {
                self.migrate_to_hash_table();
                if let LuaTableDetail::HashTable(map) = &mut self.impl_table {
                    map.raw_set(key, value);
                }

                true
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

    /// Insert value at array index i (1-based)
    /// return true if new key inserted, false if updated existing key
    pub(crate) fn insert_array_at(&mut self, i: i64, value: LuaValue) -> LuaResult<bool> {
        let index = (i - 1) as usize;
        let r = match &mut self.impl_table {
            // LuaTableDetail::TypedArray(arr) => arr.insert_at(index, value),
            LuaTableDetail::ValueArray(arr) => arr.insert_at(index, value),
            LuaTableDetail::HashTable(map) => map.insert_at(index, value),
        };

        match r {
            LuaInsertResult::Update => return Ok(false),
            LuaInsertResult::NewKeyInserted => {}
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

        Ok(true)
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

    pub fn iter_keys(&self) -> Vec<LuaValue> {
        let mut result = Vec::new();
        match &self.impl_table {
            // LuaTableDetail::TypedArray(ar) => {
            //     for i in 0..ar.array.len() {
            //         let key = LuaValue::integer((i + 1) as i64);
            //         result.push(key);
            //     }
            // }
            LuaTableDetail::ValueArray(ar) => {
                for i in 0..ar.array.len() {
                    let key = LuaValue::integer((i + 1) as i64);
                    result.push(key);
                }
            }
            LuaTableDetail::HashTable(t) => {
                // 使用 next 方法遵循接口遍历
                let mut key = LuaValue::nil();
                while let Some((k, _v)) = t.next(&key) {
                    result.push(k.clone());
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

pub enum LuaTableDetail {
    // TypedArray(LuaTypedArray),
    ValueArray(LuaValueArray),
    HashTable(LuaHashTable),
}

pub enum LuaInsertResult {
    Update,
    NewKeyInserted,
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
