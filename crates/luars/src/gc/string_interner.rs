use ahash::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

use crate::{GcObject, GcPool, GcPtrObject, LuaValue, StringId};

/// Complete string interner - ALL strings are interned for maximum performance
/// - Same content always returns same StringId
/// - StringId equality = content equality (no string comparison needed)
/// - O(1) hash lookup for new strings (using ahash for speed)
/// - GC can collect unused strings via mark-sweep
///
/// 所有字符串（包括长字符串）都被 intern，确保：
/// 1. 相同内容的字符串只存储一份
/// 2. 字符串比较只需比较 StringId（O(1)）
/// 3. Table key 查找更快
pub struct StringInterner {
    // Content hash -> StringIds mapping for deduplication
    // 使用 ahash 作为哈希算法以提升性能
    map: HashMap<u64, Vec<StringId>, RandomState>,

    hashbuilder: RandomState,
}

impl StringInterner {
    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity_and_hasher(256, RandomState::new()),
            hashbuilder: RandomState::new(),
        }
    }

    /// Intern a string - returns existing StringId if already interned, creates new otherwise
    /// 所有字符串都会被 intern，保证相同内容只存储一份
    pub fn intern(&mut self, s: &str, gc_pool: &mut GcPool) -> (LuaValue, bool) {
        let hash = self.hash_string(s);

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

    /// Fast hash function - uses ahash for speed
    #[inline(always)]
    fn hash_string(&self, s: &str) -> u64 {
        let mut hasher = self.hashbuilder.build_hasher();
        s.hash(&mut hasher);
        hasher.finish()
    }

    /// Remove dead strings (called by GC)
    pub fn remove_dead_intern(&mut self, id: StringId, s: &str) {
        let hash = self.hash_string(s);
        // Remove from map
        if let Some(ids) = self.map.get_mut(&hash) {
            ids.retain(|&i| i != id);
            if ids.is_empty() {
                self.map.remove(&hash);
            }
        }
    }
}
