// LuaTable - Rust优化的Lua Table实现
mod hash_table;
mod type_array;
mod value_array;

use super::lua_value::LuaValue;
use crate::{
    LuaResult, TableId,
    lua_value::lua_table::{hash_table::LuaHashTable, value_array::LuaValueArray},
    lua_vm::LuaError,
};

pub struct LuaTable {
    /// 压缩的元数据字段
    /// Layout: flags(8) | lsizenode(8) | metatable_id(48)
    /// metatable_id为0表示没有metatable
    /// metatable_id为n+1表示TableId(n) (1-based以避免0冲突)
    meta: u64,

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
            meta: Self::pack_meta(0, 0, None),
            impl_table,
        }
    }

    // ============ Meta字段的压缩/解压 ============

    /// 打包meta字段: flags(8) | lsizenode(8) | metatable_id(48)
    #[inline(always)]
    fn pack_meta(flags: u8, lsizenode: u8, metatable: Option<TableId>) -> u64 {
        let metatable_bits = match metatable {
            None => 0u64,
            Some(TableId(id)) => (id as u64) + 1, // 1-based以避免0
        };
        (flags as u64) | ((lsizenode as u64) << 8) | (metatable_bits << 16)
    }

    /// 获取metatable
    #[inline(always)]
    fn metatable(&self) -> Option<crate::TableId> {
        let bits = self.meta >> 16;
        if bits == 0 {
            None
        } else {
            Some(TableId((bits - 1) as u32))
        }
    }

    /// 设置metatable
    #[inline(always)]
    fn set_metatable_internal(&mut self, metatable: Option<TableId>) {
        let metatable_bits = match metatable {
            None => 0u64,
            Some(TableId(id)) => (id as u64) + 1,
        };
        self.meta = (self.meta & 0xFFFF) | (metatable_bits << 16);
    }

    pub fn get_metatable(&self) -> Option<TableId> {
        let id = self.metatable()?;
        Some(id)
    }

    pub fn set_metatable(&mut self, metatable: Option<LuaValue>) {
        let metatable_id = metatable.and_then(|v| v.as_table_id());
        self.set_metatable_internal(metatable_id);
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
        let old_impl = std::mem::replace(
            &mut self.impl_table,
            LuaTableDetail::HashTable(LuaHashTable::new(len)),
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
                for (k, v) in t.table.iter() {
                    result.push((k.clone(), v.clone()));
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
