use ahash::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

use crate::lua_value::LuaString;
use crate::{CreateResult, GC, GcObjectOwner, GcString, LuaValue, StringPtr};

/// Identity hasher — passes through pre-computed u64 hash values without re-hashing.
/// Used because we already hash strings with ahash before HashMap lookup.
struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    #[inline(always)]
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!("IdentityHasher only accepts u64");
    }
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.0
    }
}

#[derive(Clone)]
struct IdentityBuildHasher;

impl BuildHasher for IdentityBuildHasher {
    type Hasher = IdentityHasher;
    #[inline(always)]
    fn build_hasher(&self) -> IdentityHasher {
        IdentityHasher(0)
    }
}

/// Complete string interner - ALL strings are interned for maximum performance
/// - Same content always returns same StringId
/// - O(1) hash lookup for new strings (using ahash for speed)
/// - GC can collect unused strings via mark-sweep
///
pub struct StringInterner {
    // Content hash -> StringIds mapping for deduplication
    // Uses IdentityBuildHasher to avoid double-hashing (ahash is applied on content,
    // then the u64 hash is used directly as HashMap key)
    map: HashMap<u64, Vec<StringPtr>, IdentityBuildHasher>,

    hashbuilder: RandomState,
}

impl StringInterner {
    pub const SHORT_STRING_LIMIT: usize = 40;

    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity_and_hasher(256, IdentityBuildHasher),
            hashbuilder: RandomState::new(),
        }
    }

    /// Intern a string - returns existing StringId if already interned, creates new otherwise
    #[inline]
    pub fn intern(&mut self, s: &str, gc: &mut GC) -> CreateResult {
        let current_white = gc.current_white;
        let slen = s.len();

        // Long strings are not interned (like Lua 5.5)
        // Use hash=0 (lazy hash — computed on demand when used as table key)
        if slen > Self::SHORT_STRING_LIMIT {
            let size = (std::mem::size_of::<GcString>() + slen) as u32;
            let lua_string = LuaString::new(s.to_string(), 0);
            let gc_string =
                GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
            let ptr = gc_string.as_str_ptr().unwrap();
            gc.trace_object(gc_string)?;
            return Ok(LuaValue::longstring(ptr));
        }

        let hash = self.hash_string(s);

        // Short strings: check if already interned
        if let Some(ptr) = self.find_interned(hash, s, slen) {
            return Ok(LuaValue::shortstring(ptr));
        }

        // Not found - create new short string
        self.create_short_string(s.to_string(), hash, slen, current_white, gc)
    }

    /// Intern an owned string - avoids extra allocation when string is already owned
    #[inline]
    pub fn intern_owned(&mut self, s: String, gc: &mut GC) -> CreateResult {
        let current_white = gc.current_white;
        let slen = s.len();

        // Long strings are not interned — lazy hash (hash=0)
        if slen > Self::SHORT_STRING_LIMIT {
            let size = (std::mem::size_of::<GcString>() + slen) as u32;
            let lua_string = LuaString::new(s, 0); // Takes ownership, no clone, no hash
            let gc_string =
                GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
            let ptr = gc_string.as_str_ptr().unwrap();
            gc.trace_object(gc_string)?;
            return Ok(LuaValue::longstring(ptr));
        }

        let hash = self.hash_string(&s);

        // Short strings: check if already interned
        if let Some(ptr) = self.find_interned(hash, &s, slen) {
            return Ok(LuaValue::shortstring(ptr));
            // `s` is dropped here — no waste since we found it in cache
        }

        // Not found - create new short string, taking ownership of s
        self.create_short_string(s, hash, slen, current_white, gc)
    }

    /// Look up an interned short string by hash and content
    #[inline]
    fn find_interned(&mut self, hash: u64, s: &str, slen: usize) -> Option<StringPtr> {
        if let Some(ptrs) = self.map.get(&hash) {
            for &ptr in ptrs {
                let gc_str = ptr.as_ref();
                if gc_str.data.str.len() == slen && gc_str.data.str == s {
                    // Found! Resurrect if needed
                    if gc_str.header.is_white() {
                        ptr.as_mut_ref().header.make_black();
                    }
                    return Some(ptr);
                }
            }
        }
        None
    }

    /// Create a new short string GC object and add to intern map
    #[inline]
    fn create_short_string(
        &mut self,
        s: String,
        hash: u64,
        slen: usize,
        current_white: u8,
        gc: &mut GC,
    ) -> CreateResult {
        let size = (std::mem::size_of::<GcString>() + slen) as u32;
        let lua_string = LuaString::new(s, hash);
        let gc_string =
            GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
        let ptr = gc_string.as_str_ptr().unwrap();
        gc.trace_object(gc_string)?;
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
