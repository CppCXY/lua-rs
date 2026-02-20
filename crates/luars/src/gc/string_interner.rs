use ahash::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};

use crate::lua_value::LuaString;
use crate::{CreateResult, GC, GcObjectOwner, GcString, LuaValue, StringPtr};

/// Lua 5.5-style string table — flat bucket array with intrusive separate chaining.
///
/// Each bucket is a `StringPtr` (head of a singly-linked list via `LuaString.next`).
/// This mirrors C Lua's `stringtable` exactly:
/// - Zero extra allocation per bucket (no `Vec`, no `HashMap` overhead)
/// - Power-of-2 bucket count for fast modulo: `hash & (size - 1)`
/// - Grows when `nuse >= size` (load factor ≥ 1.0)
/// - Shrink support via `resize()` (called by GC when load < 0.25)
pub struct StringInterner {
    /// Bucket array — each entry is the head of a chain (null = empty bucket)
    buckets: Vec<StringPtr>,
    /// Number of interned strings
    nuse: usize,
    /// Hash builder (ahash for speed)
    hashbuilder: RandomState,
}

impl StringInterner {
    pub const SHORT_STRING_LIMIT: usize = 40;
    /// Initial bucket count (power of 2, same as C Lua)
    const INITIAL_SIZE: usize = 128;

    pub fn new() -> Self {
        Self {
            buckets: vec![StringPtr::null(); Self::INITIAL_SIZE],
            nuse: 0,
            hashbuilder: RandomState::new(),
        }
    }

    /// Number of buckets
    #[inline(always)]
    fn size(&self) -> usize {
        self.buckets.len()
    }

    /// Bucket index for a given hash
    #[inline(always)]
    fn bucket_index(&self, hash: u64) -> usize {
        (hash as usize) & (self.size() - 1)
    }

    /// Intern a string - returns existing StringPtr if already interned, creates new otherwise
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

        // Search the chain for an existing match (like C Lua's internshrstr)
        let idx = self.bucket_index(hash);
        let mut ts = self.buckets[idx];
        while !ts.is_null() {
            let gc_str = ts.as_ref();
            if gc_str.data.str.len() == slen && gc_str.data.str == s {
                // Found! Resurrect if dead-but-not-yet-collected
                if gc_str.header.is_white() {
                    ts.as_mut_ref().header.make_black();
                }
                return Ok(LuaValue::shortstring(ts));
            }
            ts = gc_str.data.next;
        }

        // Not found — grow table if load factor >= 1.0, then create
        if self.nuse >= self.size() {
            self.grow(gc);
        }
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
            let lua_string = LuaString::new(s, 0);
            let gc_string =
                GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
            let ptr = gc_string.as_str_ptr().unwrap();
            gc.trace_object(gc_string)?;
            return Ok(LuaValue::longstring(ptr));
        }

        let hash = self.hash_string(&s);

        // Search chain for existing match
        let idx = self.bucket_index(hash);
        let mut ts = self.buckets[idx];
        while !ts.is_null() {
            let gc_str = ts.as_ref();
            if gc_str.data.str.len() == slen && gc_str.data.str == s {
                // Found! Resurrect if needed. `s` dropped here — no waste.
                if gc_str.header.is_white() {
                    ts.as_mut_ref().header.make_black();
                }
                return Ok(LuaValue::shortstring(ts));
            }
            ts = gc_str.data.next;
        }

        // Not found — grow if needed, then create
        if self.nuse >= self.size() {
            self.grow(gc);
        }
        self.create_short_string(s, hash, slen, current_white, gc)
    }

    /// Create a new short string GC object and prepend to its bucket chain.
    /// This is the C Lua equivalent of: `ts->u.hnext = *list; *list = ts;`
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

        // Prepend to chain (must be done BEFORE trace_object, since ptr is stable after Box)
        let idx = self.bucket_index(hash);
        ptr.as_mut_ref().data.next = self.buckets[idx];
        self.buckets[idx] = ptr;
        self.nuse += 1;

        gc.trace_object(gc_string)?;
        Ok(LuaValue::shortstring(ptr))
    }

    /// Fast hash function - uses ahash for speed
    #[inline(always)]
    fn hash_string(&self, s: &str) -> u64 {
        let mut hasher = self.hashbuilder.build_hasher();
        s.hash(&mut hasher);
        hasher.finish()
    }

    /// Remove a dead short string from the intern table.
    /// Walks the chain to unlink the string (like C Lua's `luaS_remove`).
    pub fn remove_dead_intern(&mut self, ptr: StringPtr) {
        let gc_string = ptr.as_ref();
        let hash = gc_string.data.hash;
        let idx = self.bucket_index(hash);

        // Walk chain to find and unlink
        let mut prev_ptr: *mut StringPtr = &mut self.buckets[idx];
        unsafe {
            while !(*prev_ptr).is_null() {
                if *prev_ptr == ptr {
                    // Unlink: prev->next = current->next
                    *prev_ptr = ptr.as_ref().data.next;
                    self.nuse -= 1;
                    return;
                }
                prev_ptr = &mut (*prev_ptr).as_mut_ref().data.next;
            }
        }
    }

    /// Double the bucket array size and rehash all entries.
    /// Like C Lua's `growstrtab` / `luaS_resize`.
    fn grow(&mut self, _gc: &mut GC) {
        let new_size = self.size() * 2;
        self.resize_to(new_size);
    }

    /// Resize the bucket array (can grow or shrink). Power-of-2 only.
    /// Like C Lua's `luaS_resize` + `tablerehash`.
    pub fn resize(&mut self, new_size: usize) {
        // Clamp to power of 2
        let new_size = new_size.next_power_of_two();
        if new_size != self.size() {
            self.resize_to(new_size);
        }
    }

    fn resize_to(&mut self, new_size: usize) {
        debug_assert!(new_size.is_power_of_two());
        let old_size = self.size();

        // Extend bucket array with nulls (for grow) or prepare new array
        if new_size > old_size {
            self.buckets.resize(new_size, StringPtr::null());
        }

        // Rehash: redistribute all entries to correct buckets in the new-size array.
        // Walk every bucket in the old range, unlink each node, reinsert into new bucket.
        let mask = new_size - 1;
        for i in 0..old_size.min(new_size) {
            let mut p = self.buckets[i];
            self.buckets[i] = StringPtr::null();
            while !p.is_null() {
                let next = p.as_ref().data.next;
                let new_idx = (p.as_ref().data.hash as usize) & mask;
                p.as_mut_ref().data.next = self.buckets[new_idx];
                self.buckets[new_idx] = p;
                p = next;
            }
        }

        // Shrink: truncate array after rehashing moved all entries into lower buckets
        if new_size < old_size {
            self.buckets.truncate(new_size);
            self.buckets.shrink_to_fit();
        }
    }

    /// Check if the string table should shrink (called by GC during sweep).
    /// Like C Lua's `checkSizes`: shrink if `nuse < size / 4`.
    pub fn check_shrink(&mut self) {
        if self.nuse < self.size() / 4 && self.size() > Self::INITIAL_SIZE {
            self.resize(self.size() / 2);
        }
    }

    /// Get current intern table stats
    pub fn stats(&self) -> (usize, usize) {
        (self.nuse, self.size())
    }
}
