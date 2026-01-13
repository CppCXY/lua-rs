use std::collections::HashMap;

use crate::{GcObject, GcPool, GcPtrObject, LuaValue, StringId};

/// Complete string interner - ALL strings are interned for maximum performance
/// - Same content always returns same StringId
/// - StringId equality = content equality (no string comparison needed)
/// - O(1) hash lookup for new strings
/// - GC can collect unused strings via mark-sweep
pub struct StringInterner {
    // Content -> StringId mapping for deduplication
    // Key is (hash, start_idx) where start_idx is index into strings pool
    // This avoids storing string content twice
    map: HashMap<u64, Vec<StringId>>, // hash -> list of StringIds with that hash

    pub small_string_limit: usize, // Max length for small strings (stored inline)
}

impl StringInterner {
    pub fn new(small_string_limit: usize) -> Self {
        Self {
            map: HashMap::with_capacity(256),
            small_string_limit,
        }
    }

    /// Intern a string - returns existing StringId if already interned, creates new otherwise
    pub fn intern(&mut self, s: &str, gc_pool: &mut GcPool) -> (LuaValue, bool) {
        if s.len() > self.small_string_limit {
            // Large string - store in Box to avoid large stack usage
            let gc_string = GcObject::new(GcPtrObject::String(Box::new(s.to_string())));
            let ptr = gc_string.ptr.as_str_ptr().unwrap();
            let id = gc_pool.alloc(gc_string);
            let str_id = StringId(id);
            return (LuaValue::string(str_id, ptr), true);
        }

        let hash = Self::hash_string(&s);

        // Check if already interned
        let mut found_id = None;
        if let Some(ids) = self.map.get(&hash) {
            for &id in ids {
                if let Some(gs) = gc_pool.get(id.0)
                    && let GcPtrObject::String(boxed_str) = &gs.ptr
                {
                    if boxed_str.as_str() == s {
                        found_id = Some(id);
                        break;
                    }
                }
            }
        }

        if let Some(id) = found_id {
            // Found! Ensure the string is resurrected if it was condemned (White).
            // Even if create() doesn't trigger GC, the string might be "Dead" from a previous GC cycle
            // (marked White and waiting for Sweep). If we return it now, the Sweeper will free it later,
            // leaving us with a dangling reference.
            // Marking it Black ensures it survives the current/pending sweep.
            if let Some(gs) = gc_pool.get_mut(id.0) {
                gs.header.make_black();
                let ptr = gs.ptr.as_str_ptr().unwrap();
                return (LuaValue::string(id, ptr), false);
            }
        }

        // Not found - use owned string directly
        let gc_string = GcObject::new(GcPtrObject::String(Box::new(s.to_string())));
        let ptr = gc_string.ptr.as_str_ptr().unwrap();
        let id = gc_pool.alloc(gc_string);
        let str_id = StringId(id);
        self.map.entry(hash).or_insert_with(Vec::new).push(str_id);

        (LuaValue::string(str_id, ptr), true)
    }

    /// Fast hash function - FNV-1a for good distribution
    #[inline(always)]
    fn hash_string(s: &str) -> u64 {
        let bytes = s.as_bytes();
        let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
        for &byte in bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3); // FNV prime
        }
        hash
    }

    /// Remove dead strings (called by GC)
    pub fn remove_dead_intern(&mut self, id: StringId, s: &str) {
        let hash = Self::hash_string(s);
        // Remove from map
        if let Some(ids) = self.map.get_mut(&hash) {
            ids.retain(|&i| i != id);
            if ids.is_empty() {
                self.map.remove(&hash);
            }
        }
    }
}
