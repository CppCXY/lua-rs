// High-performance Lua table implementation following Lua 5.4 design
// - Array part for integer keys [1..n]
// - Hash part using open addressing (same as Lua 5.4)
use super::LuaValue;
use crate::LuaVM;

/// Hash node - mimics Lua 5.4's Node structure
/// Contains key+value pair and next index for collision chaining
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

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.key.is_nil() && self.value.is_nil()
    }
}

/// Lua table implementation
/// - Array part for integer keys [1..n]
/// - Hash part using open addressing with chaining (same as Lua 5.4)
pub struct LuaTable {
    /// Array part: stores values for integer keys [1..array.len()]
    /// Only allocated when first integer key is set
    pub(crate) array: Option<Vec<LuaValue>>,

    /// Hash part: open-addressed hash table with linear probing
    /// This matches Lua 5.4's design for better iteration performance
    /// - Faster iteration: linear scan vs HashMap's complex structure
    /// - Better cache locality: contiguous memory
    /// - Same lookup performance: O(1) average case
    nodes: Option<Vec<Node>>,

    /// Number of occupied slots in hash part (O(1) load factor tracking)
    hash_size: usize,

    /// Metatable - optional table that defines special behaviors  
    /// Store as LuaValue (table ID) instead of Rc for ID-based architecture
    metatable: Option<LuaValue>,
}

impl LuaTable {
    /// Create an empty table
    pub fn new(array_size: usize, hash_size: usize) -> Self {
        // Hash size must be power of 2 for fast modulo using & (size-1)
        let actual_hash_size = if hash_size > 0 {
            hash_size.next_power_of_two()
        } else {
            0
        };

        LuaTable {
            array: if array_size > 0 {
                Some(Vec::with_capacity(array_size))
            } else {
                None
            },
            nodes: if actual_hash_size > 0 {
                // Create vector with actual elements, not just capacity
                Some(vec![Node::empty(); actual_hash_size])
            } else {
                None
            },
            hash_size: 0,
            metatable: None,
        }
    }

    /// Hash function for LuaValue - matches Lua's hashing strategy
    #[inline]
    fn hash_key(key: &LuaValue, size: usize) -> usize {
        if size == 0 {
            return 0;
        }

        // Use Lua's approach: extract a numeric hash from the value
        let hash = if let Some(i) = key.as_integer() {
            i as u64
        } else if let Some(f) = key.as_float() {
            f.to_bits()
        } else {
            // For other types, use the primary value as hash
            key.primary
        };

        // Simple modulo - Lua uses power-of-2 sizes for fast masking
        (hash as usize) & (size - 1)
    }

    /// Find a node with the given key, returns Some(index) if found
    #[inline]
    fn find_node(&self, key: &LuaValue) -> Option<usize> {
        if let Some(nodes) = &self.nodes {
            let size = nodes.len();
            let mut idx = Self::hash_key(key, size);

            // Linear probe with wraparound
            let start_idx = idx;
            loop {
                let node = &nodes[idx];
                if node.is_empty() {
                    return None;
                }

                if node.key == *key {
                    return Some(idx);
                }

                // Linear probing
                idx = (idx + 1) & (size - 1);

                // Avoid infinite loop
                if idx == start_idx {
                    return None;
                }
            }
        }
        None
    }

    /// Resize hash part to new size (power of 2)
    fn resize_hash(&mut self, new_size: usize) {
        if new_size == 0 {
            self.nodes = None;
            self.hash_size = 0;
            return;
        }

        // Must be power of 2 for fast modulo
        debug_assert!(new_size.is_power_of_two());

        let old_nodes = std::mem::replace(&mut self.nodes, Some(vec![Node::empty(); new_size]));
        self.hash_size = 0; // Reset counter, will be rebuilt during rehash

        // Rehash all existing nodes
        if let Some(old_nodes) = old_nodes {
            for old_node in old_nodes {
                if !old_node.is_empty() {
                    self.insert_node_simple(old_node.key, old_node.value);
                }
            }
        }
    }

    /// Simple insert using linear probing (no complex chaining)
    fn insert_node_simple(&mut self, key: LuaValue, value: LuaValue) {
        // nodes must be initialized before calling this function
        // This is ensured by insert_node() which calls resize_hash() if needed
        if self.nodes.is_none() {
            panic!("insert_node_simple called with uninitialized nodes - this is a bug");
        }

        let nodes = self.nodes.as_mut().unwrap();
        let size = nodes.len();
        let mut idx = Self::hash_key(&key, size);
        let start_idx = idx;

        loop {
            if nodes[idx].is_empty() {
                // New insertion
                nodes[idx] = Node { key, value };
                self.hash_size += 1;
                return;
            } else if nodes[idx].key == key {
                // Update existing
                nodes[idx].value = value;
                return;
            }

            idx = (idx + 1) & (size - 1);

            if idx == start_idx {
                // Table is full - should not happen if resize is correct
                panic!(
                    "Hash table is full during insert: size={}, hash_size={}, key={:?}",
                    size, self.hash_size, key
                );
            }
        }
    }

    /// Insert a key-value pair into hash part
    fn insert_node(&mut self, key: LuaValue, value: LuaValue) {
        if self.nodes.is_none() {
            // Initialize hash table with small size
            self.resize_hash(8);
        }

        // Check load factor using O(1) counter - resize if > 75%
        // CRITICAL: Check BEFORE insert to avoid counting the new element
        let nodes_len = self.nodes.as_ref().map_or(0, |n| n.len());
        if nodes_len > 0 && self.hash_size * 4 >= nodes_len * 3 {
            // Double the size, minimum 8
            let new_size = if nodes_len == 0 { 8 } else { nodes_len * 2 };
            self.resize_hash(new_size);
        }

        self.insert_node_simple(key, value);
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
    /// Ultra-optimized hot path for ipairs iterations
    #[inline(always)]
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        if key > 0 {
            if let Some(array) = &self.array {
                let idx = (key - 1) as usize;
                // SAFETY: bounds check is done explicitly
                if idx < array.len() {
                    unsafe {
                        let val = array.get_unchecked(idx);
                        // LuaValue is Copy, so this is just a memcpy
                        if !val.is_nil() {
                            return Some(*val);
                        }
                    }
                }
            }
        }
        None
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
    #[inline]
    pub(crate) fn get_from_hash(&self, key: &LuaValue) -> Option<LuaValue> {
        if let Some(nodes) = &self.nodes {
            self.find_node(key).map(|idx| nodes[idx].value)
        } else {
            None
        }
    }

    /// Fast integer key write
    #[inline]
    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        if value.is_nil() {
            // Setting to nil - just mark as nil in array
            if key > 0 {
                if let Some(array) = &mut self.array {
                    let idx = (key - 1) as usize;
                    if idx < array.len() {
                        array[idx] = LuaValue::nil();
                    }
                }
            }
            return;
        }

        if key > 0 {
            let idx = (key - 1) as usize;

            // Initialize array if needed
            if self.array.is_none() {
                self.array = Some(Vec::new());
            }

            let array = self.array.as_mut().unwrap();
            let array_len = array.len();

            // Limit array growth to reasonable size (64K elements)
            if idx < array_len {
                array[idx] = value;
                return;
            } else if idx == array_len {
                array.push(value);
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

    /// Set in hash part - Lua-style open addressing
    fn set_in_hash(&mut self, key: LuaValue, value: LuaValue) {
        if value.is_nil() {
            // Setting to nil - remove the key
            if let Some(idx) = self.find_node(&key) {
                if let Some(nodes) = &mut self.nodes {
                    nodes[idx] = Node::empty();
                    self.hash_size -= 1;
                }
            }
        } else {
            // Insert or update
            self.insert_node(key, value);
        }
    }

    /// Get array length
    #[inline]
    pub fn len(&self) -> usize {
        self.array.as_ref().map_or(0, |a| a.len())
    }

    /// Iterator for next() function - follows Lua's iteration order
    /// First iterates array part, then hash part (linear scan for cache efficiency!)
    pub fn next(&self, key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        if key.is_nil() {
            // Start from beginning - find first non-nil in array
            if let Some(array) = &self.array {
                for (i, val) in array.iter().enumerate() {
                    if !val.is_nil() {
                        return Some((LuaValue::integer((i + 1) as i64), *val));
                    }
                }
            }
            // Then first non-empty node in hash
            if let Some(nodes) = &self.nodes {
                for node in nodes {
                    if !node.is_empty() {
                        return Some((node.key, node.value));
                    }
                }
            }
            return None;
        }

        // Continue from given key
        if let Some(i) = key.as_integer() {
            if i > 0 {
                let idx = i as usize;
                // Look for next non-nil in array
                if let Some(array) = &self.array {
                    for j in idx..array.len() {
                        if !array[j].is_nil() {
                            return Some((LuaValue::integer((j + 1) as i64), array[j]));
                        }
                    }
                }
                // End of array, move to hash - return first non-empty node
                if let Some(nodes) = &self.nodes {
                    for node in nodes {
                        if !node.is_empty() {
                            return Some((node.key, node.value));
                        }
                    }
                }
                return None;
            }
        }

        // Key is in hash part - find it quickly and return next non-empty node
        // OPTIMIZATION: Use find_node to locate current position, then scan forward
        if let Some(nodes) = &self.nodes {
            if let Some(current_idx) = self.find_node(key) {
                // Found current key - scan forward from next position
                for idx in (current_idx + 1)..nodes.len() {
                    if !nodes[idx].is_empty() {
                        return Some((nodes[idx].key, nodes[idx].value));
                    }
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

        // Initialize array if needed
        if self.array.is_none() {
            self.array = Some(Vec::new());
        }

        let array = self.array.as_mut().unwrap();

        // CRITICAL OPTIMIZATION: Fast path for appending at end (no shift needed!)
        if pos == len {
            array.push(value);
            return Ok(());
        }

        // OPTIMIZATION: Use Vec::insert which uses memmove internally
        // Much faster than manual clone loop
        array.insert(pos, value);
        Ok(())
    }

    /// Remove value at position in array part, shifting elements to the left
    /// Position is 0-indexed internally but Lua uses 1-indexed
    pub fn remove_array_at(&mut self, pos: usize) -> Result<LuaValue, String> {
        let len = self.len();
        if pos >= len {
            return Err("remove position out of bounds".to_string());
        }

        let array = self.array.as_mut().expect("array should exist");
        let removed = array[pos].clone();

        // CRITICAL OPTIMIZATION: Fast path for removing from end (no shift needed!)
        if pos == len - 1 {
            // Just pop the last element
            array.pop();
            return Ok(removed);
        }

        // OPTIMIZATION: Use copy_within for bulk memory move instead of clone loop
        // This is much faster as it's a single memmove operation
        array.copy_within(pos + 1..len, pos);

        // Remove the last element (now duplicated)
        array.pop();

        Ok(removed)
    }

    /// Iterator for GC - returns all key-value pairs
    pub fn iter_all(&self) -> Vec<(LuaValue, LuaValue)> {
        let mut result = Vec::new();

        // Iterate array part
        if let Some(array) = &self.array {
            for (i, val) in array.iter().enumerate() {
                if !val.is_nil() {
                    result.push((LuaValue::integer((i + 1) as i64), *val));
                }
            }
        }

        // Iterate hash part - linear scan!
        if let Some(nodes) = &self.nodes {
            for node in nodes {
                if !node.is_empty() {
                    result.push((node.key, node.value));
                }
            }
        }

        result
    }
}
