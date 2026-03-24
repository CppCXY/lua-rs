use ahash::RandomState;
#[cfg(feature = "shared-string")]
use std::collections::HashMap;
#[cfg(feature = "shared-string")]
use std::sync::{Arc, LazyLock, Mutex, Weak};

use crate::lua_value::{InlineShortString, LuaStrRepr, LuaString};
use crate::lua_vm::lua_limits::LUAI_MAXSHORTLEN;
use crate::{CreateResult, GC, GcObjectOwner, GcString, LuaValue, StringPtr};

#[cfg(feature = "shared-string")]
static SHARED_SHORT_STRING_POOL: LazyLock<Mutex<SharedShortStringPool>> =
    LazyLock::new(|| Mutex::new(SharedShortStringPool::default()));

#[cfg(feature = "shared-string")]
struct SharedShortStringPool {
    entries: HashMap<Vec<u8>, Weak<[u8]>>,
    cleanup_threshold: usize,
}

#[cfg(feature = "shared-string")]
impl Default for SharedShortStringPool {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            cleanup_threshold: 1024,
        }
    }
}

#[cfg(feature = "shared-string")]
impl SharedShortStringPool {
    fn intern(&mut self, bytes: &[u8]) -> Arc<[u8]> {
        if let Some(existing) = self.entries.get(bytes)
            && let Some(shared) = existing.upgrade()
        {
            return shared;
        }

        let shared = Arc::<[u8]>::from(bytes);
        self.entries
            .insert(shared.as_ref().to_vec(), Arc::downgrade(&shared));

        self.maybe_cleanup();

        shared
    }

    fn maybe_cleanup(&mut self) {
        if self.entries.len() < self.cleanup_threshold {
            return;
        }

        self.entries.retain(|_, weak| weak.strong_count() > 0);

        let grown = self.entries.len().saturating_mul(2).max(1024);
        self.cleanup_threshold = grown.next_power_of_two();
    }
}

#[derive(Copy, Clone)]
enum StringSlot {
    Empty,
    Tombstone,
    Occupied(StringPtr),
}

impl StringSlot {
    #[inline(always)]
    fn occupied(self) -> Option<StringPtr> {
        match self {
            Self::Occupied(ptr) => Some(ptr),
            Self::Empty | Self::Tombstone => None,
        }
    }
}

/// Open-addressed intern table for short strings.
pub struct StringInterner {
    slots: Vec<StringSlot>,
    /// Number of interned strings
    nuse: usize,
    ndead: usize,
    hashbuilder: RandomState,
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

impl StringInterner {
    pub const SHORT_STRING_LIMIT: usize = LUAI_MAXSHORTLEN;
    const INITIAL_SIZE: usize = 128;
    const MAX_LOAD_NUM: usize = 7;
    const MAX_LOAD_DEN: usize = 10;

    pub fn new() -> Self {
        Self {
            slots: vec![StringSlot::Empty; Self::INITIAL_SIZE],
            nuse: 0,
            ndead: 0,
            hashbuilder: RandomState::new(),
        }
    }

    #[inline(always)]
    fn size(&self) -> usize {
        self.slots.len()
    }

    #[inline(always)]
    fn slot_index(&self, hash: u64) -> usize {
        (hash as usize) & (self.size() - 1)
    }

    #[inline(always)]
    fn short_string_size() -> u32 {
        std::mem::size_of::<GcString>() as u32
    }

    #[inline(always)]
    fn long_string_size(slen: usize) -> u32 {
        (std::mem::size_of::<GcString>() + slen) as u32
    }

    #[inline]
    fn make_short_string_repr(bytes: &[u8]) -> LuaStrRepr {
        if bytes.len() <= InlineShortString::MAX_INLINE_LEN {
            return LuaStrRepr::Smol(InlineShortString::new(bytes));
        }

        #[cfg(feature = "shared-string")]
        {
            let shared = SHARED_SHORT_STRING_POOL
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .intern(bytes);
            return LuaStrRepr::Shared(shared);
        }

        #[cfg(not(feature = "shared-string"))]
        {
            LuaStrRepr::Heap(Box::<[u8]>::from(bytes))
        }
    }

    #[inline(always)]
    fn should_grow(&self) -> bool {
        (self.nuse + self.ndead) * Self::MAX_LOAD_DEN >= self.size() * Self::MAX_LOAD_NUM
    }

    #[inline(always)]
    fn hash_matches(ptr: StringPtr, hash: u64, s: &[u8]) -> bool {
        let gc_str = ptr.as_ref();
        gc_str.data.hash == hash && gc_str.data.as_bytes() == s
    }

    fn find_slot(&self, hash: u64, s: &[u8]) -> Result<usize, usize> {
        let mask = self.size() - 1;
        let mut index = self.slot_index(hash);
        let mut first_tombstone = None;

        loop {
            match self.slots[index] {
                StringSlot::Empty => return Err(first_tombstone.unwrap_or(index)),
                StringSlot::Tombstone => {
                    if first_tombstone.is_none() {
                        first_tombstone = Some(index);
                    }
                }
                StringSlot::Occupied(ptr) => {
                    if Self::hash_matches(ptr, hash, s) {
                        return Ok(index);
                    }
                }
            }
            index = (index + 1) & mask;
        }
    }

    #[inline]
    pub fn intern(&mut self, s: &str, gc: &mut GC) -> CreateResult {
        self.intern_bytes(s.as_bytes(), gc)
    }

    #[inline]
    pub fn intern_owned(&mut self, s: String, gc: &mut GC) -> CreateResult {
        self.intern_bytes_owned(s.into_bytes(), gc)
    }

    #[inline]
    pub fn intern_bytes(&mut self, bytes: &[u8], gc: &mut GC) -> CreateResult {
        let current_white = gc.current_white;
        let slen = bytes.len();

        if slen > Self::SHORT_STRING_LIMIT {
            let size = Self::long_string_size(slen);
            let lua_string = LuaString::from_bytes(LuaStrRepr::Heap(Box::<[u8]>::from(bytes)), 0);
            let gc_string =
                GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
            let ptr = gc_string.as_str_ptr().unwrap();
            gc.trace_object(gc_string)?;
            return Ok(LuaValue::longstring(ptr));
        }

        let hash = self.hash_bytes(bytes);

        if let Ok(index) = self.find_slot(hash, bytes)
            && let Some(ts) = self.slots[index].occupied()
        {
            let gc_str = ts.as_ref();
            if gc_str.header.is_white() {
                ts.as_mut_ref().header.make_black();
            }
            return Ok(LuaValue::shortstring(ts));
        }

        if self.should_grow() {
            self.grow(gc);
        }
        self.create_short_string(Self::make_short_string_repr(bytes), hash, current_white, gc)
    }

    #[inline]
    pub fn intern_bytes_owned(&mut self, bytes: Vec<u8>, gc: &mut GC) -> CreateResult {
        let current_white = gc.current_white;
        let slen = bytes.len();

        if slen > Self::SHORT_STRING_LIMIT {
            let size = Self::long_string_size(slen);
            let lua_string = LuaString::from_bytes(LuaStrRepr::Heap(bytes.into_boxed_slice()), 0);
            let gc_string =
                GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
            let ptr = gc_string.as_str_ptr().unwrap();
            gc.trace_object(gc_string)?;
            return Ok(LuaValue::longstring(ptr));
        }

        let hash = self.hash_bytes(&bytes);

        if let Ok(index) = self.find_slot(hash, &bytes)
            && let Some(ts) = self.slots[index].occupied()
        {
            let gc_str = ts.as_ref();
            if gc_str.header.is_white() {
                ts.as_mut_ref().header.make_black();
            }
            return Ok(LuaValue::shortstring(ts));
        }

        if self.should_grow() {
            self.grow(gc);
        }
        self.create_short_string(
            Self::make_short_string_repr(&bytes),
            hash,
            current_white,
            gc,
        )
    }

    #[inline]
    fn create_short_string(
        &mut self,
        s: LuaStrRepr,
        hash: u64,
        current_white: u8,
        gc: &mut GC,
    ) -> CreateResult {
        let size = Self::short_string_size();
        let lua_string = LuaString::from_bytes(s, hash);
        let gc_string =
            GcObjectOwner::String(Box::new(GcString::new(lua_string, current_white, size)));
        let ptr = gc_string.as_str_ptr().unwrap();

        let slot = match self.find_slot(hash, ptr.as_ref().data.as_bytes()) {
            Ok(index) | Err(index) => index,
        };
        if matches!(self.slots[slot], StringSlot::Tombstone) {
            self.ndead -= 1;
        }
        self.slots[slot] = StringSlot::Occupied(ptr);
        self.nuse += 1;

        gc.trace_object(gc_string)?;
        Ok(LuaValue::shortstring(ptr))
    }

    #[inline(always)]
    fn hash_bytes(&self, s: &[u8]) -> u64 {
        self.hashbuilder.hash_one(s)
    }

    pub fn remove_dead_intern(&mut self, ptr: StringPtr) {
        let gc_string = ptr.as_ref();
        let hash = gc_string.data.hash;
        let mut index = self.slot_index(hash);
        let mask = self.size() - 1;

        loop {
            match self.slots[index] {
                StringSlot::Empty => return,
                StringSlot::Tombstone => {}
                StringSlot::Occupied(candidate) => {
                    if candidate == ptr {
                        self.slots[index] = StringSlot::Tombstone;
                        self.nuse -= 1;
                        self.ndead += 1;
                        return;
                    }
                }
            }
            index = (index + 1) & mask;
        }
    }

    fn grow(&mut self, _gc: &mut GC) {
        let new_size = self.size() * 2;
        self.resize_to(new_size);
    }

    pub fn resize(&mut self, new_size: usize) {
        let new_size = new_size.next_power_of_two();
        if new_size != self.size() {
            self.resize_to(new_size);
        }
    }

    fn resize_to(&mut self, new_size: usize) {
        debug_assert!(new_size.is_power_of_two());
        let mut new_slots = vec![StringSlot::Empty; new_size];
        let mask = new_size - 1;

        for slot in self.slots.iter().copied() {
            if let StringSlot::Occupied(ptr) = slot {
                let mut index = (ptr.as_ref().data.hash as usize) & mask;
                loop {
                    if matches!(new_slots[index], StringSlot::Empty) {
                        new_slots[index] = StringSlot::Occupied(ptr);
                        break;
                    }
                    index = (index + 1) & mask;
                }
            }
        }

        self.slots = new_slots;
        self.ndead = 0;
    }

    pub fn check_shrink(&mut self) {
        if self.nuse < self.size() / 4 && self.size() > Self::INITIAL_SIZE {
            self.resize(self.size() / 2);
        } else if self.ndead > self.size() / 8 {
            self.resize_to(self.size());
        }
    }

    pub fn stats(&self) -> (usize, usize) {
        (self.nuse, self.size())
    }
}

#[cfg(all(test, feature = "shared-string"))]
mod tests {
    use super::StringInterner;
    use std::sync::Arc;

    use crate::GC;
    use crate::lua_value::LuaStrRepr;
    use crate::lua_vm::SafeOption;

    #[test]
    fn shared_short_strings_reuse_arc_storage_across_interners() {
        let key = "0123456789abcdefghijklmnopqr";
        let mut left_interner = StringInterner::new();
        let mut right_interner = StringInterner::new();
        let mut left_gc = GC::new(SafeOption::default());
        let mut right_gc = GC::new(SafeOption::default());

        let left = left_interner.intern(key, &mut left_gc).unwrap();
        let right = right_interner.intern(key, &mut right_gc).unwrap();

        let left_ptr = left.as_string_ptr().unwrap();
        let right_ptr = right.as_string_ptr().unwrap();

        match (&left_ptr.as_ref().data.str, &right_ptr.as_ref().data.str) {
            (LuaStrRepr::Shared(left), LuaStrRepr::Shared(right)) => {
                assert_eq!(left.as_ref(), right.as_ref());
                assert_eq!(Arc::as_ptr(left), Arc::as_ptr(right));
            }
            (left, right) => panic!("expected short strings, got {left:?} and {right:?}"),
        }
    }

    #[test]
    fn shared_string_storage_boundaries_follow_feature_thresholds() {
        let mut interner = StringInterner::new();
        let mut gc = GC::new(SafeOption::default());

        let smol = interner.intern(&"a".repeat(23), &mut gc).unwrap();
        let shared_24 = interner.intern(&"b".repeat(24), &mut gc).unwrap();
        let shared_60 = interner.intern(&"c".repeat(60), &mut gc).unwrap();
        let heap_61 = interner.intern(&"d".repeat(61), &mut gc).unwrap();

        assert!(smol.is_short_string());
        assert!(shared_24.is_short_string());
        assert!(shared_60.is_short_string());
        assert!(!heap_61.is_short_string());

        assert!(matches!(
            smol.as_string_ptr().unwrap().as_ref().data.str,
            LuaStrRepr::Smol(_)
        ));
        assert!(matches!(
            shared_24.as_string_ptr().unwrap().as_ref().data.str,
            LuaStrRepr::Shared(_)
        ));
        assert!(matches!(
            shared_60.as_string_ptr().unwrap().as_ref().data.str,
            LuaStrRepr::Shared(_)
        ));
        assert!(matches!(
            heap_61.as_string_ptr().unwrap().as_ref().data.str,
            LuaStrRepr::Heap(_)
        ));
    }
}

#[cfg(all(test, not(feature = "shared-string")))]
mod non_shared_tests {
    use super::StringInterner;

    use crate::GC;
    use crate::lua_value::LuaStrRepr;
    use crate::lua_vm::SafeOption;

    #[test]
    fn non_shared_storage_boundaries_follow_default_thresholds() {
        let mut interner = StringInterner::new();
        let mut gc = GC::new(SafeOption::default());

        let smol = interner.intern(&"a".repeat(23), &mut gc).unwrap();
        let heap_24 = interner.intern(&"b".repeat(24), &mut gc).unwrap();
        let heap_40 = interner.intern(&"c".repeat(40), &mut gc).unwrap();
        let heap_41 = interner.intern(&"d".repeat(41), &mut gc).unwrap();

        assert!(smol.is_short_string());
        assert!(heap_24.is_short_string());
        assert!(heap_40.is_short_string());
        assert!(!heap_41.is_short_string());

        assert!(matches!(
            smol.as_string_ptr().unwrap().as_ref().data.str,
            LuaStrRepr::Smol(_)
        ));
        assert!(matches!(
            heap_24.as_string_ptr().unwrap().as_ref().data.str,
            LuaStrRepr::Heap(_)
        ));
        assert!(matches!(
            heap_40.as_string_ptr().unwrap().as_ref().data.str,
            LuaStrRepr::Heap(_)
        ));
        assert!(matches!(
            heap_41.as_string_ptr().unwrap().as_ref().data.str,
            LuaStrRepr::Heap(_)
        ));
    }
}
