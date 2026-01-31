use ahash::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

use crate::lua_value::LuaString;
use crate::{CreateResult, GC, GcObjectOwner, GcString, LuaValue, StringPtr};

/// Complete string interner - ALL strings are interned for maximum performance
/// - Same content always returns same StringId
/// - O(1) hash lookup for new strings (using ahash for speed)
/// - GC can collect unused strings via mark-sweep
///
pub struct StringInterner {
    // Content hash -> StringIds mapping for deduplication
    // 使用 ahash 作为哈希算法以提升性能
    map: HashMap<u64, Vec<StringPtr>, RandomState>,

    hashbuilder: RandomState,
}

impl StringInterner {
    pub const SHORT_STRING_LIMIT: usize = 40;

    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity_and_hasher(256, RandomState::new()),
            hashbuilder: RandomState::new(),
        }
    }

    /// Intern a string - returns existing StringId if already interned, creates new otherwise
    pub fn intern(&mut self, s: &str, gc: &mut GC) -> CreateResult {
        let current_white = gc.current_white;
        let hash = self.hash_string(s);
        
        // Long strings are not interned (like Lua 5.5)
        if s.len() > Self::SHORT_STRING_LIMIT {
            let size = (std::mem::size_of::<GcString>() + s.len()) as u32;
            let lua_string = LuaString::new(s.to_string(), hash);
            let gc_string =
                GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
            let ptr = gc_string.as_str_ptr().unwrap();
            gc.trace_object(gc_string)?;
            // Return as long string (not interned)
            return Ok(LuaValue::longstring(ptr));
        }

        // Short strings: check if already interned
        let mut found_ptr = None;
        if let Some(ptrs) = self.map.get(&hash) {
            for &ptr in ptrs {
                // Quick check: compare string content directly (already in cache line)
                if ptr.as_ref().data.as_str() == s {
                    // Found! Check if it's dead and needs resurrection
                    let header = ptr.as_ref().header;
                    if header.is_white() {
                        // Resurrect by marking black (like Lua 5.5's changewhite)
                        ptr.as_mut_ref().header.make_black();
                    }
                    found_ptr = Some(ptr);
                    break;
                }
            }
        }

        if let Some(ptr) = found_ptr {
            return Ok(LuaValue::shortstring(ptr)); // Return as short string
        }

        // Not found - create new short string with correct white color
        let size = (std::mem::size_of::<GcString>() + s.len()) as u32;
        let lua_string = LuaString::new(s.to_string(), hash);
        let gc_string =
            GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
        let ptr = gc_string.as_str_ptr().unwrap();
        gc.trace_object(gc_string)?;
        self.map.entry(hash).or_insert_with(Vec::new).push(ptr);

        Ok(LuaValue::shortstring(ptr)) // Return as short string
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
