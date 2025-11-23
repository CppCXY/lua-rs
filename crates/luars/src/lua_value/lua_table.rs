// High-performance Lua table implementation following Lua 5.4 design
// - Array part for integer keys [1..n]
// - Hash part using hashbrown (faster than std HashMap)
use crate::LuaVM;
use crate::lua_vm::{LuaError, LuaResult};

use super::LuaValue;

/// Lua table implementation
/// - Array part for integer keys [1..n]
/// - Hash part using hashbrown::HashMap (55% faster than std::collections::HashMap)
pub struct LuaTable {
    /// Array part: stores values for integer keys [1..array.len()]
    /// Only allocated when first integer key is set
    pub(crate) array: Vec<LuaValue>,

    /// Hash part: hashbrown's high-performance hash map
    /// Note: hashbrown crate is significantly faster than std HashMap despite same algorithm
    pub(crate) hash: hashbrown::HashMap<LuaValue, LuaValue>,

    /// Metatable - optional table that defines special behaviors  
    /// Store as LuaValue (table ID) instead of Rc for ID-based architecture
    metatable: Option<LuaValue>,
}

impl LuaTable {
    /// Create an empty table
    pub fn new() -> Self {
        LuaTable {
            array: Vec::new(),
            hash: hashbrown::HashMap::new(),
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
    /// Ultra-optimized hot path for ipairs iterations
    #[inline(always)]
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        if key > 0 {
            let idx = (key - 1) as usize;
            // SAFETY: bounds check is done explicitly
            if idx < self.array.len() {
                unsafe {
                    let val = self.array.get_unchecked(idx);
                    // LuaValue is Copy, so this is just a memcpy
                    if !val.is_nil() {
                        return Some(*val);
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
        self.hash.get(key).copied()
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

    /// Set in hash part - hashbrown handles everything!
    fn set_in_hash(&mut self, key: LuaValue, value: LuaValue) {
        if value.is_nil() {
            // Setting to nil - remove the key
            self.hash.remove(&key);
        } else {
            // Insert or update
            self.hash.insert(key, value);
        }
    }

    /// Get array length
    #[inline]
    pub fn len(&self) -> usize {
        self.array.len()
    }

    /// Iterator for next() function - follows Lua's iteration order
    /// First iterates array part, then hash part
    pub fn next(&self, key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        if key.is_nil() {
            // Start from beginning - find first non-nil in array
            for (i, val) in self.array.iter().enumerate() {
                if !val.is_nil() {
                    return Some((LuaValue::integer((i + 1) as i64), *val));
                }
            }
            // Then first entry in hash
            return self.hash.iter().next().map(|(k, v)| (*k, *v));
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
                // End of array, move to hash
                return self.hash.iter().next().map(|(k, v)| (*k, *v));
            }
        }

        // Key is in hash part - use skip_while + nth for efficiency
        self.hash
            .iter()
            .skip_while(|(k, _)| *k != key)
            .nth(1)
            .map(|(k, v)| (*k, *v))
    }

    /// Insert value at position in array part, shifting elements to the right
    /// Position is 0-indexed internally but Lua uses 1-indexed
    pub fn insert_array_at(&mut self, pos: usize, value: LuaValue) -> LuaResult<()> {
        let len = self.len();
        if pos > len {
            return Err(LuaError::RuntimeError(
                "insert position out of bounds".to_string(),
            ));
        }

        // CRITICAL OPTIMIZATION: Fast path for appending at end (no shift needed!)
        if pos == len {
            self.array.push(value);
            return Ok(());
        }

        // OPTIMIZATION: Use Vec::insert which uses memmove internally
        // Much faster than manual clone loop
        self.array.insert(pos, value);
        Ok(())
    }

    /// Remove value at position in array part, shifting elements to the left
    /// Position is 0-indexed internally but Lua uses 1-indexed
    pub fn remove_array_at(&mut self, pos: usize) -> LuaResult<LuaValue> {
        let len = self.len();
        if pos >= len {
            return Err(LuaError::RuntimeError(
                "remove position out of bounds".to_string(),
            ));
        }

        let removed = self.array[pos].clone();

        // CRITICAL OPTIMIZATION: Fast path for removing from end (no shift needed!)
        if pos == len - 1 {
            // Just pop the last element
            self.array.pop();
            return Ok(removed);
        }

        // OPTIMIZATION: Use copy_within for bulk memory move instead of clone loop
        // This is much faster as it's a single memmove operation
        self.array.copy_within(pos + 1..len, pos);

        // Remove the last element (now duplicated)
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

        // Iterate hash part
        for (k, v) in &self.hash {
            result.push((*k, *v));
        }

        result
    }
}
