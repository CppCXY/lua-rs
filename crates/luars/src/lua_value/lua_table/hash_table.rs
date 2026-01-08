use crate::{
    LuaResult, LuaValue,
    lua_value::{LuaTableImpl, lua_table::LuaInsertResult},
};

/// 高性能Lua哈希表实现 - Lua 5.5风格链式哈希
///
/// 关键优化：
/// 1. 链式哈希（Chained Scatter with Brent's Variation）
/// 2. 简化哈希函数（整数直接用值，无需Fibonacci）
/// 3. 直接存储节点（无额外indirection）
/// 4. lastfree快速分配
///
/// Lua 5.5不变式：
/// - 如果元素不在主位置，碰撞元素必在自己的主位置
/// - 即使100%负载因子性能依然良好
pub struct LuaHashTable {
    /// 节点数组：直接存储键值对 + 链表指针
    nodes: Vec<Node>,
    
    /// 下一个空闲位置（从后向前搜索）
    last_free: usize,
    
    /// 数组部分长度记录（#操作符）
    array_len: usize,
}

/// 哈希节点 - 模拟Lua 5.5的Node结构
#[derive(Clone)]
struct Node {
    key: LuaValue,
    value: LuaValue,
    /// 链表指针：相对偏移（0 = 链尾）
    /// 负数 = 向前，正数 = 向后
    next: i32,
}

const INITIAL_CAPACITY: usize = 4;
const DEAD_KEY: LuaValue = LuaValue::nil();  // 死键标记

impl Node {
    #[inline(always)]
    fn new_empty() -> Self {
        Self {
            key: DEAD_KEY,
            value: LuaValue::nil(),
            next: 0,
        }
    }
    
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.key.is_nil() && self.next == 0
    }
    
    #[inline(always)]
    fn is_main_position(&self) -> bool {
        self.next == 0 || self.key.is_nil()
    }
}

impl LuaHashTable {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(INITIAL_CAPACITY);
        // 使用奇数容量以获得更好的哈希分布
        let capacity = if capacity.is_power_of_two() {
            capacity + 1
        } else {
            capacity
        };
        
        let nodes = vec![Node::new_empty(); capacity];
        let last_free = capacity;
        
        Self {
            nodes,
            last_free,
            array_len: 0,
        }
    }

    /// 哈希函数 - 简化版，模拟Lua 5.5
    /// 整数直接使用值，其他类型简单组合
    #[inline(always)]
    fn hash_key(key: &LuaValue) -> u64 {
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
                        // Integer: 直接使用值（简单且快速）
                        key.value.i as u64
                    }
                }
                _ => {
                    // GC类型：id混合type
                    let id = key.gcid() as u64;
                    let tt = (key.tt() as u64) << 32;
                    id ^ tt
                }
            }
        }
    }

    /// 主位置：hash % size（使用奇数取模）
    #[inline(always)]
    fn main_position(&self, hash: u64) -> usize {
        let size = self.nodes.len();
        // Lua 5.5风格：对奇数取模获得更好分布
        (hash as usize) % size
    }
    
    /// 从node获取其主位置（用于冲突检测）
    #[inline]
    fn get_main_position(&self, node_idx: usize) -> usize {
        let node = &self.nodes[node_idx];
        if node.key.is_nil() {
            node_idx
        } else {
            let hash = Self::hash_key(&node.key);
            self.main_position(hash)
        }
    }
    
    /// 查找空闲节点（从last_free向前搜索）
    fn get_free_pos(&mut self) -> Option<usize> {
        while self.last_free > 0 {
            self.last_free -= 1;
            if self.nodes[self.last_free].is_empty() {
                return Some(self.last_free);
            }
        }
        None  // 表满，需要扩容
    }
    
    /// 查找键 - Lua 5.5风格链式查找
    #[inline(always)]
    fn find_node(&self, key: &LuaValue) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }
        
        let hash = Self::hash_key(key);
        let mut idx = self.main_position(hash);
        
        // 沿着链表查找
        loop {
            let node = unsafe { self.nodes.get_unchecked(idx) };
            
            if &node.key == key {
                return Some(idx);
            }
            
            if node.next == 0 {
                return None;  // 链尾，未找到
            }
            
            // 跟随链表（相对偏移）
            idx = (idx as i32 + node.next) as usize;
        }
    }
    
    /// 插入新键 - Lua 5.5的Brent's variation
    /// 核心不变式：如果元素不在主位置，碰撞元素必在自己的主位置
    fn insert_new_key(&mut self, key: LuaValue, value: LuaValue, hash: u64) {
        let main_pos = self.main_position(hash);
        
        // 情况1：主位置为空
        if self.nodes[main_pos].is_empty() {
            self.nodes[main_pos] = Node {
                key,
                value,
                next: 0,
            };
            return;
        }
        
        // 情况2：需要处理冲突
        // 获取空闲位置
        let free_pos = match self.get_free_pos() {
            Some(pos) => pos,
            None => {
                // 表满，扩容后重试
                self.resize();
                return self.insert(key, value);
            }
        };
        
        // 检查主位置的元素是否在其主位置
        let other_main_pos = self.get_main_position(main_pos);
        
        if other_main_pos == main_pos {
            // 主位置元素在正确位置，新键链接到链表末尾
            self.nodes[free_pos] = Node {
                key,
                value,
                next: 0,
            };
            
            // 找到链表末尾并连接
            let mut idx = main_pos;
            loop {
                let next = self.nodes[idx].next;
                if next == 0 {
                    // 计算相对偏移
                    self.nodes[idx].next = (free_pos as i32) - (idx as i32);
                    break;
                }
                idx = (idx as i32 + next) as usize;
            }
        } else {
            // Brent's variation: 主位置元素不在其主位置
            // 将主位置元素移到free_pos，新键占据主位置
            
            // 移动旧元素
            self.nodes[free_pos] = self.nodes[main_pos].clone();
            
            // 更新指向旧元素的链表
            let mut idx = other_main_pos;
            loop {
                let next = self.nodes[idx].next;
                let next_idx = (idx as i32 + next) as usize;
                if next_idx == main_pos {
                    // 更新指针指向新位置
                    self.nodes[idx].next = (free_pos as i32) - (idx as i32);
                    break;
                }
                idx = next_idx;
            }
            
            // 新键占据主位置
            self.nodes[main_pos] = Node {
                key,
                value,
                next: 0,
            };
        }
    }

    /// 更新数组长度 (#操作符)
    fn update_array_len_insert(&mut self, key: i64) {
        if key == (self.array_len as i64 + 1) {
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
    
    /// 扩容 - 重建整个表
    fn resize(&mut self) {
        let new_size = (self.nodes.len() * 2).max(INITIAL_CAPACITY);
        // 使用奇数
        let new_size = if new_size.is_power_of_two() {
            new_size + 1
        } else {
            new_size
        };
        
        self.grow_to_size(new_size);
    }
    
    /// 扩容到指定大小
    fn grow_to_size(&mut self, new_size: usize) {
        let new_size = if new_size.is_power_of_two() {
            new_size + 1  // 使用奇数
        } else {
            new_size
        };
        
        let old_nodes = std::mem::replace(&mut self.nodes, vec![Node::new_empty(); new_size]);
        self.last_free = new_size;
        
        // 重新插入所有元素
        for node in old_nodes {
            if !node.key.is_nil() {
                let hash = Self::hash_key(&node.key);
                self.insert_new_key(node.key, node.value, hash);
            }
        }
    }

    /// 插入或更新
    #[inline]
    fn insert(&mut self, key: LuaValue, value: LuaValue) {
        if self.nodes.is_empty() {
            self.nodes = vec![Node::new_empty(); INITIAL_CAPACITY + 1];
            self.last_free = INITIAL_CAPACITY + 1;
        }
        
        // 查找是否已存在
        let hash = Self::hash_key(&key);
        let main_pos = self.main_position(hash);
        
        // 先检查主位置
        if &self.nodes[main_pos].key == &key {
            self.nodes[main_pos].value = value;
            return;
        }
        
        // 沿链表查找
        let mut idx = main_pos;
        loop {
            let node = &self.nodes[idx];
            if &node.key == &key {
                // 找到，更新值
                self.nodes[idx].value = value;
                
                // 更新数组长度
                if let Some(k) = key.as_integer() {
                    self.update_array_len_insert(k);
                }
                return;
            }
            
            if node.next == 0 {
                break;  // 未找到，需要插入新键
            }
            
            idx = (idx as i32 + node.next) as usize;
        }
        
        // 插入新键
        self.insert_new_key(key, value, hash);
        
        // 更新数组长度
        if let Some(k) = value.as_integer() {
            self.update_array_len_insert(k);
        }
    }

    /// 删除键
    fn remove(&mut self, key: &LuaValue) -> bool {
        if let Some(idx) = self.find_node(key) {
            // 将值设为nil（保留键结构以维持链表）
            self.nodes[idx].value = LuaValue::nil();
            self.nodes[idx].key = DEAD_KEY;
            
            // 更新数组长度
            if let Some(k) = key.as_integer() {
                self.update_array_len_remove(k);
            }
            
            true
        } else {
            false
        }
    }

    /// 获取值 - 内联的快速路径
    #[inline(always)]
    fn get(&self, key: &LuaValue) -> Option<LuaValue> {
        if self.nodes.is_empty() {
            return None;
        }
        
        let hash = Self::hash_key(key);
        let mut idx = self.main_position(hash);
        
        // 手动内联链表遍历以获得最佳性能
        loop {
            let node = unsafe { self.nodes.get_unchecked(idx) };
            
            if &node.key == key {
                return Some(node.value);
            }
            
            if node.next == 0 {
                return None;
            }
            
            idx = (idx as i32 + node.next) as usize;
        }
    }
}

impl LuaTableImpl for LuaHashTable {
    #[inline(always)]
    fn get_int(&self, key: i64) -> Option<LuaValue> {
        // 快速路径：直接内联整数查找，避免创建临时 LuaValue
        if self.nodes.is_empty() {
            return None;
        }
        
        // 整数键：直接使用值作为哈希
        let hash = key as u64;
        let mut idx = (hash as usize) % self.nodes.len();
        
        loop {
            let node = unsafe { self.nodes.get_unchecked(idx) };
            
            // 快速整数比较
            if let Some(node_key) = node.key.as_integer() {
                if node_key == key {
                    return Some(node.value);
                }
            }
            
            if node.next == 0 {
                return None;
            }
            
            idx = (idx as i32 + node.next) as usize;
        }
    }

    #[inline(always)]
    fn set_int(&mut self, key: i64, value: LuaValue) -> LuaInsertResult {
        // 快速路径：内联整数插入
        if self.nodes.is_empty() {
            self.grow_to_size(4);  // 初始大小
        }
        
        // 整数键：直接使用值作为哈希
        let hash = key as u64;
        let mut idx = (hash as usize) % self.nodes.len();
        
        // 查找是否已存在
        loop {
            let node = &self.nodes[idx];
            
            if let Some(node_key) = node.key.as_integer() {
                if node_key == key {
                    // 更新现有值
                    self.nodes[idx].value = value;
                    return LuaInsertResult::Success;
                }
            }
            
            if node.next == 0 {
                break;  // 需要插入新键
            }
            
            idx = (idx as i32 + node.next) as usize;
        }
        
        // 插入新键（走通用路径）
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
            // 返回第一个非空节点
            for node in &self.nodes {
                if !node.key.is_nil() && !node.value.is_nil() {
                    return Some((node.key, node.value));
                }
            }
            return None;
        }

        // 查找当前键，返回下一个
        if let Some(idx) = self.find_node(input_key) {
            // 从当前位置向后查找下一个有效节点
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
        // 返回数组部分长度 (Lua # operator)
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

        assert!(count >= 3);  // 至少找到3个元素
    }

    #[test]
    fn test_grow() {
        let mut table = LuaHashTable::new(4);

        // 插入大量元素，触发扩容
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
    
    #[test]
    fn test_chain_collision() {
        let mut table = LuaHashTable::new(5);  // 小容量，强制冲突
        
        // 插入会冲突的键
        for i in 0..20 {
            table.set_int(i, LuaValue::integer(i * 100));
        }
        
        // 验证链式哈希正确处理冲突
        for i in 0..20 {
            assert_eq!(table.get_int(i), Some(LuaValue::integer(i * 100)));
        }
    }
}
