use crate::{
    LuaResult, LuaValue,
    lua_value::{LuaTableImpl, lua_table::LuaInsertResult},
};
use ahash::RandomState;
use indexmap::IndexMap;

/// 高性能Lua哈希表实现 - 基于 indexmap + ahash
///
/// 使用 indexmap::IndexMap 提供：
/// 1. 高性能哈希表操作（基于 hashbrown）
/// 2. 保持插入顺序（对 next() 迭代很重要）
/// 3. ahash 提供更快的哈希算法
pub struct LuaHashTable {
    /// 核心哈希表（使用 ahash）
    map: IndexMap<LuaValue, LuaValue, RandomState>,

    /// 数组部分长度记录（#操作符）
    array_len: usize,
}

impl LuaHashTable {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: IndexMap::with_capacity_and_hasher(capacity, RandomState::new()),
            array_len: 0,
        }
    }

    /// 更新数组长度
    fn update_array_len_insert(&mut self, key: i64) {
        if key == (self.array_len as i64 + 1) {
            self.array_len += 1;
        }
    }

    fn update_array_len_remove(&mut self, key: i64) {
        if key == self.array_len as i64 {
            self.array_len -= 1;
        }
    }
}

impl LuaTableImpl for LuaHashTable {
    #[inline(always)]
    fn get_int(&self, key: i64) -> Option<LuaValue> {
        let key_value = LuaValue::integer(key);
        self.map.get(&key_value).copied()
    }

    #[inline(always)]
    fn set_int(&mut self, key: i64, value: LuaValue) -> LuaInsertResult {
        let key_value = LuaValue::integer(key);

        if value.is_nil() {
            self.map.shift_remove(&key_value);
            self.update_array_len_remove(key)
        } else {
            self.map.insert(key_value, value);
            self.update_array_len_insert(key)
        }

        LuaInsertResult::Update
    }

    fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        self.map.get(key).copied()
    }

    fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> LuaInsertResult {
        let mut result = LuaInsertResult::Update;
        if value.is_nil() {
            self.map.shift_remove(key);
            if let Some(k) = key.as_integer() {
                self.update_array_len_remove(k);
            }
        } else {
            if self.map.insert(*key, value).is_none() {
                result = LuaInsertResult::NewKeyInserted;
            }
            if let Some(k) = key.as_integer() {
                self.update_array_len_insert(k);
            }
        }
        
        result
    }

    fn next(&self, input_key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        if input_key.is_nil() {
            // 返回第一个键值对
            return self.map.get_index(0).map(|(k, v)| (*k, *v));
        }

        // 找到当前键的索引，然后返回下一个
        if let Some(index) = self.map.get_index_of(input_key) {
            return self.map.get_index(index + 1).map(|(k, v)| (*k, *v));
        }

        None
    }

    fn len(&self) -> usize {
        self.array_len
    }

    fn insert_at(&mut self, _index: usize, _value: LuaValue) -> LuaInsertResult {
        LuaInsertResult::Update
    }

    fn remove_at(&mut self, _index: usize) -> LuaResult<LuaValue> {
        Ok(LuaValue::nil())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let mut table = LuaHashTable::new(0);

        table.set_int(1, LuaValue::integer(100));
        table.set_int(2, LuaValue::integer(200));

        assert_eq!(table.get_int(1), Some(LuaValue::integer(100)));
        assert_eq!(table.get_int(2), Some(LuaValue::integer(200)));
        assert_eq!(table.get_int(3), None);

        table.set_int(1, LuaValue::integer(150));
        assert_eq!(table.get_int(1), Some(LuaValue::integer(150)));
    }

    #[test]
    fn test_grow() {
        let mut table = LuaHashTable::new(4);

        for i in 0..100 {
            table.set_int(i, LuaValue::integer(i * 10));
        }

        for i in 0..100 {
            assert_eq!(table.get_int(i), Some(LuaValue::integer(i * 10)));
        }
    }

    #[test]
    fn test_chain_collision() {
        let mut table = LuaHashTable::new(5);

        for i in 0..20 {
            table.set_int(i, LuaValue::integer(i * 100));
        }

        for i in 0..20 {
            assert_eq!(table.get_int(i), Some(LuaValue::integer(i * 100)));
        }
    }

    #[test]
    fn test_next() {
        let mut table = LuaHashTable::new(0);

        table.set_int(1, LuaValue::integer(10));
        table.set_int(2, LuaValue::integer(20));
        table.set_int(3, LuaValue::integer(30));

        // 从 nil 开始
        let (k1, v1) = table.next(&LuaValue::nil()).unwrap();
        assert_eq!(k1, LuaValue::integer(1));
        assert_eq!(v1, LuaValue::integer(10));

        // 继续迭代
        let (k2, v2) = table.next(&k1).unwrap();
        assert_eq!(k2, LuaValue::integer(2));
        assert_eq!(v2, LuaValue::integer(20));

        let (k3, v3) = table.next(&k2).unwrap();
        assert_eq!(k3, LuaValue::integer(3));
        assert_eq!(v3, LuaValue::integer(30));

        // 结束
        assert!(table.next(&k3).is_none());
    }
}
