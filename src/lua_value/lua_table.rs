use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{LuaString, LuaValue};

/// Lua table (mutable associative array)
/// Uses lazy allocation - only creates Vec/HashMap when needed
#[derive(Debug)]
pub struct LuaTable {
    /// Array part - allocated on first integer key access
    array: Option<Vec<LuaValue>>,
    /// Hash part - allocated on first non-array key access
    hash: Option<HashMap<LuaValue, LuaValue>>,
    /// Metatable - optional table that defines special behaviors
    metatable: Option<Rc<RefCell<LuaTable>>>,
}

impl LuaTable {
    pub fn new() -> Self {
        LuaTable {
            array: None,
            hash: None,
            metatable: None,
        }
    }

    /// Get the metatable of this table
    pub fn get_metatable(&self) -> Option<Rc<RefCell<LuaTable>>> {
        self.metatable.clone()
    }

    /// Set the metatable of this table
    pub fn set_metatable(&mut self, mt: Option<Rc<RefCell<LuaTable>>>) {
        self.metatable = mt;
    }

    /// Fast integer index access - specialized for array access
    #[inline(always)]
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        let idx = key as usize;
        if idx > 0 {
            if let Some(ref arr) = self.array {
                if idx <= arr.len() {
                    return arr.get(idx - 1).cloned();
                }
            }
        }
        // Fallback to hash for out-of-range integers
        self.hash
            .as_ref()
            .and_then(|h| h.get(&LuaValue::integer(key)).cloned())
    }

    /// Fast string key access - specialized for hash access
    #[inline(always)]
    pub fn get_str(&self, key: &Rc<LuaString>) -> Option<LuaValue> {
        self.hash
            .as_ref()
            .and_then(|h| h.get(&LuaValue::from_string_rc(Rc::clone(key))).cloned())
    }

    /// Get value with raw access (no metamethods)
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        // Fast paths for common cases
        if let Some(i) = key.as_integer() {
            return self.get_int(i);
        }
        if let Some(s) = key.as_string() {
            return self.get_str(&s);
        }

        // Generic fallback
        self.hash.as_ref().and_then(|h| h.get(key).cloned())
    }

    /// Fast integer index set - specialized for array access
    #[inline(always)]
    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        let idx = key as usize;
        if idx > 0 {
            let arr = self.array.get_or_insert_with(Vec::new);
            if idx <= arr.len() + 1 {
                if idx == arr.len() + 1 {
                    arr.push(value);
                } else {
                    arr[idx - 1] = value;
                }
                return;
            }
        }
        // Fallback to hash
        let hash = self.hash.get_or_insert_with(HashMap::new);
        hash.insert(LuaValue::integer(key), value);
    }

    /// Fast string key set - specialized for hash access
    #[inline(always)]
    pub fn set_str(&mut self, key: Rc<LuaString>, value: LuaValue) {
        let hash = self.hash.get_or_insert_with(HashMap::new);
        hash.insert(LuaValue::from_string_rc(key), value);
    }

    /// Set value with raw access (no metamethods)
    pub fn raw_set(&mut self, key: LuaValue, value: LuaValue) {
        // Fast paths for common cases
        if let Some(i) = key.as_integer() {
            self.set_int(i, value);
            return;
        }
        if let Some(s) = key.as_string() {
            self.set_str(s, value);
            return;
        }

        // Generic fallback
        let hash = self.hash.get_or_insert_with(HashMap::new);
        hash.insert(key, value);
    }

    pub fn len(&self) -> usize {
        self.array.as_ref().map(|a| a.len()).unwrap_or(0)
    }

    /// Iterate over all key-value pairs (both array and hash parts)
    pub fn iter_all(&self) -> impl Iterator<Item = (LuaValue, LuaValue)> + '_ {
        let array_iter = self
            .array
            .as_ref()
            .map(|a| {
                a.iter()
                    .enumerate()
                    .map(|(i, v)| (LuaValue::integer((i + 1) as i64), v.clone()))
            })
            .into_iter()
            .flatten();

        let hash_iter = self
            .hash
            .as_ref()
            .map(|h| h.iter().map(|(k, v)| (k.clone(), v.clone())))
            .into_iter()
            .flatten();

        array_iter.chain(hash_iter)
    }

    pub fn insert_array_at(&mut self, index: usize, value: LuaValue) -> Result<(), String> {
        let arr = self.array.get_or_insert_with(Vec::new);
        if index <= arr.len() {
            arr.insert(index, value);
        } else if index == arr.len() + 1 {
            arr.push(value);
        } else {
            return Err("Index out of bounds for array insertion".to_string());
        }

        Ok(())
    }

    pub fn remove_array_at(&mut self, index: usize) -> Result<LuaValue, String> {
        if let Some(ref mut arr) = self.array {
            if index < arr.len() {
                return Ok(arr.remove(index));
            }
        }
        Err("Index out of bounds for array removal".to_string())
    }

    pub fn get_array_part(&mut self) -> Option<&mut Vec<LuaValue>> {
        self.array.as_mut()
    }
}
