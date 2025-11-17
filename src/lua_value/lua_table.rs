// High-performance Lua table implementation following Lua 5.4 design
// - Array part for integer keys [1..n]
// - Hash part using open addressing with separate chaining
// - No insertion_order vector needed - natural iteration order

use crate::LuaVM;

use super::LuaValue;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Hash node for the hash part of the table
/// Uses open addressing with chaining via next pointer
#[derive(Clone)]
struct Node {
    key: LuaValue,
    value: LuaValue,
    /// Index of next node in chain, or -1 if end of chain
    next: i32,
}

impl Node {
    #[inline]
    fn is_empty(&self) -> bool {
        self.key.is_nil()
    }
}

/// Lua table implementation following Lua 5.4 design
/// - Array part for integer keys [1..n]
/// - Hash part using open addressing with separate chaining
/// - Power-of-2 sized hash table for fast modulo
pub struct LuaTable {
    /// Array part: stores values for integer keys [1..array.len()]
    /// Only allocated when first integer key is set
    array: Vec<LuaValue>,

    /// Hash part: open-addressed hash table with chaining
    /// Size is always a power of 2 (or 0)
    /// Only allocated when first non-array key is set
    node: Vec<Node>,

    /// Last free position in hash table (used for allocation)
    last_free: i32,

    /// Metatable - optional table that defines special behaviors  
    /// Store as LuaValue (table ID) instead of Rc for ID-based architecture
    metatable: Option<LuaValue>,
}

impl LuaTable {
    /// Create an empty table
    pub fn new() -> Self {
        LuaTable {
            array: Vec::new(),
            node: Vec::new(),
            last_free: -1,
            metatable: None,
        }
    }

    /// Get the metatable of this table
    pub fn get_metatable(&self) -> Option<LuaValue> {
        self.metatable.clone()
    }

    /// Set the metatable of this table
    pub fn set_metatable(&mut self, mt: Option<LuaValue>) {
        self.metatable = mt;
    }

    /// Fast integer key access - O(1) for array part
    #[inline(always)]
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        if key > 0 {
            let idx = (key - 1) as usize;
            if idx < self.array.len() {
                let val = &self.array[idx];
                if !val.is_nil() {
                    return Some(val.clone());
                }
            }
        }
        None
    }

    /// Optimized string key access using &str - avoids LuaValue allocation
    /// This is a hot path for table access with string literals
    #[inline(always)]
    pub fn get_str(&self, vm: &mut LuaVM, key_str: &str) -> Option<LuaValue> {
        if self.node.is_empty() {
            return None;
        }

        let key = vm.create_string(key_str);
        self.get_from_hash(&key)
    }

    /// Generic key access
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        // Try array part first for integer keys
        if let Some(i) = key.as_integer() {
            if let Some(val) = self.get_int(i) {
                return Some(val);
            }
        }
        // Fall back to hash part
        self.get_from_hash(key)
    }

    /// Get from hash part
    #[inline]
    fn get_from_hash(&self, key: &LuaValue) -> Option<LuaValue> {
        if self.node.is_empty() {
            return None;
        }

        let hash = self.hash_key(key);
        let mask = self.node.len() - 1;
        let mut idx = (hash & mask) as usize;

        loop {
            let node = &self.node[idx];
            if node.is_empty() {
                return None;
            }
            if node.key == *key {
                return Some(node.value.clone());
            }
            if node.next < 0 {
                return None;
            }
            idx = node.next as usize;
        }
    }

    /// Fast integer key write
    #[inline]
    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        if value.is_nil() {
            // Setting to nil - just mark as nil in array
            if key > 0 {
                let idx = (key - 1) as usize;
                if idx < self.array.len() {
                    self.array[idx] = LuaValue::nil();
                }
            }
            return;
        }

        if key > 0 {
            let idx = (key - 1) as usize;
            let array_len = self.array.len();
            // Limit array growth to reasonable size (64K elements)
            if idx < array_len {
                self.array[idx] = value;
                return;
            } else if idx == array_len {
                self.array.push(value);
                return;
            }
        }

        // Out of array range, use hash
        self.set_in_hash(LuaValue::integer(key), value);
    }

    /// Generic key write
    pub fn raw_set(&mut self, key: LuaValue, value: LuaValue) {
        // Try array part for small positive integers (up to 64K)
        if let Some(i) = key.as_integer() {
            self.set_int(i, value);
            return;
        }
        self.set_in_hash(key, value);
    }

    /// Set in hash part - with automatic rehash on growth
    fn set_in_hash(&mut self, key: LuaValue, value: LuaValue) {
        if value.is_nil() {
            // Setting to nil - find and remove
            self.remove_from_hash(&key);
            return;
        }

        // Initialize hash table if empty
        if self.node.is_empty() {
            self.resize_hash(4); // Start with small size
        }

        // Try to insert
        if !self.insert_into_hash(key.clone(), value.clone()) {
            // Table is full, need to resize
            let new_size = (self.node.len() * 2).max(4);
            self.resize_hash(new_size);
            // Retry insertion (must succeed now)
            self.insert_into_hash(key, value);
        }
    }

    /// Insert into hash table, returns false if table is full
    fn insert_into_hash(&mut self, key: LuaValue, value: LuaValue) -> bool {
        let hash = self.hash_key(&key);
        let mask = self.node.len() - 1;
        let main_pos = (hash & mask) as usize;

        // Check if key already exists
        let mut idx = main_pos;
        loop {
            let node = &self.node[idx];
            if node.is_empty() {
                break;
            }
            if node.key == key {
                // Update existing key
                self.node[idx].value = value;
                return true;
            }
            if node.next < 0 {
                break;
            }
            idx = node.next as usize;
        }

        // Key doesn't exist, need to insert new node
        if self.node[main_pos].is_empty() {
            // Main position is free, use it
            self.node[main_pos] = Node {
                key,
                value,
                next: -1,
            };
            return true;
        }

        // Main position occupied, need to handle collision
        // Clone colliding node's key before finding free position
        let colliding_key = self.node[main_pos].key.clone();

        // Find free position
        let free_pos = match self.find_free_pos() {
            Some(pos) => pos,
            None => return false, // Table is full
        };

        // Check if colliding node is in its main position
        let colliding_hash = self.hash_key(&colliding_key);
        let colliding_main = (colliding_hash & mask) as usize;

        if colliding_main == main_pos {
            // Colliding node is in correct position, chain new node
            self.node[free_pos] = Node {
                key,
                value,
                next: -1,
            };
            // Add to end of chain
            let mut idx = main_pos;
            while self.node[idx].next >= 0 {
                idx = self.node[idx].next as usize;
            }
            self.node[idx].next = free_pos as i32;
        } else {
            // Colliding node is not in main position, move it
            self.node[free_pos] = self.node[main_pos].clone();

            // Update chain pointing to colliding node
            let mut idx = colliding_main;
            while self.node[idx].next != main_pos as i32 {
                idx = self.node[idx].next as usize;
            }
            self.node[idx].next = free_pos as i32;

            // Put new node in main position
            self.node[main_pos] = Node {
                key,
                value,
                next: -1,
            };
        }

        true
    }

    /// Find a free position in hash table
    fn find_free_pos(&mut self) -> Option<usize> {
        while self.last_free >= 0 {
            let pos = self.last_free as usize;
            self.last_free -= 1;
            if self.node[pos].is_empty() {
                return Some(pos);
            }
        }
        None
    }

    /// Remove key from hash part
    fn remove_from_hash(&mut self, key: &LuaValue) {
        if self.node.is_empty() {
            return;
        }

        let hash = self.hash_key(key);
        let mask = self.node.len() - 1;
        let main_pos = (hash & mask) as usize;

        // Find the node and its predecessor
        let mut prev_idx: Option<usize> = None;
        let mut idx = main_pos;

        loop {
            let node = &self.node[idx];
            if node.is_empty() {
                return; // Key not found
            }
            if node.key == *key {
                // Found the key
                if let Some(prev) = prev_idx {
                    // Remove from middle of chain
                    self.node[prev].next = node.next;
                } else if node.next >= 0 {
                    // Remove from head of chain, move next node here
                    let next_idx = node.next as usize;
                    self.node[idx] = self.node[next_idx].clone();
                    self.node[next_idx] = Node {
                        key: LuaValue::nil(),
                        value: LuaValue::nil(),
                        next: -1,
                    };
                    return;
                }
                // Clear the node
                self.node[idx] = Node {
                    key: LuaValue::nil(),
                    value: LuaValue::nil(),
                    next: -1,
                };
                return;
            }
            if node.next < 0 {
                return; // End of chain, key not found
            }
            prev_idx = Some(idx);
            idx = node.next as usize;
        }
    }

    /// Resize hash table
    fn resize_hash(&mut self, new_size: usize) {
        let old_nodes = std::mem::take(&mut self.node);

        // Create new table
        self.node = Vec::with_capacity(new_size);
        for _ in 0..new_size {
            self.node.push(Node {
                key: LuaValue::nil(),
                value: LuaValue::nil(),
                next: -1,
            });
        }
        self.last_free = new_size as i32 - 1;

        // Rehash all old entries
        for node in old_nodes {
            if !node.is_empty() {
                self.insert_into_hash(node.key, node.value);
            }
        }
    }

    /// Hash a key
    #[inline]
    fn hash_key(&self, key: &LuaValue) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish() as usize
    }

    /// Get array length (number of consecutive non-nil elements from index 1)
    pub fn len(&self) -> usize {
        let mut len = 0;
        for val in &self.array {
            if val.is_nil() {
                break;
            }
            len += 1;
        }
        len
    }

    /// Iterator for next() function - follows Lua's iteration order
    /// First iterates array part, then hash part
    pub fn next(&self, key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        if key.is_nil() {
            // Start from beginning - find first non-nil in array
            for (i, val) in self.array.iter().enumerate() {
                if !val.is_nil() {
                    return Some((LuaValue::integer((i + 1) as i64), val.clone()));
                }
            }
            // Then first entry in hash
            return self.next_hash_entry(None);
        }

        // Continue from given key
        if let Some(i) = key.as_integer() {
            if i > 0 {
                let idx = i as usize;
                // Look for next non-nil in array
                for j in idx..self.array.len() {
                    if !self.array[j].is_nil() {
                        return Some((LuaValue::integer((j + 1) as i64), self.array[j].clone()));
                    }
                }
                // End of array, move to hash
                return self.next_hash_entry(None);
            }
        }

        // Key is in hash part, find next entry
        self.next_hash_entry(Some(key))
    }

    /// Get next entry in hash part after given key (or first if None)
    fn next_hash_entry(&self, after_key: Option<&LuaValue>) -> Option<(LuaValue, LuaValue)> {
        if self.node.is_empty() {
            return None;
        }

        let start_idx = if let Some(key) = after_key {
            // Find current key's position, then continue from next
            let hash = self.hash_key(key);
            let mask = self.node.len() - 1;
            let mut idx = (hash & mask) as usize;

            loop {
                if self.node[idx].key == *key {
                    // Found it, start searching from next position
                    break idx + 1;
                }
                if self.node[idx].next < 0 {
                    // Key not found, shouldn't happen but handle gracefully
                    return None;
                }
                idx = self.node[idx].next as usize;
            }
        } else {
            0
        };

        // Find next non-empty node
        for i in start_idx..self.node.len() {
            let node = &self.node[i];
            if !node.is_empty() {
                return Some((node.key.clone(), node.value.clone()));
            }
        }

        None
    }

    /// Insert value at position in array part, shifting elements to the right
    /// Position is 0-indexed internally but Lua uses 1-indexed
    pub fn insert_array_at(&mut self, pos: usize, value: LuaValue) -> Result<(), String> {
        let len = self.len();
        if pos > len {
            return Err("insert position out of bounds".to_string());
        }

        // Ensure array is large enough
        if self.array.len() <= len {
            self.array.resize(len + 1, LuaValue::nil());
        }

        // Shift elements to the right
        for i in (pos..len).rev() {
            self.array[i + 1] = self.array[i].clone();
        }

        // Insert new value
        self.array[pos] = value;
        Ok(())
    }

    /// Remove value at position in array part, shifting elements to the left
    /// Position is 0-indexed internally but Lua uses 1-indexed
    pub fn remove_array_at(&mut self, pos: usize) -> Result<LuaValue, String> {
        let len = self.len();
        if pos >= len {
            return Err("remove position out of bounds".to_string());
        }

        let removed = self.array[pos].clone();

        // Shift elements to the left
        for i in pos..len - 1 {
            self.array[i] = self.array[i + 1].clone();
        }

        // Clear the last element
        if len > 0 {
            self.array[len - 1] = LuaValue::nil();
        }

        Ok(removed)
    }

    /// Iterator for GC - returns all key-value pairs
    pub fn iter_all(&self) -> Vec<(LuaValue, LuaValue)> {
        let mut result = Vec::new();

        // Iterate array part
        for (i, val) in self.array.iter().enumerate() {
            if !val.is_nil() {
                result.push((LuaValue::integer((i + 1) as i64), val.clone()));
            }
        }

        // Iterate hash part
        for node in &self.node {
            if !node.is_empty() {
                result.push((node.key.clone(), node.value.clone()));
            }
        }

        result
    }
}
