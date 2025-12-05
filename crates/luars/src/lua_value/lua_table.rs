// High-performance Lua table implementation following Lua 5.4 design
// - Array part for integer keys [1..n]
// - Hash part using open addressing (same as Lua 5.4)
use super::LuaValue;
use crate::{LuaVM, TableId};

/// Hash node - mimics Lua 5.4's Node structure
/// Contains key+value pair
#[derive(Clone, Copy)]
struct Node {
    key: LuaValue,
    value: LuaValue,
}

impl Node {
    #[inline(always)]
    fn empty() -> Self {
        Node {
            key: LuaValue::nil(),
            value: LuaValue::nil(),
        }
    }

    /// Check if node is empty - only need to check key (nil key = empty slot)
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.key.is_nil()
    }
}

/// Metamethod flags for fast lookup (like Lua 5.4's flags field)
/// A bit set to 1 means the metamethod is NOT present (absence cache)
/// Only the first 6 metamethods use this optimization (TM_INDEX..TM_EQ)
pub mod tm_flags {
    pub const TM_INDEX: u8 = 1 << 0; // __index
    pub const TM_NEWINDEX: u8 = 1 << 1; // __newindex
    pub const TM_GC: u8 = 1 << 2; // __gc
    pub const TM_MODE: u8 = 1 << 3; // __mode
    pub const TM_LEN: u8 = 1 << 4; // __len
    pub const TM_EQ: u8 = 1 << 5; // __eq
    pub const TM_CALL: u8 = 1 << 6; // __call (bonus: very common)
    pub const MASK_ALL: u8 = 0x7F; // All 7 bits
}

/// Lua table implementation
/// - Array part for integer keys [1..n]
/// - Hash part using open addressing with chaining (same as Lua 5.4)
pub struct LuaTable {
    /// Array part: stores values for integer keys [1..array.len()]
    /// Always initialized (empty Vec if no array elements)
    pub(crate) array: Vec<LuaValue>,

    /// Hash part: open-addressed hash table with linear probing
    /// This matches Lua 5.4's design for better iteration performance
    /// Always initialized (empty Vec if no hash elements)
    nodes: Vec<Node>,

    /// Number of occupied slots in hash part (O(1) load factor tracking)
    hash_size: usize,

    /// Metatable - optional table that defines special behaviors  
    /// Store as LuaValue (table ID) instead of Rc for ID-based architecture
    metatable: Option<TableId>,

    /// Metamethod absence flags (like Lua 5.4)
    /// A bit set to 1 means the metamethod is NOT present (cached absence)
    /// This allows O(1) check for common metamethods instead of hash lookup
    pub tm_flags: u8,
}

impl LuaTable {
    /// Create an empty table
    #[inline(always)]
    pub fn new(array_size: usize, hash_size: usize) -> Self {
        // FAST PATH: Most common case is empty table
        if array_size == 0 && hash_size == 0 {
            return LuaTable {
                array: Vec::new(),
                nodes: Vec::new(),
                hash_size: 0,
                metatable: None,
                tm_flags: 0, // All metamethods unknown initially
            };
        }

        // Hash size must be power of 2 for fast modulo using & (size-1)
        let actual_hash_size = if hash_size > 0 {
            hash_size.next_power_of_two()
        } else {
            0
        };

        LuaTable {
            array: if array_size > 0 {
                Vec::with_capacity(array_size)
            } else {
                Vec::new()
            },
            nodes: if actual_hash_size > 0 {
                vec![Node::empty(); actual_hash_size]
            } else {
                Vec::new()
            },
            hash_size: 0,
            metatable: None,
            tm_flags: 0, // All metamethods unknown initially
        }
    }

    /// Hash function for LuaValue - optimized for speed
    /// Uses identity hash for GC objects (string/table/function ID) and value hash for primitives
    #[inline(always)]
    fn hash_key(key: &LuaValue, size: usize) -> usize {
        // Combine primary and secondary for best distribution
        // For strings: primary has tag|id, secondary is 0 -> hash of id
        // For integers: primary has tag, secondary has value -> hash of value
        // For floats: similar to integers
        // XOR gives good mixing without branch
        let raw = key.primary.wrapping_add(key.secondary);

        // Fibonacci hashing - excellent distribution for sequential IDs
        // Golden ratio: 2^64 / phi ≈ 0x9e3779b97f4a7c15
        let hash = raw.wrapping_mul(0x9e3779b97f4a7c15);

        // Fast modulo using bitmask (size is power of 2)
        (hash >> 32) as usize & (size - 1)
    }

    /// Find a node with the given key, returns Some(index) if found
    #[inline(always)]
    fn find_node(&self, key: &LuaValue) -> Option<usize> {
        let size = self.nodes.len();
        if size == 0 {
            return None;
        }

        let mut idx = Self::hash_key(key, size);
        let start_idx = idx;

        // Fast path: check first slot (no collision case - most common)
        let node = unsafe { self.nodes.get_unchecked(idx) };
        if node.is_empty() {
            return None;
        }
        if node.key == *key {
            return Some(idx);
        }

        // Collision path: linear probe
        loop {
            idx = (idx + 1) & (size - 1);

            if idx == start_idx {
                return None;
            }

            let node = unsafe { self.nodes.get_unchecked(idx) };
            if node.is_empty() {
                return None;
            }
            if node.key == *key {
                return Some(idx);
            }
        }
    }

    /// Resize hash part to new size (power of 2)
    fn resize_hash(&mut self, new_size: usize) {
        if new_size == 0 {
            self.nodes = Vec::new();
            self.hash_size = 0;
            return;
        }

        // Must be power of 2 for fast modulo
        debug_assert!(new_size.is_power_of_two());

        let old_nodes = std::mem::replace(&mut self.nodes, vec![Node::empty(); new_size]);
        self.hash_size = 0; // Reset counter, will be rebuilt during rehash

        // Rehash all existing nodes
        for old_node in old_nodes {
            if !old_node.is_empty() {
                self.insert_node_simple(old_node.key, old_node.value);
            }
        }
    }

    /// Simple insert using linear probing (no complex chaining)
    #[inline]
    fn insert_node_simple(&mut self, key: LuaValue, value: LuaValue) {
        let size = self.nodes.len();
        debug_assert!(size > 0, "insert_node_simple called with empty nodes");

        let mut idx = Self::hash_key(&key, size);

        // Fast path: first slot is empty (common case)
        let node = unsafe { self.nodes.get_unchecked(idx) };
        if node.is_empty() {
            self.nodes[idx] = Node { key, value };
            self.hash_size += 1;
            return;
        }
        if node.key == key {
            self.nodes[idx].value = value;
            return;
        }

        // Collision path
        let start_idx = idx;
        loop {
            idx = (idx + 1) & (size - 1);

            if idx == start_idx {
                // Table is full - should not happen if resize is correct
                panic!(
                    "Hash table is full during insert: size={}, hash_size={}, key={:?}",
                    size, self.hash_size, key
                );
            }

            let node = unsafe { self.nodes.get_unchecked(idx) };
            if node.is_empty() {
                self.nodes[idx] = Node { key, value };
                self.hash_size += 1;
                return;
            }
            if node.key == key {
                self.nodes[idx].value = value;
                return;
            }
        }
    }

    /// Insert a key-value pair into hash part
    fn insert_node(&mut self, key: LuaValue, value: LuaValue) {
        if self.nodes.is_empty() {
            // Initialize hash table with small size
            self.resize_hash(8);
        }

        // Check load factor using O(1) counter - resize if > 75%
        let nodes_len = self.nodes.len();
        if nodes_len > 0 && self.hash_size * 4 >= nodes_len * 3 {
            // Double the size, minimum 8
            let new_size = nodes_len * 2;
            self.resize_hash(new_size);
        }

        self.insert_node_simple(key, value);
    }

    /// Get the metatable of this table
    pub fn get_metatable(&self) -> Option<LuaValue> {
        self.metatable
            .map(|mt_id| LuaValue::table(mt_id))
    }

    /// Set the metatable of this table
    /// Resets tm_flags since the new metatable may have different metamethods
    pub fn set_metatable(&mut self, mt: Option<LuaValue>) {
        self.metatable = mt.and_then(|v| v.as_table_id());
        // Reset all flags - metamethods need to be re-checked
        self.tm_flags = 0;
    }

    /// Fast metamethod absence check (like Lua 5.4's fasttm macro)
    /// Returns true if the metamethod is known to be absent (flag is set)
    /// This is O(1) vs O(n) hash lookup
    #[inline(always)]
    pub fn tm_absent(&self, flag: u8) -> bool {
        (self.tm_flags & flag) != 0
    }

    /// Mark a metamethod as absent (cache the lookup result)
    /// Called after a failed lookup to speed up future checks
    #[inline(always)]
    pub fn set_tm_absent(&mut self, flag: u8) {
        self.tm_flags |= flag;
    }

    /// Clear a specific tm flag (called when metamethod is set)
    #[inline(always)]
    pub fn clear_tm_absent(&mut self, flag: u8) {
        self.tm_flags &= !flag;
    }

    /// Fast integer key access - O(1) for array part
    /// Ultra-optimized hot path for ipairs iterations
    /// Note: This only checks the array part for performance.
    /// Use get_int_full() if the value might be in the hash part.
    #[inline(always)]
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        if key > 0 {
            let idx = (key - 1) as usize;
            if idx < self.array.len() {
                // SAFETY: bounds check is done explicitly
                unsafe {
                    let val = self.array.get_unchecked(idx);
                    if !val.is_nil() {
                        return Some(*val);
                    }
                }
            }
        }
        None
    }

    /// Integer key access that also checks hash part
    /// Used by GETI when array lookup fails
    #[inline]
    pub fn get_int_full(&self, key: i64) -> Option<LuaValue> {
        // First try array part (fast path)
        if let Some(val) = self.get_int(key) {
            return Some(val);
        }
        // Fall back to hash part
        self.get_from_hash(&LuaValue::integer(key))
    }

    /// Optimized string key access using &str - avoids LuaValue allocation
    /// This is a hot path for table access with string literals
    #[inline(always)]
    pub fn get_str(&self, vm: &mut LuaVM, key_str: &str) -> Option<LuaValue> {
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
    #[inline(always)]
    pub(crate) fn get_from_hash(&self, key: &LuaValue) -> Option<LuaValue> {
        match self.find_node(key) {
            Some(idx) => Some(unsafe { self.nodes.get_unchecked(idx).value }),
            None => None,
        }
    }

    /// Fast integer key write
    /// OPTIMIZED: Use resize_with for better performance
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

            if idx < array_len {
                // Fast path: within existing array
                self.array[idx] = value;
                return;
            } else if idx == array_len {
                // Sequential append - use reserve to reduce reallocations
                if self.array.capacity() == array_len {
                    // Need to grow - reserve extra space
                    let extra = if array_len == 0 { 8 } else { array_len };
                    self.array.reserve(extra);
                }
                self.array.push(value);
                return;
            } else if idx < array_len + 8 && idx < 256 {
                // Small gap - fill with nils and extend
                self.array.resize_with(idx + 1, LuaValue::nil);
                self.array[idx] = value;
                return;
            }
        }

        // Out of array range or large gap, use hash
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

    /// Set in hash part - Lua-style open addressing
    fn set_in_hash(&mut self, key: LuaValue, value: LuaValue) {
        if value.is_nil() {
            // Setting to nil - remove the key
            if let Some(idx) = self.find_node(&key) {
                self.nodes[idx] = Node::empty();
                self.hash_size -= 1;
            }
        } else {
            // Insert or update
            self.insert_node(key, value);
        }
    }

    /// Get array length
    #[inline]
    pub fn len(&self) -> usize {
        self.array.len()
    }

    /// Iterator for next() function - follows Lua's iteration order
    /// First iterates array part, then hash part (linear scan for cache efficiency!)
    pub fn next(&self, key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        if key.is_nil() {
            // Start from beginning - find first non-nil in array
            for (i, val) in self.array.iter().enumerate() {
                if !val.is_nil() {
                    return Some((LuaValue::integer((i + 1) as i64), *val));
                }
            }
            // Then first non-empty node in hash
            for node in &self.nodes {
                if !node.is_empty() {
                    return Some((node.key, node.value));
                }
            }
            return None;
        }

        // Continue from given key
        if let Some(i) = key.as_integer() {
            if i > 0 {
                let idx = i as usize;
                // Look for next non-nil in array
                for j in idx..self.array.len() {
                    if !self.array[j].is_nil() {
                        return Some((LuaValue::integer((j + 1) as i64), self.array[j]));
                    }
                }
                // End of array, move to hash - return first non-empty node
                for node in &self.nodes {
                    if !node.is_empty() {
                        return Some((node.key, node.value));
                    }
                }
                return None;
            }
        }

        // Key is in hash part - find it quickly and return next non-empty node
        if let Some(current_idx) = self.find_node(key) {
            // Found current key - scan forward from next position
            for idx in (current_idx + 1)..self.nodes.len() {
                if !self.nodes[idx].is_empty() {
                    return Some((self.nodes[idx].key, self.nodes[idx].value));
                }
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

        // CRITICAL OPTIMIZATION: Fast path for appending at end (no shift needed!)
        if pos == len {
            self.array.push(value);
            return Ok(());
        }

        // OPTIMIZATION: Use Vec::insert which uses memmove internally
        self.array.insert(pos, value);
        Ok(())
    }

    /// Remove value at position in array part, shifting elements to the left
    /// Position is 0-indexed internally but Lua uses 1-indexed
    pub fn remove_array_at(&mut self, pos: usize) -> Result<LuaValue, String> {
        let len = self.len();
        if pos >= len {
            return Err("remove position out of bounds".to_string());
        }

        let removed = self.array[pos];

        // CRITICAL OPTIMIZATION: Fast path for removing from end (no shift needed!)
        if pos == len - 1 {
            self.array.pop();
            return Ok(removed);
        }

        // OPTIMIZATION: Use copy_within for bulk memory move
        self.array.copy_within(pos + 1..len, pos);
        self.array.pop();

        Ok(removed)
    }

    /// Iterator for GC - returns all key-value pairs
    pub fn iter_all(&self) -> Vec<(LuaValue, LuaValue)> {
        let mut result = Vec::new();

        // Iterate array part
        for (i, val) in self.array.iter().enumerate() {
            if !val.is_nil() {
                result.push((LuaValue::integer((i + 1) as i64), *val));
            }
        }

        // Iterate hash part - linear scan!
        for node in &self.nodes {
            if !node.is_empty() {
                result.push((node.key, node.value));
            }
        }

        result
    }
}
