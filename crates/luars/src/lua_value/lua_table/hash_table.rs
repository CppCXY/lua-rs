use crate::{
    LuaResult, LuaValue,
    lua_value::{LuaTableImpl, lua_table::LuaInsertResult},
};
use ahash::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};

/// 高性能Lua哈希表实现 - 简化的链式哈希
///
/// 关键特性：
/// 1. 开放寻址 + 链式哈希混合
/// 2. 使用绝对索引而非相对偏移（避免溢出）
/// 3. 使用 ahash 快速哈希算法
/// 4. 快速路径优化
pub struct LuaHashTable {
    /// 节点数组：直接存储键值对 + 链表指针
    nodes: Vec<Node>,

    /// 下一个空闲位置（从后向前搜索）
    last_free: usize,

    /// 数组部分长度记录（#操作符）
    array_len: usize,

    /// 实际元素数量
    count: usize,

    /// ahash 随机状态（用于快速哈希）
    hasher_state: RandomState,
}

/// 哈希节点
#[derive(Clone)]
struct Node {
    key: LuaValue,
    value: LuaValue,
    /// 链表指针：直接存储下一个节点索引 (usize::MAX = 链尾)
    next: usize,
}

const INITIAL_CAPACITY: usize = 4;
const DEAD_KEY: LuaValue = LuaValue::nil();
const NO_NEXT: usize = usize::MAX;

impl Node {
    #[inline(always)]
    fn new_empty() -> Self {
        Self {
            key: DEAD_KEY,
            value: LuaValue::nil(),
            next: NO_NEXT,
        }
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.key.is_nil() && self.next == NO_NEXT
    }

    #[inline(always)]
    fn is_occupied(&self) -> bool {
        !self.key.is_nil()
    }
}

impl LuaHashTable {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(INITIAL_CAPACITY);
        let nodes = vec![Node::new_empty(); capacity];
        let last_free = capacity;

        Self {
            nodes,
            last_free,
            array_len: 0,
            count: 0,
            hasher_state: RandomState::new(),
        }
    }

    /// 快速哈希函数 - 使用 ahash 和 LuaValue 的 Hash trait
    #[inline(always)]
    fn hash_key(&self, key: &LuaValue) -> u64 {
        let mut hasher = self.hasher_state.build_hasher();
        key.hash(&mut hasher);
        hasher.finish()
    }

    /// 计算主位置
    #[inline(always)]
    fn main_position(&self, hash: u64) -> usize {
        (hash as usize) % self.nodes.len()
    }

    /// 查找空闲位置
    fn get_free_pos(&mut self) -> Option<usize> {
        while self.last_free > 0 {
            self.last_free -= 1;
            if self.nodes[self.last_free].is_empty() {
                return Some(self.last_free);
            }
        }
        None
    }

    /// 扩容并重新哈希
    fn resize(&mut self) {
        let new_capacity = (self.nodes.len() * 2).max(INITIAL_CAPACITY);
        let old_nodes = std::mem::replace(
            &mut self.nodes,
            vec![Node::new_empty(); new_capacity],
        );
        
        self.last_free = new_capacity;
        self.count = 0;
        self.array_len = 0;

        // 重新插入所有元素
        for node in old_nodes {
            if node.is_occupied() {
                self.insert(node.key, node.value);
            }
        }
    }

    /// 增长到指定大小
    fn grow_to_size(&mut self, min_size: usize) {
        if self.nodes.len() < min_size {
            let new_capacity = min_size.max(INITIAL_CAPACITY);
            let old_nodes = std::mem::replace(
                &mut self.nodes,
                vec![Node::new_empty(); new_capacity],
            );
            
            self.last_free = new_capacity;
            self.count = 0;
            self.array_len = 0;

            // 重新插入所有元素
            for node in old_nodes {
                if node.is_occupied() {
                    self.insert(node.key, node.value);
                }
            }
        }
    }

    /// 查找节点索引
    fn find_node(&self, key: &LuaValue) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }

        let hash = self.hash_key(key);
        let mut idx = self.main_position(hash);

        // 沿着链表查找
        loop {
            let node = &self.nodes[idx];

            if &node.key == key {
                return Some(idx);
            }

            if node.next == NO_NEXT {
                return None;
            }

            // 使用绝对索引，避免溢出
            idx = node.next;
            if idx >= self.nodes.len() {
                return None;
            }
        }
    }

    /// 插入新键
    fn insert_new_key(&mut self, key: LuaValue, value: LuaValue, hash: u64) {
        let main_pos = self.main_position(hash);

        // 主位置为空
        if self.nodes[main_pos].is_empty() {
            self.nodes[main_pos] = Node {
                key,
                value,
                next: NO_NEXT,
            };
            self.count += 1;
            return;
        }

        // 需要处理冲突
        let free_pos = match self.get_free_pos() {
            Some(pos) => pos,
            None => {
                self.resize();
                return self.insert(key, value);
            }
        };

        // 新键放在空闲位置
        self.nodes[free_pos] = Node {
            key,
            value,
            next: NO_NEXT,
        };

        // 链接到主位置的链表尾部
        let mut idx = main_pos;
        loop {
            let next = self.nodes[idx].next;
            if next == NO_NEXT {
                self.nodes[idx].next = free_pos;
                break;
            }
            idx = next;
            if idx >= self.nodes.len() {
                break;
            }
        }
        
        self.count += 1;
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

    /// 插入或更新键值对
    #[inline]
    fn insert(&mut self, key: LuaValue, value: LuaValue) {
        if self.nodes.is_empty() {
            self.nodes = vec![Node::new_empty(); INITIAL_CAPACITY];
            self.last_free = INITIAL_CAPACITY;
        }

        let hash = self.hash_key(&key);
        let main_pos = self.main_position(hash);

        // 检查主位置
        if &self.nodes[main_pos].key == &key {
            self.nodes[main_pos].value = value;
            return;
        }

        // 沿链表查找
        let mut idx = main_pos;
        loop {
            let node = &self.nodes[idx];
            if &node.key == &key {
                self.nodes[idx].value = value;
                if let Some(k) = key.as_integer() {
                    self.update_array_len_insert(k);
                }
                return;
            }

            if node.next == NO_NEXT {
                break;
            }

            idx = node.next;
            if idx >= self.nodes.len() {
                break;
            }
        }

        // 插入新键
        self.insert_new_key(key.clone(), value, hash);

        if let Some(k) = key.as_integer() {
            self.update_array_len_insert(k);
        }
    }

    /// 删除键
    fn remove(&mut self, key: &LuaValue) -> bool {
        if let Some(idx) = self.find_node(key) {
            self.nodes[idx].value = LuaValue::nil();
            self.nodes[idx].key = DEAD_KEY;

            if let Some(k) = key.as_integer() {
                self.update_array_len_remove(k);
            }

            if self.count > 0 {
                self.count -= 1;
            }

            true
        } else {
            false
        }
    }

    /// 获取值
    #[inline(always)]
    fn get(&self, key: &LuaValue) -> Option<LuaValue> {
        if self.nodes.is_empty() {
            return None;
        }

        let hash = self.hash_key(key);
        let mut idx = self.main_position(hash);

        loop {
            let node = &self.nodes[idx];

            if &node.key == key {
                return Some(node.value);
            }

            if node.next == NO_NEXT {
                return None;
            }

            idx = node.next;
            if idx >= self.nodes.len() {
                return None;
            }
        }
    }
}

impl LuaTableImpl for LuaHashTable {
    #[inline(always)]
    fn get_int(&self, key: i64) -> Option<LuaValue> {
        if self.nodes.is_empty() {
            return None;
        }

        let key_value = LuaValue::integer(key);
        let hash = self.hash_key(&key_value);
        let mut idx = self.main_position(hash);

        loop {
            let node = &self.nodes[idx];

            if let Some(node_key) = node.key.as_integer() {
                if node_key == key {
                    return Some(node.value);
                }
            }

            if node.next == NO_NEXT {
                return None;
            }

            idx = node.next;
            if idx >= self.nodes.len() {
                return None;
            }
        }
    }

    #[inline(always)]
    fn set_int(&mut self, key: i64, value: LuaValue) -> LuaInsertResult {
        if self.nodes.is_empty() {
            self.grow_to_size(4);
        }

        let key_value = LuaValue::integer(key);
        let hash = self.hash_key(&key_value);
        let mut idx = self.main_position(hash);

        loop {
            let node = &self.nodes[idx];

            if let Some(node_key) = node.key.as_integer() {
                if node_key == key {
                    self.nodes[idx].value = value;
                    return LuaInsertResult::Success;
                }
            }

            if node.next == NO_NEXT {
                break;
            }

            idx = node.next;
            if idx >= self.nodes.len() {
                break;
            }
        }

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
        if input_key.is_nil() {
            for node in &self.nodes {
                if !node.key.is_nil() && !node.value.is_nil() {
                    return Some((node.key, node.value));
                }
            }
            return None;
        }

        if let Some(idx) = self.find_node(input_key) {
            for i in (idx + 1)..self.nodes.len() {
                let node = &self.nodes[i];
                if !node.key.is_nil() && !node.value.is_nil() {
                    return Some((node.key, node.value));
                }
            }
        }

        None
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
}
