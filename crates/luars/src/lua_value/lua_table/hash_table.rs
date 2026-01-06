use crate::{
    LuaResult, LuaValue,
    lua_value::{LuaTableImpl, lua_table::LuaInsertResult},
};
use hashbrown::HashTable;
use std::hash::{Hash, Hasher};

pub struct LuaHashTable {
    pub(crate) table: HashTable<(LuaValue, LuaValue)>,
}

impl LuaHashTable {
    pub fn new(capacity: usize) -> Self {
        Self {
            table: HashTable::with_capacity(capacity),
        }
    }

    /// 计算LuaValue的hash
    #[inline]
    fn hash_key(key: &LuaValue) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }
}

impl LuaTableImpl for LuaHashTable {
    fn get_int(&self, key: i64) -> Option<LuaValue> {
        let key_value = LuaValue::integer(key);
        let hash = Self::hash_key(&key_value);

        self.table
            .find(hash, |(k, _)| k == &key_value)
            .map(|(_, v)| *v)
    }

    fn set_int(&mut self, key: i64, value: LuaValue) -> LuaInsertResult {
        let key_value = LuaValue::integer(key);
        let hash = Self::hash_key(&key_value);

        match self.table.find_entry(hash, |(k, _)| k == &key_value) {
            Ok(mut entry) => {
                // 键已存在，更新值
                entry.get_mut().1 = value;
            }
            Err(_) => {
                // 键不存在，插入新键值对
                self.table
                    .insert_unique(hash, (key_value, value), |(k, _)| Self::hash_key(k));
            }
        }

        LuaInsertResult::Success
    }

    fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        let hash = Self::hash_key(key);

        self.table.find(hash, |(k, _)| k == key).map(|(_, v)| *v)
    }

    fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> LuaInsertResult {
        let hash = Self::hash_key(key);

        match self.table.find_entry(hash, |(k, _)| k == key) {
            Ok(mut entry) => {
                // 键已存在，更新值
                entry.get_mut().1 = value;
            }
            Err(_) => {
                // 键不存在，插入新键值对
                self.table
                    .insert_unique(hash, (key.clone(), value), |(k, _)| Self::hash_key(k));
            }
        }

        LuaInsertResult::Success
    }

    fn next(&self, input_key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        if input_key.is_nil() {
            // 返回第一个元素
            self.table.iter().next().map(|(k, v)| (*k, *v))
        } else {
            // 遍历找到input_key，返回下一个
            // 注意：这仍然是O(n)，但这是HashTable遍历的固有特性
            let mut found = false;
            for (k, v) in self.table.iter() {
                if found {
                    return Some((*k, *v));
                }
                if k == input_key {
                    found = true;
                }
            }
            None
        }
    }

    fn len(&self) -> usize {
        self.table.len()
    }

    fn insert_at(&mut self, _index: usize, _value: LuaValue) -> LuaInsertResult {
        LuaInsertResult::Success
    }

    fn remove_at(&mut self, _index: usize) -> LuaResult<LuaValue> {
        Ok(LuaValue::nil())
    }
}
