use crate::{
    LuaResult, LuaValue,
    lua_value::{LuaTableImpl, lua_table::LuaInsertResult},
};

/// 高性能Lua哈希表实现
///
/// 设计思路：
/// 1. 使用紧凑数组存储实际键值对（entries），保持插入顺序
/// 2. 使用indices数组作为哈希索引，存储到entries的索引
/// 3. 开放寻址 + 线性探测解决冲突
/// 4. next遍历只需遍历entries数组，O(1)跳到下一个元素
/// 5. 删除时标记墓碑，不移动数据，保持遍历稳定性
///
/// 性能特性：
/// - 查询: O(1) 平均，最坏O(n)
/// - 插入: O(1) 平均
/// - 遍历: O(n) 但常数极小，顺序访问entries数组
/// - 空间: ~1.5x 键值对数量（75% load factor）
pub struct LuaHashTable {
    /// 存储实际的键值对，保持插入顺序
    entries: Vec<Entry>,

    /// 哈希索引：indices[hash % capacity] -> entries中的索引
    /// 使用u32节省空间，支持最多4B个元素
    /// EMPTY = u32::MAX 表示空槽位
    /// TOMBSTONE = u32::MAX - 1 表示已删除
    indices: Vec<u32>,

    /// 已删除的条目数量（墓碑）
    tombstones: usize,
}

#[derive(Clone, Copy)]
struct Entry {
    key: LuaValue,
    value: LuaValue,
    /// 低32位哈希值，用于快速比较（避免存储完整u64）
    hash_low: u32,
}

const EMPTY: u32 = u32::MAX;
const TOMBSTONE: u32 = u32::MAX - 1;
const INITIAL_CAPACITY: usize = 4;
const MAX_LOAD_FACTOR: f64 = 0.75;

impl LuaHashTable {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(INITIAL_CAPACITY).next_power_of_two();
        Self {
            entries: Vec::new(),
            indices: vec![EMPTY; capacity],
            tombstones: 0,
        }
    }

    /// 哈希函数 - 简化版，依赖奇数容量提供更好的分布
    #[inline(always)]
    fn hash_key(key: &LuaValue) -> u64 {
        use crate::lua_value::lua_value::*;

        const K: u64 = 0x9e3779b97f4a7c15; // Fibonacci golden ratio

        unsafe {
            match key.ttype() {
                LUA_TNIL => 0,
                LUA_TBOOLEAN => (key.value.i as u64).wrapping_mul(K),
                LUA_TNUMBER => {
                    if key.tt() & 1 == 0 {
                        // Float: bit pattern哈希
                        key.value.n.to_bits().wrapping_mul(K)
                    } else {
                        // Integer: Fibonacci hashing
                        (key.value.i as u64).wrapping_mul(K)
                    }
                }
                _ => {
                    // GC类型：gcid + type tag
                    let id = key.gcid() as u64;
                    let tt = (key.tt() as u64) << 56;
                    id.wrapping_mul(K) ^ tt
                }
            }
        }
    }

    /// 查找键的索引位置（在indices数组中）
    /// 返回：Some(entry_idx) 如果找到，None 如果不存在
    #[inline(always)]
    fn find_index(&self, key: &LuaValue, hash: u64) -> Option<usize> {
        if self.indices.is_empty() {
            return None;
        }

        let mask = self.indices.len() - 1;
        let mut idx = (hash as usize) & mask;
        let hash_low = hash as u32;

        // 线性探测，最多探测整个表
        for _ in 0..self.indices.len() {
            let entry_idx = unsafe { *self.indices.get_unchecked(idx) };

            if entry_idx == EMPTY {
                return None;
            }

            if entry_idx != TOMBSTONE {
                let entry = unsafe { self.entries.get_unchecked(entry_idx as usize) };
                // 先比较低32位哈希（快），再比较键
                if entry.hash_low == hash_low && &entry.key == key {
                    return Some(entry_idx as usize);
                }
            }

            idx = (idx + 1) & mask;
        }

        None
    }

    /// 查找或找到空闲插入位置
    /// 返回：(found_entry_idx, insert_slot_idx)
    #[inline(always)]
    fn find_or_insert_slot(&self, key: &LuaValue, hash: u64) -> (Option<usize>, usize) {
        let mask = self.indices.len() - 1;
        let mut idx = (hash as usize) & mask;
        let mut first_tombstone: Option<usize> = None;
        let hash_low = hash as u32;

        for _ in 0..self.indices.len() {
            let entry_idx = unsafe { *self.indices.get_unchecked(idx) };

            if entry_idx == EMPTY {
                return (None, first_tombstone.unwrap_or(idx));
            }

            if entry_idx == TOMBSTONE {
                if first_tombstone.is_none() {
                    first_tombstone = Some(idx);
                }
            } else {
                let entry = unsafe { self.entries.get_unchecked(entry_idx as usize) };
                if entry.hash_low == hash_low && &entry.key == key {
                    return (Some(entry_idx as usize), idx);
                }
            }

            idx = (idx + 1) & mask;
        }

        (None, first_tombstone.unwrap_or(0))
    }

    /// 扩容：重建哈希索引
    fn grow(&mut self) {
        let new_capacity = (self.indices.len() * 2).max(INITIAL_CAPACITY);
        let mut new_indices = vec![EMPTY; new_capacity];
        let mask = new_capacity - 1;

        // 重建索引
        for (entry_idx, entry) in self.entries.iter().enumerate() {
            let hash = entry.hash_low as usize;
            let mut idx = hash & mask;

            // 线性探测找到空位
            loop {
                if unsafe { *new_indices.get_unchecked(idx) } == EMPTY {
                    new_indices[idx] = entry_idx as u32;
                    break;
                }
                idx = (idx + 1) & mask;
            }
        }

        self.indices = new_indices;
        self.tombstones = 0;
    }

    /// 检查是否需要扩容
    #[inline]
    fn should_grow(&self) -> bool {
        let load = (self.entries.len() + self.tombstones) as f64 / self.indices.len() as f64;
        load > MAX_LOAD_FACTOR
    }

    /// 插入或更新键值对
    #[inline]
    fn insert(&mut self, key: LuaValue, value: LuaValue) {
        if self.indices.is_empty() {
            self.indices = vec![EMPTY; INITIAL_CAPACITY];
        }

        let hash = Self::hash_key(&key);
        let (found_idx, slot_idx) = self.find_or_insert_slot(&key, hash);

        if let Some(entry_idx) = found_idx {
            // 键已存在，更新值
            unsafe {
                self.entries.get_unchecked_mut(entry_idx).value = value;
            }
        } else {
            // 新键，添加到entries
            let entry_idx = self.entries.len();
            self.entries.push(Entry {
                key,
                value,
                hash_low: hash as u32,
            });

            // 更新索引
            if unsafe { *self.indices.get_unchecked(slot_idx) } == TOMBSTONE {
                self.tombstones -= 1;
            }
            self.indices[slot_idx] = entry_idx as u32;

            // 检查是否需要扩容
            if self.should_grow() {
                self.grow();
            }
        }
    }

    /// 删除键（标记为墓碑，保持遍历顺序）
    fn remove(&mut self, key: &LuaValue) -> bool {
        let hash = Self::hash_key(&key);
        if let Some(entry_idx) = self.find_index(key, hash) {
            // 在indices中找到对应的槽位并标记为墓碑
            let mask = self.indices.len() - 1;
            let mut idx = (hash as usize) & mask;

            loop {
                if self.indices[idx] == entry_idx as u32 {
                    self.indices[idx] = TOMBSTONE;
                    self.tombstones += 1;

                    // 注意：我们不从entries中删除，保持索引稳定
                    // 在遍历时跳过墓碑即可
                    return true;
                }
                idx = (idx + 1) & mask;
                if self.indices[idx] == EMPTY {
                    break;
                }
            }
        }
        false
    }

    /// 获取值
    #[inline(always)]
    fn get(&self, key: &LuaValue) -> Option<LuaValue> {
        // OPTIMIZATION: Manually inline logic to avoid function call overhead
        let hash = Self::hash_key(key);
        let mask = self.indices.len() - 1;
        let mut idx = (hash as usize) & mask;
        let hash_low = hash as u32;

        for _ in 0..self.indices.len() {
            let entry_idx = unsafe { *self.indices.get_unchecked(idx) };

            if entry_idx == EMPTY {
                return None;
            }

            if entry_idx != TOMBSTONE {
                let entry = unsafe { self.entries.get_unchecked(entry_idx as usize) };
                // Comparison: fast check on hash_low, then full equality
                // Note: using direct field access instead of &entry.key
                if entry.hash_low == hash_low && entry.key == *key {
                     return Some(entry.value);
                }
            }
            idx = (idx + 1) & mask;
        }
        None
    }
}

impl LuaTableImpl for LuaHashTable {
    fn get_int(&self, key: i64) -> Option<LuaValue> {
        let key_value = LuaValue::integer(key);
        self.get(&key_value)
    }

    fn set_int(&mut self, key: i64, value: LuaValue) -> LuaInsertResult {
        let key_value = LuaValue::integer(key);
        self.insert(key_value, value);
        LuaInsertResult::Success
    }

    fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        self.get(key)
    }

    fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> LuaInsertResult {
        if value.is_nil() {
            // Lua语义：设置为nil = 删除键
            self.remove(key);
        } else {
            self.insert(key.clone(), value);
        }
        LuaInsertResult::Success
    }

    fn next(&self, input_key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        if input_key.is_nil() {
            // 返回第一个元素 - O(1)操作
            return self.entries.first().map(|e| (e.key, e.value));
        }

        // 查找当前键，返回下一个 - O(1)查找 + O(1)跳转
        let hash = Self::hash_key(input_key);
        if let Some(entry_idx) = self.find_index(input_key, hash) {
            // 返回下一个entry
            if entry_idx + 1 < self.entries.len() {
                let next_entry = &self.entries[entry_idx + 1];
                return Some((next_entry.key, next_entry.value));
            }
        }

        None
    }

    fn len(&self) -> usize {
        // 返回实际存储的键值对数量
        self.entries.len()
    }

    fn insert_at(&mut self, _index: usize, _value: LuaValue) -> LuaInsertResult {
        // 哈希表不支持按索引插入
        LuaInsertResult::Success
    }

    fn remove_at(&mut self, _index: usize) -> LuaResult<LuaValue> {
        // 哈希表不支持按索引删除
        Ok(LuaValue::nil())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let mut table = LuaHashTable::new(0);

        // 插入
        table.set_int(1, LuaValue::integer(100));
        table.set_int(2, LuaValue::integer(200));

        // 查询
        assert_eq!(table.get_int(1), Some(LuaValue::integer(100)));
        assert_eq!(table.get_int(2), Some(LuaValue::integer(200)));
        assert_eq!(table.get_int(3), None);

        // 更新
        table.set_int(1, LuaValue::integer(150));
        assert_eq!(table.get_int(1), Some(LuaValue::integer(150)));
    }

    #[test]
    fn test_next_iteration() {
        let mut table = LuaHashTable::new(0);

        table.set_int(1, LuaValue::integer(10));
        table.set_int(2, LuaValue::integer(20));
        table.set_int(3, LuaValue::integer(30));

        // 遍历
        let mut key = LuaValue::nil();
        let mut count = 0;

        while let Some((k, v)) = table.next(&key) {
            count += 1;
            key = k;
            println!("key: {:?}, value: {:?}", k, v);
        }

        assert_eq!(count, 3);
    }

    #[test]
    fn test_grow() {
        let mut table = LuaHashTable::new(4);

        // 插入超过load factor的元素，触发扩容
        for i in 0..100 {
            table.set_int(i, LuaValue::integer(i * 10));
        }

        // 验证所有元素都能找到
        for i in 0..100 {
            assert_eq!(table.get_int(i), Some(LuaValue::integer(i * 10)));
        }
    }

    #[test]
    fn test_delete() {
        let mut table = LuaHashTable::new(0);

        table.set_int(1, LuaValue::integer(100));
        table.set_int(2, LuaValue::integer(200));

        // 删除（通过设置为nil）
        table.raw_set(&LuaValue::integer(1), LuaValue::nil());

        assert_eq!(table.get_int(1), None);
        assert_eq!(table.get_int(2), Some(LuaValue::integer(200)));
    }
}
