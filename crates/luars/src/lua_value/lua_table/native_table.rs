// Native Lua 5.5-style table implementation
// Port of ltable.c with minimal abstractions for maximum performance

use crate::lua_value::{LuaValue, lua_value::{Value, LUA_VNIL}};
use std::alloc::{self, Layout};
use std::ptr;

/// Node for hash table - mimics Lua 5.5's Node structure
/// Key-Value pair + next pointer for collision chaining
#[repr(C)]
struct Node {
    /// Value stored in this node
    value: LuaValue,
    /// Key stored in this node  
    key: LuaValue,
    /// Next node in collision chain (offset, 0 = end)
    next: i32,
}

impl Node {
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.key.is_nil()
    }
}

/// Dummy node for empty hash tables
const DUMMY_NODE: Node = Node {
    value: LuaValue {
        value: Value { i: 0 },
        tt: LUA_VNIL,
    },
    key: LuaValue {
        value: Value { i: 0 },
        tt: LUA_VNIL,
    },
    next: 0,
};

/// Native Lua table implementation - mimics Lua 5.5's Table struct
pub struct NativeTable {
    /// Array part (for integer keys 1..asize)
    array: *mut LuaValue,
    /// Array size
    asize: u32,
    
    /// Hash part (Node array)
    node: *mut Node,
    /// log2 of hash size (size = 1 << lsizenode)
    lsizenode: u8,
    
    /// Cached array length for # operator
    array_len: u32,
}

impl NativeTable {
    /// Create new table with given capacity
    pub fn new(array_cap: u32, hash_cap: u32) -> Self {
        let mut table = Self {
            array: ptr::null_mut(),
            asize: 0,
            node: ptr::null_mut(),
            lsizenode: 0,
            array_len: 0,
        };
        
        // Allocate array part
        if array_cap > 0 {
            table.resize_array(array_cap);
        }
        
        // Allocate hash part
        if hash_cap > 0 {
            let lsize = Self::compute_lsizenode(hash_cap);
            table.resize_hash(lsize);
        }
        
        table
    }
    
    /// Compute log2(size) for hash part
    #[inline]
    fn compute_lsizenode(size: u32) -> u8 {
        if size == 0 {
            return 0;
        }
        let mut lsize = 0u8;
        let mut s = size - 1;
        while s > 0 {
            s >>= 1;
            lsize += 1;
        }
        lsize
    }
    
    /// Get hash size (number of nodes)
    #[inline(always)]
    fn sizenode(&self) -> usize {
        if self.node.is_null() || self.node == &DUMMY_NODE as *const Node as *mut Node {
            0
        } else {
            1usize << self.lsizenode
        }
    }
    
    #[inline(always)]
    fn is_dummy(&self) -> bool {
        self.node.is_null() || self.node == &DUMMY_NODE as *const Node as *mut Node
    }
    
    /// Resize array part
    fn resize_array(&mut self, new_size: u32) {
        if new_size == 0 {
            if !self.array.is_null() && self.asize > 0 {
                let layout = Layout::array::<LuaValue>(self.asize as usize).unwrap();
                unsafe { alloc::dealloc(self.array as *mut u8, layout) };
            }
            self.array = ptr::null_mut();
            self.asize = 0;
            self.array_len = 0;
            return;
        }
        
        let old_size = self.asize;
        let layout = Layout::array::<LuaValue>(new_size as usize).unwrap();
        
        let new_array = unsafe { alloc::alloc(layout) as *mut LuaValue };
        if new_array.is_null() {
            panic!("Failed to allocate array");
        }
        
        // Initialize new elements to nil
        unsafe {
            for i in 0..new_size {
                ptr::write(new_array.add(i as usize), LuaValue::nil());
            }
        }
        
        // Copy old data
        if !self.array.is_null() && old_size > 0 {
            let copy_size = old_size.min(new_size) as usize;
            unsafe {
                ptr::copy_nonoverlapping(self.array, new_array, copy_size);
            }
            
            let old_layout = Layout::array::<LuaValue>(old_size as usize).unwrap();
            unsafe { alloc::dealloc(self.array as *mut u8, old_layout) };
        }
        
        self.array = new_array;
        self.asize = new_size;
        
        // Update array_len if it's too large
        if self.array_len > new_size {
            self.array_len = new_size;
        }
    }
    
    /// Resize hash part
    fn resize_hash(&mut self, new_lsize: u8) {
        let old_size = self.sizenode();
        let new_size = if new_lsize == 0 { 0 } else { 1usize << new_lsize };
        
        let old_node = self.node;
        let was_dummy = self.is_dummy();
        
        if new_size == 0 {
            // Switch to dummy
            if !was_dummy && old_size > 0 {
                let layout = Layout::array::<Node>(old_size).unwrap();
                unsafe { alloc::dealloc(old_node as *mut u8, layout) };
            }
            self.node = ptr::null_mut();
            self.lsizenode = 0;
            return;
        }
        
        // Allocate new hash array
        let layout = Layout::array::<Node>(new_size).unwrap();
        let new_node = unsafe { alloc::alloc(layout) as *mut Node };
        if new_node.is_null() {
            panic!("Failed to allocate hash nodes");
        }
        
        // Initialize all nodes
        unsafe {
            for i in 0..new_size {
                let node = new_node.add(i);
                ptr::write(node, Node {
                    value: LuaValue::nil(),
                    key: LuaValue::nil(),
                    next: 0,
                });
            }
        }
        
        self.node = new_node;
        self.lsizenode = new_lsize;
        
        // Rehash old entries
        if !was_dummy && old_size > 0 {
            for i in 0..old_size {
                unsafe {
                    let old_n = old_node.add(i);
                    if !(*old_n).key.is_nil() {
                        let key = (*old_n).key;
                        let value = (*old_n).value;
                        self.set_node(key, value);
                    }
                }
            }
            
            let old_layout = Layout::array::<Node>(old_size).unwrap();
            unsafe { alloc::dealloc(old_node as *mut u8, old_layout) };
        }
    }
    
    /// Get main position for a key (hash index)
    #[inline(always)]
    fn mainposition(&self, key: &LuaValue) -> *mut Node {
        let size = self.sizenode();
        if size == 0 {
            return self.node;
        }
        
        let hash = key.hash_value();
        let index = (hash as usize) & (size - 1); // size is power of 2
        
        unsafe { self.node.add(index) }
    }
    
    /// Get value from array part
    #[inline(always)]
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        if key >= 1 && key <= self.asize as i64 {
            let index = (key - 1) as usize;
            let val = unsafe { *self.array.add(index) };
            if !val.is_nil() {
                return Some(val);
            }
        }
        
        // Try hash part
        let key_val = LuaValue::integer(key);
        self.get_from_hash(&key_val)
    }
    
    /// Set value in array part
    #[inline(always)]
    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        if key >= 1 && key <= self.asize as i64 {
            let index = (key - 1) as usize;
            unsafe {
                *self.array.add(index) = value;
            }
            
            // Update array_len
            if key == self.array_len as i64 + 1 && !value.is_nil() {
                self.array_len += 1;
            } else if key == self.array_len as i64 && value.is_nil() {
                self.array_len -= 1;
            }
            return;
        }
        
        // Put in hash part
        let key_val = LuaValue::integer(key);
        self.set_node(key_val, value);
    }
    
    /// Get value from hash part - CRITICAL HOT PATH
    #[inline(always)]
    fn get_from_hash(&self, key: &LuaValue) -> Option<LuaValue> {
        if self.sizenode() == 0 {
            return None;
        }
        
        // Fast path for short strings - direct pointer comparison
        if key.is_string() {
            return self.get_shortstr_fast(key);
        }
        
        // General case
        let mut node = self.mainposition(key);
        
        loop {
            unsafe {
                // Compare keys
                if (*node).key == *key {
                    let val = (*node).value;
                    return if val.is_nil() { None } else { Some(val) };
                }
                
                let next = (*node).next;
                if next == 0 {
                    return None;
                }
                node = node.offset(next as isize);
            }
        }
    }
    
    /// Fast path for short string lookup - mimics luaH_Hgetshortstr
    #[inline(always)]
    fn get_shortstr_fast(&self, key: &LuaValue) -> Option<LuaValue> {
        let mut node = self.mainposition(key);
        
        unsafe {
            loop {
                // Short strings: pointer comparison only (interned)
                if (*node).key.is_string() && (*node).key.value.i == key.value.i {
                    let val = (*node).value;
                    return if val.is_nil() { None } else { Some(val) };
                }
                
                let next = (*node).next;
                if next == 0 {
                    return None;
                }
                node = node.offset(next as isize);
            }
        }
    }
    
    /// Generic get
    #[inline(always)]
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        // Try array part for integers
        if let Some(i) = key.as_integer() {
            if i >= 1 && i <= self.asize as i64 {
                let index = (i - 1) as usize;
                let val = unsafe { *self.array.add(index) };
                if !val.is_nil() {
                    return Some(val);
                }
            }
        }
        
        // Hash part
        self.get_from_hash(key)
    }
    
    /// Set value in hash part
    fn set_node(&mut self, key: LuaValue, value: LuaValue) {
        if self.sizenode() == 0 {
            // Need to allocate hash part
            self.resize_hash(2); // Start with 4 nodes
        }
        
        let mp = self.mainposition(&key);
        
        unsafe {
            // If main position is free, use it
            if (*mp).key.is_nil() {
                (*mp).key = key;
                (*mp).value = value;
                (*mp).next = 0;
                return;
            }
            
            // Check if key already exists
            let mut node = mp;
            loop {
                if (*node).key == key {
                    (*node).value = value;
                    return;
                }
                
                let next = (*node).next;
                if next == 0 {
                    break;
                }
                node = node.offset(next as isize);
            }
            
            // Need to add new node - find free position
            let size = self.sizenode();
            for i in (0..size).rev() {
                let free_node = self.node.add(i);
                if (*free_node).key.is_nil() {
                    // Found free node
                    (*free_node).key = key;
                    (*free_node).value = value;
                    (*free_node).next = 0;
                    
                    // Link to chain
                    (*node).next = (free_node as isize - node as isize) as i32 / std::mem::size_of::<Node>() as i32;
                    return;
                }
            }
            
            // No free nodes - need to resize
            self.resize_hash(self.lsizenode + 1);
            self.set_node(key, value);
        }
    }
    
    /// Generic set
    #[inline(always)]
    pub fn raw_set(&mut self, key: &LuaValue, value: LuaValue) {
        // Try array part for integers
        if let Some(i) = key.as_integer() {
            if i >= 1 && i <= self.asize as i64 {
                self.set_int(i, value);
                return;
            }
        }
        
        // Hash part
        self.set_node(*key, value);
    }
    
    /// Get length (#t)
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.array_len as usize
    }
}

impl Drop for NativeTable {
    fn drop(&mut self) {
        // Free array
        if !self.array.is_null() && self.asize > 0 {
            let layout = Layout::array::<LuaValue>(self.asize as usize).unwrap();
            unsafe { alloc::dealloc(self.array as *mut u8, layout) };
        }
        
        // Free hash
        let size = self.sizenode();
        if size > 0 && !self.is_dummy() {
            let layout = Layout::array::<Node>(size).unwrap();
            unsafe { alloc::dealloc(self.node as *mut u8, layout) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_native_table_basic() {
        let mut t = NativeTable::new(4, 4);
        
        // Test integer keys
        let key1 = LuaValue::integer(1);
        let val1 = LuaValue::integer(100);
        t.raw_set(&key1, val1);
        
        assert_eq!(t.raw_get(&key1), Some(val1));
        
        // Test more integer keys
        for i in 1..=10 {
            t.set_int(i, LuaValue::integer(i * 10));
        }
        
        for i in 1..=10 {
            assert_eq!(t.get_int(i), Some(LuaValue::integer(i * 10)));
        }
    }
    
    #[test]
    fn test_array_part() {
        let mut t = NativeTable::new(10, 0);
        
        for i in 1..=10 {
            t.set_int(i, LuaValue::integer(i * 10));
        }
        
        for i in 1..=10 {
            assert_eq!(t.get_int(i), Some(LuaValue::integer(i * 10)));
        }
        
        assert_eq!(t.len(), 10);
    }
    
    #[test]
    fn test_hash_collisions() {
        let mut t = NativeTable::new(0, 4);
        
        // Add many items to force collisions
        for i in 0..20 {
            let key = LuaValue::integer(i);
            let val = LuaValue::integer(i * 100);
            t.raw_set(&key, val);
        }
        
        // Verify all items
        for i in 0..20 {
            let key = LuaValue::integer(i);
            let expected = LuaValue::integer(i * 100);
            assert_eq!(t.raw_get(&key), Some(expected), "Failed for key {}", i);
        }
    }
    
    #[test]
    fn test_performance_integer_keys() {
        use std::time::Instant;
        
        let mut t = NativeTable::new(100, 100);
        
        let start = Instant::now();
        
        // Insert
        for i in 0..10000 {
            t.set_int(i, LuaValue::integer(i));
        }
        
        // Read
        for i in 0..10000 {
            let val = t.get_int(i);
            assert_eq!(val, Some(LuaValue::integer(i)));
        }
        
        let elapsed = start.elapsed();
        println!("NativeTable integer ops (20k ops): {:?}", elapsed);
        println!("Per-op: {:?}", elapsed / 20000);
    }
}
