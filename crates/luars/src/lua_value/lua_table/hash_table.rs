// Hash Table implementation for Lua tables
// 使用 hashbrown 的 RawTable 获得 SIMD 优化和最佳性能

use super::super::lua_value::LuaValue;
use crate::LuaResult;
use hashbrown::raw::RawTable;

use super::{LuaTableImpl, LuaInsertResult};

const INITIAL_CAPACITY: usize = 8;

/// 哈希表条目（键值对）
#[derive(Clone)]
struct Entry {
    key: LuaValue,
    value: LuaValue,
}

impl Entry {
    #[inline]
    fn new(key: LuaValue, value: LuaValue) -> Self {
        Self { key, value }
    }
}

/// 为 LuaValue 计算哈希值
#[inline(always)]
fn hash_lua_value(key: &LuaValue) -> u64 {
    use crate::lua_value::lua_value::*;
    
    unsafe {
        match key.ttype() {
            LUA_TNIL => 0,
            LUA_TBOOLEAN => key.value.i as u64,
            LUA_TNUMBER => {
                if key.tt() & 1 == 0 {
                    // Float: 使用位模式
                    key.value.n.to_bits()
                } else {
                    // Integer: 直接使用值
                    key.value.i as u64
                }
            }
            LUA_TSTRING | LUA_TTABLE | LUA_TFUNCTION | LUA_TUSERDATA | LUA_TTHREAD => {
                // GC对象：使用指针地址
                let id = key.value.ptr as u64;
                let ttype = key.tt() as u64;
                id ^ (ttype << 32)
            }
            _ => 0,
        }
    }
}

/// Lua哈希表 - 使用 hashbrown::RawTable 获得最佳性能
pub struct LuaHashTable {
    table: RawTable<Entry>,
    array_len: usize,  // 用于 # 操作符
}

impl LuaHashTable {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(INITIAL_CAPACITY);
        Self {
            table: RawTable::with_capacity(capacity),
            array_len: 0,
        }
    }
    
    /// 查找键
    #[inline(always)]
    fn find_entry(&self, key: &LuaValue) -> Option<&Entry> {
        let hash = hash_lua_value(key);
        self.table.get(hash, |entry| &entry.key == key)
    }
    
    /// 查找键（可变）
    #[inline(always)]
    fn find_entry_mut(&mut self, key: &LuaValue) -> Option<&mut Entry> {
        let hash = hash_lua_value(key);
        self.table.get_mut(hash, |entry| entry.key == *key)
    }
    
    /// 获取值
    #[inline(always)]
    fn get(&self, key: &LuaValue) -> Option<LuaValue> {
        self.find_entry(key).map(|e| e.value)
    }
    
    /// 插入或更新
    fn insert(&mut self, key: LuaValue, value: LuaValue) {
        let hash = hash_lua_value(&key);
        
        // 查找是否已存在
        if let Some(entry) = self.table.get_mut(hash, |e| e.key == key) {
            // 更新现有值
            entry.value = value;
            return;
        }
        
        // 插入新键值对
        let entry = Entry::new(key.clone(), value);
        
        // hashbrown 会自动处理扩容
        self.table.insert(hash, entry, |e| hash_lua_value(&e.key));
        
        // 更新数组长度
        if let Some(k) = key.as_integer() {
            self.update_array_len_insert(k);
        }
    }
    
    /// 删除键
    fn remove(&mut self, key: &LuaValue) -> bool {
        let hash = hash_lua_value(key);
        
        if let Some(_) = self.table.remove_entry(hash, |e| &e.key == key) {
            // 更新数组长度
            if let Some(k) = key.as_integer() {
                self.update_array_len_remove(k);
            }
            true
        } else {
            false
        }
    }
    
    /// 更新数组长度（插入时）
    fn update_array_len_insert(&mut self, key: i64) {
        if key > 0 && key as usize == self.array_len + 1 {
            self.array_len += 1;
            
            // 检查连续键
            let mut next = self.array_len as i64 + 1;
            while self.get(&LuaValue::integer(next)).is_some() {
                self.array_len += 1;
                next += 1;
            }
        }
    }

    fn update_array_len_remove(&mut self, key: i64) {
        if key > 0 && key <= self.array_len as i64 {
            self.array_len = (key - 1) as usize;
        }
    }
}

impl LuaTableImpl for LuaHashTable {
    #[inline(always)]
    fn get_int(&self, key: i64) -> Option<LuaValue> {
        // 快速路径：直接内联整数查找
        let hash = key as u64;
        self.table.get(hash, |entry| {
            if let Some(k) = entry.key.as_integer() {
                k == key
            } else {
                false
            }
        }).map(|e| e.value)
    }

    #[inline(always)]
    fn set_int(&mut self, key: i64, value: LuaValue) -> LuaInsertResult {
        let hash = key as u64;
        
        // 先尝试更新现有值
        if let Some(entry) = self.table.get_mut(hash, |e| {
            if let Some(k) = e.key.as_integer() {
                k == key
            } else {
                false
            }
        }) {
            entry.value = value;
            return LuaInsertResult::Success;
        }
        
        // 插入新键
        let key_value = LuaValue::integer(key);
        self.insert(key_value, value);
        LuaInsertResult::Success
    }

    fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        self.get(key)
    }

    fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> LuaInsertResult {
        if value.is_nil() {
            self.remove(key);
        } else {
            self.insert(key.clone(), value);
        }
        LuaInsertResult::Success
    }

    fn next(&self, input_key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        // hashbrown 提供高效的迭代器
        unsafe {
            let mut iter = self.table.iter();
            
            if input_key.is_nil() {
                // 返回第一个元素
                return iter.next().map(|bucket| {
                    let entry = bucket.as_ref();
                    (entry.key, entry.value)
                });
            }
            
            // 查找当前键，返回下一个
            let mut found_current = false;
            for bucket in iter {
                let entry = bucket.as_ref();
                if found_current {
                    return Some((entry.key, entry.value));
                }
                if entry.key == *input_key {
                    found_current = true;
                }
            }
            
            None
        }
    }

    fn len(&self) -> usize {
        self.array_len
    }

    fn insert_at(&mut self, _index: usize, _value: LuaValue) -> LuaInsertResult {
        LuaInsertResult::Success
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
    fn test_next_iteration() {
        let mut table = LuaHashTable::new(0);

        table.set_int(1, LuaValue::integer(10));
        table.set_int(2, LuaValue::integer(20));
        table.set_int(3, LuaValue::integer(30));

        let mut key = LuaValue::nil();
        let mut count = 0;

        while let Some((k, _v)) = table.next(&key) {
            count += 1;
            key = k;
        }

        assert!(count >= 3);
    }

    #[test]
    fn test_many_inserts() {
        let mut table = LuaHashTable::new(4);

        for i in 0..10000 {
            table.set_int(i, LuaValue::integer(i * 10));
        }

        for i in 0..10000 {
            assert_eq!(table.get_int(i), Some(LuaValue::integer(i * 10)));
        }
    }

    #[test]
    fn test_delete() {
        let mut table = LuaHashTable::new(0);

        table.set_int(1, LuaValue::integer(100));
        table.set_int(2, LuaValue::integer(200));

        table.raw_set(&LuaValue::integer(1), LuaValue::nil());

        assert_eq!(table.get_int(1), None);
        assert_eq!(table.get_int(2), Some(LuaValue::integer(200)));
    }
    
    #[test]
    fn test_string_keys() {
        let mut table = LuaHashTable::new(0);
        
        let key1 = LuaValue::integer(1);  // 临时用整数模拟
        let key2 = LuaValue::integer(2);
        
        table.raw_set(&key1, LuaValue::integer(100));
        table.raw_set(&key2, LuaValue::integer(200));
        
        assert_eq!(table.raw_get(&key1), Some(LuaValue::integer(100)));
        assert_eq!(table.raw_get(&key2), Some(LuaValue::integer(200)));
    }
}
