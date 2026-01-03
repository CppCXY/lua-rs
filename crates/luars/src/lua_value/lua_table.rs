// LuaTable - Rust优化的Lua Table实现
//
// **设计理念**: 在保持Lua语义的前提下，利用Rust的优势进行优化
//
// **与Lua C实现的差异**:
// 1. Array存储: Vec<LuaValue>代替分离的Value*+tag* - 更简单，内存局部性好
// 2. Hash存储: Box<[Node]>代替Vec<Node> - 节省capacity字段（8字节）
// 3. 移除lastfree: Rust Vec操作已很快，无需额外优化
// 4. 移除lenhint: 简化实现，按需计算长度
//
// **内存布局对比** (64位系统):
// Lua C Table: ~48-56字节 (使用指针)
// Rust优化前: 112字节 (3个Vec + 未使用字段)
// Rust优化后: ~56字节 (Vec + Box + 基础字段)

use super::lua_value::{
    LUA_TDEADKEY, LUA_TNIL, LUA_VNUMINT, LUA_VSHRSTR, LuaValue, LuaValueKind, Value, ctb, novariant,
};
use crate::{TableId, gc::ObjectPool};

// ============ Constants ============

/// BITDUMMY flag: table没有hash part (使用dummy node)
const BITDUMMY: u8 = 1 << 6;

/// LOG_2查找表: log_2[i-1] = ceil(log2(i))
/// 用于快速计算ceil(log2(x))
const LOG_2: [u8; 256] = [
    0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
];

// ============ Node (Hash table节点) ============

/// Hash table节点
///
/// 保持与Lua C一致的结构，但使用struct而非union (Rust安全)
#[derive(Clone, Copy)]
pub struct Node {
    /// 节点的value
    pub value: LuaValue,

    /// Key的类型tag
    pub key_tt: u8,

    /// 冲突链表的next索引 (-1表示链结束)
    pub next: i32,

    /// Key的值
    pub key_val: Value,
}

impl Node {
    /// 创建空节点 (使用DEADKEY标记)
    #[inline(always)]
    pub fn empty() -> Self {
        Self {
            value: LuaValue::empty(),
            key_tt: LUA_TDEADKEY,
            next: -1,
            key_val: Value { i: 0 },
        }
    }

    /// 检查节点是否为空
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        novariant(self.key_tt) == LUA_TNIL
    }

    /// 检查key是否为dead
    #[inline(always)]
    pub fn is_dead(&self) -> bool {
        self.key_tt == LUA_TDEADKEY
    }

    /// 获取key (构造LuaValue)
    #[inline(always)]
    pub fn key(&self) -> LuaValue {
        LuaValue {
            value_: self.key_val,
            tt_: self.key_tt,
        }
    }

    /// 设置key
    #[inline(always)]
    pub fn set_key(&mut self, key: &LuaValue) {
        self.key_val = key.value_;
        self.key_tt = key.tt_;
    }

    /// 检查key是否为integer
    #[inline(always)]
    pub fn key_is_integer(&self) -> bool {
        self.key_tt == LUA_VNUMINT
    }

    /// 获取integer key的值
    #[inline(always)]
    pub fn key_integer(&self) -> i64 {
        debug_assert!(self.key_is_integer());
        unsafe { self.key_val.i }
    }

    /// 检查key是否为short string
    #[inline(always)]
    pub fn key_is_shrstr(&self) -> bool {
        self.key_tt == ctb(LUA_VSHRSTR)
    }

    /// 获取string key的ID
    #[inline(always)]
    pub fn key_string_id(&self) -> u32 {
        debug_assert!(self.key_is_shrstr());
        unsafe { self.key_val.gc_id }
    }
}

// ============ LuaTable ============

/// Rust极致优化的Lua Table
///
/// **内存布局** (64位):
/// - meta: 8 bytes (压缩: flags[8bits] + lsizenode[8bits] + metatable_id[48bits])
/// - array: 24 bytes (Vec: ptr+len+cap)
/// - nodes: 16 bytes (Box<[T]>: ptr+len)
/// - object_pool: 8 bytes (指向ObjectPool的裸指针)
/// **总计: 56字节!**
///
/// **meta字段位布局**:
/// - bits 0-7:   flags (metamethod缓存)
/// - bits 8-15:  lsizenode (hash大小的log2)
/// - bits 16-63: metatable_id (0表示无metatable，1-based)
pub struct LuaTable {
    /// 压缩的元数据字段
    /// Layout: flags(8) | lsizenode(8) | metatable_id(48)
    /// metatable_id为0表示没有metatable
    /// metatable_id为n+1表示TableId(n) (1-based以避免0冲突)
    meta: u64,

    /// Array part: 存储正整数key (1..array.len())
    /// 移除了asize字段 - array.len()就是大小
    array: Vec<LuaValue>,

    /// Hash part: 存储非数组key
    pub nodes: Box<[Node]>,

    object_pool: *const ObjectPool, // 用于hash计算和key比较
}

impl LuaTable {
    /// 创建新table
    pub fn new(asize: u32, hsize: u32, object_pool: *const ObjectPool) -> Self {
        // 计算hash part大小
        let lsizenode = if hsize == 0 {
            0
        } else {
            Self::ceillog2(hsize) as u8
        };

        let actual_hsize = if lsizenode == 0 { 0 } else { 1u32 << lsizenode };

        // 初始化array part
        let array = vec![LuaValue::empty(); asize as usize];

        // 初始化hash part
        let nodes = if actual_hsize == 0 {
            Box::new([])
        } else {
            vec![Node::empty(); actual_hsize as usize].into_boxed_slice()
        };

        // 构建meta字段
        let flags = if actual_hsize == 0 { BITDUMMY } else { 0 };
        let meta = Self::pack_meta(flags, lsizenode, None);

        Self {
            meta,
            array,
            nodes,
            object_pool,
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

    /// 获取flags
    #[inline(always)]
    fn flags(&self) -> u8 {
        (self.meta & 0xFF) as u8
    }

    /// 设置flags
    #[inline(always)]
    fn set_flags(&mut self, flags: u8) {
        self.meta = (self.meta & !0xFF) | (flags as u64);
    }

    /// 获取lsizenode
    #[inline(always)]
    fn lsizenode(&self) -> u8 {
        ((self.meta >> 8) & 0xFF) as u8
    }

    /// 设置lsizenode
    #[inline(always)]
    fn set_lsizenode(&mut self, lsizenode: u8) {
        self.meta = (self.meta & !(0xFFu64 << 8)) | ((lsizenode as u64) << 8);
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

    /// 计算ceil(log2(x)) - 来自Lua的luaO_ceillog2
    /// 返回最小的整数n使得 x <= 2^n
    fn ceillog2(mut x: u32) -> u32 {
        if x == 0 {
            return 0;
        }

        let mut l = 0;
        x -= 1;
        while x >= 256 {
            l += 8;
            x >>= 8;
        }
        l + LOG_2[x as usize] as u32
    }

    // ============ Hash functions ============

    /// 计算key的hash位置 - mainposition
    fn mainposition(&self, key: &LuaValue) -> usize {
        let sizenode = self.sizenode();
        if sizenode == 0 {
            return 0;
        }

        // Lua的hashmod: hash % ((sizenode-1)|1)
        // |1 确保除数是奇数，避免偶数hash值产生冲突
        let modulo = (sizenode - 1) | 1;

        match key.kind() {
            LuaValueKind::Integer => {
                // hashint: 对integer取模
                // 使用整数的位表示作为hash值（与Lua ltable.c的hashint一致）
                let i = unsafe { key.value_.i };
                (i as usize) % modulo
            }
            LuaValueKind::Float => {
                // hashfloat: 对float取模，使用位表示作为hash
                let n = unsafe { key.value_.n };
                let bits = n.to_bits();
                (bits as usize) % modulo
            }
            LuaValueKind::String => {
                // 使用LuaString的hash
                let string_id = key.as_string_id().unwrap();
                if let Some(s) = unsafe { &*self.object_pool }.get_string(string_id) {
                    (s.hash as usize) % modulo
                } else {
                    // 字符串不存在，使用ID作为hash
                    (string_id.raw() as usize) % modulo
                }
            }
            LuaValueKind::Boolean => {
                let b = unsafe { key.value_.i }; // 布尔值存储在i字段
                (b as usize) % modulo
            }
            LuaValueKind::Table => {
                let id = unsafe { key.value_.gc_id };
                (id as usize) % modulo
            }
            _ => {
                let hash = unsafe { key.value_.i };
                ((hash as u64) as usize) % modulo
            }
        }
    }

    /// 在hash part中查找key
    fn getnode<'a>(&'a self, key: &LuaValue) -> Option<&'a Node> {
        if self.is_dummy() {
            return None;
        }

        let mp = self.mainposition(key);
        let mut node = &self.nodes[mp];

        let object_pool = unsafe { &*self.object_pool };
        // 沿着collision chain查找
        loop {
            if !node.is_dead() {
                let node_key = node.key();
                if node_key.raw_equal(key, object_pool) {
                    return Some(node);
                }
            }

            if node.next < 0 {
                break;
            }
            node = &self.nodes[node.next as usize];
        }

        None
    }

    /// 在hash part中查找key (可变版本)
    fn getnode_mut<'a>(&'a mut self, key: &LuaValue) -> Option<&'a mut Node> {
        if self.is_dummy() {
            return None;
        }

        let mp = self.mainposition(key);
        let mut idx = mp;
        let object_pool = unsafe { &*self.object_pool };
        loop {
            let node_key = self.nodes[idx].key();
            // SAFETY: pool指针来自caller，在调用期间保持有效
            if !self.nodes[idx].is_dead() && node_key.raw_equal(key, object_pool) {
                return Some(&mut self.nodes[idx]);
            }

            let next = self.nodes[idx].next;
            if next < 0 {
                break;
            }
            idx = next as usize;
        }

        None
    }

    /// 查找第一个空闲节点
    fn get_free_pos(&self) -> Option<usize> {
        if self.is_dummy() {
            return None;
        }

        // 从后向前查找空闲节点
        for i in (0..self.nodes.len()).rev() {
            if self.nodes[i].is_dead() {
                return Some(i);
            }
        }
        None
    }

    /// 调整hash part大小
    fn resize_hash(&mut self, new_size: usize) {
        if new_size == 0 {
            // 设置为dummy
            self.nodes = Box::new([]);
            self.set_lsizenode(0);
            self.set_flags(self.flags() | BITDUMMY);
            return;
        }

        // 保存旧节点
        let old_nodes = std::mem::replace(&mut self.nodes, Box::new([]));

        // 创建新的hash part
        let lsizenode = Self::ceillog2(new_size as u32) as u8;
        let actual_size = 1usize << lsizenode;
        self.nodes = vec![Node::empty(); actual_size].into_boxed_slice();
        self.set_lsizenode(lsizenode);
        self.set_flags(self.flags() & !BITDUMMY);

        // 重新插入所有旧节点
        for old_node in old_nodes.iter() {
            if !old_node.is_dead() && !old_node.is_empty() {
                let key = old_node.key();
                let value = old_node.value;
                self.set_hash_value(&key, value);
            }
        }
    }

    // ============ Public API ============

    /// 获取hash part大小
    #[inline(always)]
    pub fn sizenode(&self) -> usize {
        if self.is_dummy() {
            0
        } else {
            1usize << self.lsizenode()
        }
    }

    /// 检查是否使用dummy node
    #[inline(always)]
    pub fn is_dummy(&self) -> bool {
        (self.flags() & BITDUMMY) != 0
    }

    /// 获取array大小
    #[inline(always)]
    pub fn asize(&self) -> usize {
        self.array.len()
    }

    /// Get value by integer key
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        // Try array part (Lua uses 1-based indexing)
        let asize = self.asize();
        if key > 0 && (key as usize) <= asize {
            let idx = (key - 1) as usize;
            let val = self.array[idx];
            if !val.is_nil() {
                return Some(val);
            }
        }
        // Search hash part
        let key_val = LuaValue::integer(key);
        self.getnode(&key_val).map(|node| node.value)
    }

    /// Set value by integer key
    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        // Try array part
        let asize = self.asize();
        if key > 0 && (key as usize) <= asize {
            let idx = (key - 1) as usize;
            self.array[idx] = value;
            return;
        }

        // Lua语义：如果key == asize + 1，追加到数组末尾
        if key > 0 && key as usize == asize + 1 {
            self.array.push(value);
            return;
        }

        // 其他情况：Set in hash part
        let key_val = LuaValue::integer(key);
        self.set_hash_value(&key_val, value);
    }

    /// Get value by key
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        // Try integer key in array part
        if let Some(i) = key.as_integer() {
            return self.get_int(i);
        }
        // Search hash part for other key types
        self.getnode(key).map(|node| node.value)
    }

    /// Get value by key (strict version for kcache - no numeric type conversion)
    /// Unlike raw_get, this does NOT treat integer 0 and float 0.0 as the same key
    pub fn raw_get_strict(&self, key: &LuaValue) -> Option<LuaValue> {
        // For kcache, we need strict type matching
        // Integer keys go to array part ONLY if they are actually integers
        if key.ttisinteger() {
            if let Some(i) = key.as_integer_strict() {
                return self.get_int(i);
            }
        }
        // Search hash part for all other keys (including floats)
        self.getnode(key).map(|node| node.value)
    }

    /// Set value by key
    pub fn raw_set(&mut self, key: LuaValue, value: LuaValue) {
        // Try integer key in array part
        if let Some(i) = key.as_integer() {
            self.set_int(i, value);
            return;
        }
        // Set in hash part for other key types
        self.set_hash_value(&key, value);
    }

    /// Set value by key (strict version for kcache - no numeric type conversion)
    /// Unlike raw_set, this does NOT treat integer 0 and float 0.0 as the same key
    pub fn raw_set_strict(&mut self, key: LuaValue, value: LuaValue) {
        // For kcache, we need strict type matching
        // Integer keys go to array part ONLY if they are actually integers
        if key.ttisinteger() {
            if let Some(i) = key.as_integer_strict() {
                self.set_int(i, value);
                return;
            }
        }
        // Set in hash part for all other keys (including floats)
        self.set_hash_value(&key, value);
    }

    /// Set value in hash part
    fn set_hash_value(&mut self, key: &LuaValue, value: LuaValue) {
        if self.is_dummy() {
            // 扩展hash part：分配初始大小为4的hash表
            self.resize_hash(4);
        }

        // 查找已存在的key
        if let Some(node) = self.getnode_mut(key) {
            node.value = value;
            return;
        }

        // Key不存在，需要插入新key
        // 查找空闲位置
        let free_pos = match self.get_free_pos() {
            Some(pos) => pos,
            None => {
                // 需要rehash：扩大一倍
                let new_size = self.sizenode() * 2;
                self.resize_hash(new_size);
                // 重新设置值
                self.set_hash_value(key, value);
                return;
            }
        };

        // 获取mainposition
        let mp = self.mainposition(key);
        let main_node_key = self.nodes[mp].key();
        let pool = unsafe { &*self.object_pool };
        if self.nodes[mp].is_dead() {
            // mainposition是空的，直接插入
            self.nodes[mp].set_key(key);
            self.nodes[mp].value = value;
            self.nodes[mp].next = -1;
        } else if main_node_key.raw_equal(key, pool) {
            // Key已存在（理论上不应该到这里，因为上面已经处理了）
            self.nodes[mp].value = value;
        } else {
            // Collision: mainposition被占用
            let other_mp = self.mainposition(&main_node_key);

            if other_mp == mp {
                // 当前节点在正确位置，新key插入free_pos，链到mp
                self.nodes[free_pos].set_key(key);
                self.nodes[free_pos].value = value;
                self.nodes[free_pos].next = self.nodes[mp].next;
                self.nodes[mp].next = free_pos as i32;
            } else {
                // 当前节点不在正确位置，移动它到free_pos
                // 先找到指向mp的节点
                let mut prev_idx = other_mp;
                while self.nodes[prev_idx].next as usize != mp {
                    prev_idx = self.nodes[prev_idx].next as usize;
                }

                // 将mp的内容移到free_pos
                self.nodes[free_pos] = self.nodes[mp];
                // 更新链表
                self.nodes[prev_idx].next = free_pos as i32;
                // 在mp位置插入新key
                self.nodes[mp].set_key(key);
                self.nodes[mp].value = value;
                self.nodes[mp].next = -1;
            }
        }
    }

    /// Get array length
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.array.len()
    }

    /// Get metatable
    pub fn get_metatable(&self) -> Option<LuaValue> {
        self.metatable().map(LuaValue::table)
    }

    /// Set metatable
    pub fn set_metatable(&mut self, metatable: Option<LuaValue>) {
        let mt = metatable.and_then(|v| v.as_table_id());
        self.set_metatable_internal(mt);
    }

    /// Insert at array index (Lua 1-based indexing)
    /// 在指定位置插入元素，后面的元素向后移动
    pub fn insert_array_at(&mut self, idx: usize, value: LuaValue) -> Result<(), String> {
        if idx == 0 {
            return Err("index must be >= 1 (Lua uses 1-based indexing)".to_string());
        }

        let asize = self.asize() as usize;

        if idx > asize + 1 {
            return Err(format!(
                "index {} out of bounds (array size is {})",
                idx, asize
            ));
        }

        if idx == asize + 1 {
            // 追加到末尾
            self.array.push(value);
        } else {
            // 插入到中间，Vec的insert使用0-based索引
            self.array.insert(idx - 1, value);
        }

        Ok(())
    }

    /// Metamethod absence check
    #[inline(always)]
    pub fn tm_absent(&self, flag: u8) -> bool {
        (self.flags() & flag) != 0
    }

    /// Set metamethod absent flag
    #[inline(always)]
    pub fn set_tm_absent(&mut self, flag: u8) {
        let flags = self.flags() | flag;
        self.set_flags(flags);
    }

    /// Clear metamethod absent flag
    #[inline(always)]
    pub fn clear_tm_absent(&mut self, flag: u8) {
        let flags = self.flags() & !flag;
        self.set_flags(flags);
    }

    /// Iterate all key-value pairs (for GC)
    /// 包含array part和hash part的所有元素
    pub fn iter_all(&self) -> impl Iterator<Item = (LuaValue, LuaValue)> + '_ {
        // Array part: 索引从1开始
        let array_iter = self.array.iter().enumerate().filter_map(|(idx, val)| {
            if !val.is_nil() {
                Some((LuaValue::integer((idx + 1) as i64), *val))
            } else {
                None
            }
        });

        // Hash part
        let hash_iter = self.nodes.iter().filter_map(|node| {
            if !node.is_empty() && !node.is_dead() {
                Some((node.key(), node.value))
            } else {
                None
            }
        });

        array_iter.chain(hash_iter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_table() {
        let object_pool = ObjectPool::new();
        let t = LuaTable::new(0, 0, &object_pool as *const _);
        assert_eq!(t.asize(), 0);
        assert_eq!(t.sizenode(), 0);
        assert!(t.is_dummy());
    }

    #[test]
    fn test_table_with_array() {
        let object_pool = ObjectPool::new();
        let t = LuaTable::new(10, 0, &object_pool as *const _);
        assert_eq!(t.asize(), 10);
        assert_eq!(t.len(), 10);
        assert!(t.is_dummy());
    }

    #[test]
    fn test_table_with_hash() {
        let object_pool = ObjectPool::new();
        let t = LuaTable::new(0, 8, &object_pool as *const _);
        assert_eq!(t.asize(), 0);
        assert_eq!(t.sizenode(), 8);
        assert!(!t.is_dummy());
        assert_eq!(t.lsizenode(), 3);
    }

    #[test]
    fn test_ceillog2() {
        assert_eq!(LuaTable::ceillog2(0), 0);
        assert_eq!(LuaTable::ceillog2(1), 0);
        assert_eq!(LuaTable::ceillog2(2), 1);
        assert_eq!(LuaTable::ceillog2(3), 2);
        assert_eq!(LuaTable::ceillog2(4), 2);
        assert_eq!(LuaTable::ceillog2(5), 3);
        assert_eq!(LuaTable::ceillog2(8), 3);
        assert_eq!(LuaTable::ceillog2(9), 4);
    }

    #[test]
    fn test_meta_packing() {
        let object_pool = ObjectPool::new();
        let mut t = LuaTable::new(0, 8, &object_pool as *const _);

        // 测试flags
        assert_eq!(t.flags(), 0);
        t.set_flags(0x42);
        assert_eq!(t.flags(), 0x42);

        // 测试lsizenode
        assert_eq!(t.lsizenode(), 3);
        t.set_lsizenode(5);
        assert_eq!(t.lsizenode(), 5);
        assert_eq!(t.flags(), 0x42); // flags应该不变

        // 测试metatable
        assert_eq!(t.metatable(), None);
        t.set_metatable_internal(Some(TableId(999)));
        assert_eq!(t.metatable(), Some(TableId(999)));
        assert_eq!(t.flags(), 0x42); // flags应该不变
        assert_eq!(t.lsizenode(), 5); // lsizenode应该不变

        // 测试清除metatable
        t.set_metatable_internal(None);
        assert_eq!(t.metatable(), None);
    }

    #[test]
    fn test_simple_hash_insert() {
        let mut pool = ObjectPool::new();
        let tid = pool.create_table(0, 0); // 从dummy开始

        let (key, _) = pool.create_string("test");
        let key_value = LuaValue::string(key);

        if let Some(table) = pool.get_table_mut(tid) {
            println!(
                "Before insert: is_dummy={}, sizenode={}",
                table.is_dummy(),
                table.sizenode()
            );
            table.raw_set(key_value, LuaValue::integer(42));
            println!(
                "After insert: is_dummy={}, sizenode={}",
                table.is_dummy(),
                table.sizenode()
            );

            let result = table.raw_get(&key_value).unwrap();

            assert!(
                result.raw_equal(&LuaValue::integer(42), &pool),
                "Failed to get after set"
            );
        }
    }

    #[test]
    fn test_string_key_equality() {
        let mut pool = ObjectPool::new();
        let tid = pool.create_table(0, 8);

        // 测试短字符串（会被intern）
        let str1 = "test_string";
        let (key1, _) = pool.create_string(str1);
        let (key2, _) = pool.create_string(str1);

        // 短字符串会被intern，所以ID应该相同
        assert_eq!(key1, key2, "Short strings should be interned");
        assert!(key1.is_short(), "Should be marked as short string");

        // 测试长字符串（不会被intern，但内容相同）
        let long_str1 = "a".repeat(100);
        let long_str2 = "a".repeat(100);

        let (long_key1, _) = pool.create_string(&long_str1);
        let (long_key2, _) = pool.create_string(&long_str2);

        assert_ne!(
            long_key1, long_key2,
            "Long strings should have different IDs"
        );
        assert!(long_key1.is_long(), "Should be marked as long string");
        assert!(long_key2.is_long(), "Should be marked as long string");

        let long_val1 = LuaValue::string(long_key1);
        let long_val2 = LuaValue::string(long_key2);

        // 验证LuaValue正确识别为长字符串
        assert!(!long_val1.ttisshrstring(), "Should not be short string");
        assert!(!long_val2.ttisshrstring(), "Should not be short string");

        // 用内容相同但ID不同的long_key2查找，应该也能找到（通过raw_equal比较内容）
        if let Some(table) = pool.get_table(tid) {
            let result = table.raw_get(&long_val2).unwrap();
            assert!(
                result.raw_equal(&LuaValue::integer(42), &pool),
                "Should find with content-equal key"
            );
        }
    }
}
