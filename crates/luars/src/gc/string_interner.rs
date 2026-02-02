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
    #[inline]
    pub fn intern(&mut self, s: &str, gc: &mut GC) -> CreateResult {
        let current_white = gc.current_white;
        let hash = self.hash_string(s);
        let slen = s.len();

        // Long strings are not interned (like Lua 5.5)
        if slen > Self::SHORT_STRING_LIMIT {
            let size = (std::mem::size_of::<GcString>() + slen) as u32;
            let lua_string = LuaString::new(s.to_string(), hash);
            let gc_string =
                GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
            let ptr = gc_string.as_str_ptr().unwrap();
            gc.trace_object(gc_string)?;
            return Ok(LuaValue::longstring(ptr));
        }

        // Short strings: check if already interned
        // OPTIMIZATION: Check length first (cheapest comparison)
        if let Some(ptrs) = self.map.get(&hash) {
            for &ptr in ptrs {
                let gc_str = ptr.as_ref();
                // Fast path: compare length first, then content
                if gc_str.data.str.len() == slen {
                    // Length matches, check content
                    if gc_str.data.str == s {
                        // Found! Resurrect if needed
                        if gc_str.header.is_white() {
                            ptr.as_mut_ref().header.make_black();
                        }
                        return Ok(LuaValue::shortstring(ptr));
                    }
                }
            }
        }

        // Not found - create new short string
        let size = (std::mem::size_of::<GcString>() + slen) as u32;
        let lua_string = LuaString::new(s.to_string(), hash);
        let gc_string =
            GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
        let ptr = gc_string.as_str_ptr().unwrap();
        gc.trace_object(gc_string)?;

        // Add to intern map
        self.map.entry(hash).or_insert_with(Vec::new).push(ptr);

        Ok(LuaValue::shortstring(ptr))
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
