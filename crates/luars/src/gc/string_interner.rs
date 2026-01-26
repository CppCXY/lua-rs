use ahash::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

use crate::lua_value::LuaString;
use crate::{GC, GcObjectOwner, GcString, LuaValue, StringPtr};

/// Complete string interner - ALL strings are interned for maximum performance
/// - Same content always returns same StringId
/// - O(1) hash lookup for new strings (using ahash for speed)
/// - GC can collect unused strings via mark-sweep
///
pub struct StringInterner {
    // Content hash -> StringIds mapping for deduplication
    // 使用 ahash 作为哈希算法以提升性能
    map: HashMap<u64, Vec<StringPtr>, RandomState>,

    short_string_limit: usize,

    hashbuilder: RandomState,
}

impl StringInterner {
    pub fn new(short_string_limit: usize) -> Self {
        Self {
            map: HashMap::with_capacity_and_hasher(256, RandomState::new()),
            short_string_limit,
            hashbuilder: RandomState::new(),
        }
    }

    /// Intern a string - returns existing StringId if already interned, creates new otherwise
    pub fn intern(&mut self, s: &str, gc: &mut GC) -> LuaValue {
        let current_white = gc.current_white;
        let hash = self.hash_string(s);
        if s.len() > self.short_string_limit {
            // Long strings are not interned
            let size = (64 + s.len()) as u32;
            let lua_string = LuaString::new(s.to_string(), hash);
            let gc_string =
                GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
            let ptr = gc_string.as_str_ptr().unwrap();
            gc.trace_object(gc_string);
            return LuaValue::string(ptr);
        }

        // Check if already interned
        let mut found_ptr = None;
        if let Some(ptrs) = self.map.get(&hash) {
            for &ptr in ptrs {
                let header = ptr.as_ref().header;
                let other_white = 1 - gc.current_white;
                
                // Skip dead strings that haven't been removed from map yet
                if header.is_dead(other_white) {
                    continue;
                }
                
                if ptr.as_ref().data.as_str() == s {
                    found_ptr = Some(ptr);
                    break;
                }
            }
        }

        if let Some(ptr) = found_ptr {
            // Found! Ensure the string is resurrected if it was condemned (White).
            // Even if create() doesn't trigger GC, the string might be "Dead" from a previous GC cycle
            // (marked White and waiting for Sweep). If we return it now, the Sweeper will free it later,
            // leaving us with a dangling reference.
            // Marking it Black ensures it survives the current/pending sweep.
            ptr.as_mut_ref().header.make_black();
            return LuaValue::string(ptr);
        }

        // Not found - create with correct white color (Port of lgc.c: luaC_newobj)
        let size = (64 + s.len()) as u32;
        let lua_string = LuaString::new(s.to_string(), hash);
        let gc_string =
            GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
        let ptr = gc_string.as_str_ptr().unwrap();
        gc.trace_object(gc_string);
        self.map.entry(hash).or_insert_with(Vec::new).push(ptr);

        LuaValue::string(ptr)
    }

    /// Fast hash function - uses ahash for speed
    #[inline(always)]
    fn hash_string(&self, s: &str) -> u64 {
        let mut hasher = self.hashbuilder.build_hasher();
        s.hash(&mut hasher);
        hasher.finish()
    }

    /// Remove dead strings (called by GC)
    pub fn remove_dead_intern(&mut self, ptr: StringPtr) {
        let gc_string = ptr.as_ref();
        let hash = gc_string.data.hash;

        // Remove from map
        if let Some(ids) = self.map.get_mut(&hash) {
            ids.retain(|&i| i != ptr);
            if ids.is_empty() {
                self.map.remove(&hash);
            }
        }
    }
}
