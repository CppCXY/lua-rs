// Object Pool V3 - Simplified high-performance design
//
// Key Design Principles:
// 1. All IDs are u32 indices into Vec storage
// 2. Small objects (String, Function, Upvalue) use Vec<Option<T>>
// 3. Large objects (Table, Thread) use Vec<Option<Box<T>>> to avoid copy on resize
// 4. No chunking overhead - direct Vec indexing for O(1) access
// 5. Free list for slot reuse
// 6. GC headers embedded in objects for mark-sweep

use crate::lua_value::{Chunk, LuaThread, LuaUserdata};
use crate::{
    FunctionId, GcFunction, GcHeader, GcString, GcTable, GcThread, GcUpvalue, LuaString, LuaTable,
    LuaValue, StringId, TableId, ThreadId, UpvalueId, UpvalueState, UserdataId,
};
use std::rc::Rc;

// ============ Pool Storage ============

/// Simple Vec-based pool for small objects
/// - Direct O(1) indexing with no chunking overhead
/// - Free list for slot reuse
/// - Objects stored inline in Vec
pub struct Pool<T> {
    data: Vec<Option<T>>,
    free_list: Vec<u32>,
    count: usize,
}

impl<T> Pool<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            free_list: Vec::new(),
            count: 0,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            data: Vec::with_capacity(cap),
            free_list: Vec::with_capacity(cap / 8),
            count: 0,
        }
    }

    /// Allocate a new object and return its ID
    #[inline]
    pub fn alloc(&mut self, value: T) -> u32 {
        self.count += 1;

        if let Some(free_id) = self.free_list.pop() {
            self.data[free_id as usize] = Some(value);
            return free_id;
        }

        let id = self.data.len() as u32;
        self.data.push(Some(value));
        id
    }

    /// Get immutable reference by ID
    #[inline(always)]
    pub fn get(&self, id: u32) -> Option<&T> {
        self.data.get(id as usize).and_then(|opt| opt.as_ref())
    }

    /// Get reference by ID without bounds checking
    /// SAFETY: id must be a valid index from alloc() and not freed
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, id: u32) -> &T {
        unsafe {
            self.data
                .get_unchecked(id as usize)
                .as_ref()
                .unwrap_unchecked()
        }
    }

    /// Get mutable reference by ID
    #[inline(always)]
    pub fn get_mut(&mut self, id: u32) -> Option<&mut T> {
        self.data.get_mut(id as usize).and_then(|opt| opt.as_mut())
    }

    /// Get mutable reference by ID without bounds checking
    /// SAFETY: id must be a valid index from alloc() and not freed
    #[inline(always)]
    pub unsafe fn get_mut_unchecked(&mut self, id: u32) -> &mut T {
        unsafe {
            self.data
                .get_unchecked_mut(id as usize)
                .as_mut()
                .unwrap_unchecked()
        }
    }

    /// Free a slot (mark for reuse)
    #[inline]
    pub fn free(&mut self, id: u32) {
        if let Some(slot) = self.data.get_mut(id as usize) {
            if slot.is_some() {
                *slot = None;
                self.free_list.push(id);
                self.count -= 1;
            }
        }
    }

    /// Get number of free slots in the free list
    #[inline]
    pub fn free_slots_count(&self) -> usize {
        self.free_list.len()
    }

    /// Trim trailing None values from the pool to reduce iteration overhead
    /// This removes None values from the end of the data vec
    pub fn trim_tail(&mut self) {
        // Remove trailing None values
        while self.data.last().map_or(false, |v| v.is_none()) {
            self.data.pop();
        }
        // Remove free list entries that are now out of bounds
        let max_valid = self.data.len() as u32;
        self.free_list.retain(|&id| id < max_valid);
    }

    /// Check if a slot is occupied
    #[inline(always)]
    pub fn is_valid(&self, id: u32) -> bool {
        self.data
            .get(id as usize)
            .map(|opt| opt.is_some())
            .unwrap_or(false)
    }

    /// Current number of live objects
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Iterate over all live objects
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.data
            .iter()
            .enumerate()
            .filter_map(|(id, opt)| opt.as_ref().map(|v| (id as u32, v)))
    }

    /// Iterate over all live objects mutably
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T)> {
        self.data
            .iter_mut()
            .enumerate()
            .filter_map(|(id, opt)| opt.as_mut().map(|v| (id as u32, v)))
    }

    /// Shrink internal storage
    pub fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
        self.free_list.shrink_to_fit();
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        self.data.clear();
        self.free_list.clear();
        self.count = 0;
    }
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Box-based pool for large objects
/// - Objects stored as Box<T> to avoid copying on Vec resize
/// - Same interface as Pool<T>
pub struct BoxPool<T> {
    data: Vec<Option<Box<T>>>,
    free_list: Vec<u32>,
    count: usize,
}

impl<T> BoxPool<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            free_list: Vec::new(),
            count: 0,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            data: Vec::with_capacity(cap),
            free_list: Vec::with_capacity(cap / 8),
            count: 0,
        }
    }

    /// Allocate a new object and return its ID
    #[inline]
    pub fn alloc(&mut self, value: T) -> u32 {
        self.count += 1;

        if let Some(free_id) = self.free_list.pop() {
            self.data[free_id as usize] = Some(Box::new(value));
            return free_id;
        }

        let id = self.data.len() as u32;
        self.data.push(Some(Box::new(value)));
        id
    }

    /// Get immutable reference by ID
    #[inline(always)]
    pub fn get(&self, id: u32) -> Option<&T> {
        self.data
            .get(id as usize)
            .and_then(|opt| opt.as_ref().map(|b| b.as_ref()))
    }

    /// Get reference by ID without bounds checking
    /// SAFETY: id must be a valid index from alloc() and not freed
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, id: u32) -> &T {
        unsafe {
            self.data
                .get_unchecked(id as usize)
                .as_ref()
                .unwrap_unchecked()
                .as_ref()
        }
    }

    /// Get mutable reference by ID
    #[inline(always)]
    pub fn get_mut(&mut self, id: u32) -> Option<&mut T> {
        self.data
            .get_mut(id as usize)
            .and_then(|opt| opt.as_mut().map(|b| b.as_mut()))
    }

    /// Get mutable reference by ID without bounds checking
    /// SAFETY: id must be a valid index from alloc() and not freed
    #[inline(always)]
    pub unsafe fn get_mut_unchecked(&mut self, id: u32) -> &mut T {
        unsafe {
            self.data
                .get_unchecked_mut(id as usize)
                .as_mut()
                .unwrap_unchecked()
                .as_mut()
        }
    }

    /// Free a slot (mark for reuse)
    #[inline]
    pub fn free(&mut self, id: u32) {
        if let Some(slot) = self.data.get_mut(id as usize) {
            if slot.is_some() {
                *slot = None;
                self.free_list.push(id);
                self.count -= 1;
            }
        }
    }

    /// Get number of free slots in the free list
    #[inline]
    pub fn free_slots_count(&self) -> usize {
        self.free_list.len()
    }

    /// Trim trailing None values from the pool to reduce iteration overhead
    pub fn trim_tail(&mut self) {
        while self.data.last().map_or(false, |v| v.is_none()) {
            self.data.pop();
        }
        let max_valid = self.data.len() as u32;
        self.free_list.retain(|&id| id < max_valid);
    }

    /// Check if a slot is occupied
    #[inline(always)]
    pub fn is_valid(&self, id: u32) -> bool {
        self.data
            .get(id as usize)
            .map(|opt| opt.is_some())
            .unwrap_or(false)
    }

    /// Current number of live objects
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Iterate over all live objects
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.data
            .iter()
            .enumerate()
            .filter_map(|(id, opt)| opt.as_ref().map(|b| (id as u32, b.as_ref())))
    }

    /// Iterate over all live objects mutably
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T)> {
        self.data
            .iter_mut()
            .enumerate()
            .filter_map(|(id, opt)| opt.as_mut().map(|b| (id as u32, b.as_mut())))
    }

    /// Shrink internal storage
    pub fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
        self.free_list.shrink_to_fit();
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        self.data.clear();
        self.free_list.clear();
        self.count = 0;
    }
}

impl<T> Default for BoxPool<T> {
    fn default() -> Self {
        Self::new()
    }
}

// Keep Arena as an alias for backward compatibility
pub type Arena<T> = Pool<T>;

// ============ Object Pool V3 ============

/// High-performance object pool for the Lua VM
/// - Small objects (String, Function, Upvalue) use Pool<T> with direct Vec storage
/// - Large objects (Table, Thread) use BoxPool<T> to avoid copy on resize
pub struct ObjectPool {
    pub strings: Pool<GcString>,
    pub tables: BoxPool<GcTable>,
    pub functions: Pool<GcFunction>,
    pub upvalues: Pool<GcUpvalue>,
    pub userdata: Pool<LuaUserdata>,
    pub threads: BoxPool<GcThread>,

    // String interning table using Lua-style open addressing
    // Key: (hash, StringId) pairs in a flat array for cache efficiency
    // Uses linear probing with string content comparison for collision handling
    string_intern: StringInternTable,
    max_intern_length: usize,

    // Pre-cached metamethod name StringIds (like Lua's G(L)->tmname[])
    // These are created at initialization and never collected
    // Stored as StringId to avoid repeated hash lookup in hot paths
    pub tm_index: StringId,     // "__index"
    pub tm_newindex: StringId,  // "__newindex"
    pub tm_call: StringId,      // "__call"
    pub tm_tostring: StringId,  // "__tostring"
    pub tm_len: StringId,       // "__len"
    pub tm_pairs: StringId,     // "__pairs"
    pub tm_ipairs: StringId,    // "__ipairs"
    pub tm_gc: StringId,        // "__gc"
    pub tm_close: StringId,     // "__close"
    pub tm_mode: StringId,      // "__mode"
    pub tm_name: StringId,      // "__name"
    pub tm_eq: StringId,        // "__eq"
    pub tm_lt: StringId,        // "__lt"
    pub tm_le: StringId,        // "__le"
    pub tm_add: StringId,       // "__add"
    pub tm_sub: StringId,       // "__sub"
    pub tm_mul: StringId,       // "__mul"
    pub tm_div: StringId,       // "__div"
    pub tm_mod: StringId,       // "__mod"
    pub tm_pow: StringId,       // "__pow"
    pub tm_unm: StringId,       // "__unm"
    pub tm_idiv: StringId,      // "__idiv"
    pub tm_band: StringId,      // "__band"
    pub tm_bor: StringId,       // "__bor"
    pub tm_bxor: StringId,      // "__bxor"
    pub tm_bnot: StringId,      // "__bnot"
    pub tm_shl: StringId,       // "__shl"
    pub tm_shr: StringId,       // "__shr"
    pub tm_concat: StringId,    // "__concat"
    pub tm_metatable: StringId, // "__metatable"
    
    // Pre-cached coroutine status strings for fast coroutine.status
    pub str_suspended: StringId,  // "suspended"
    pub str_running: StringId,    // "running"
    pub str_normal: StringId,     // "normal"
    pub str_dead: StringId,       // "dead"
}

// ============ Lua-style String Interning Table ============

/// Lua-style string interning with open addressing hash table
/// Based on Lua 5.4's stringtable (lstring.c)
#[allow(dead_code)]
struct StringInternTable {
    // Each bucket stores Option<(hash, StringId)>
    // None = empty slot, Some = occupied
    buckets: Vec<Option<(u64, StringId)>>,
    count: usize,
    // Size is always power of 2 for fast modulo via bitwise AND
    size_mask: usize,
}

#[allow(unused)]
impl StringInternTable {
    const INITIAL_SIZE: usize = 128; // Must be power of 2
    const LOAD_FACTOR: f64 = 0.75;

    fn new() -> Self {
        Self {
            buckets: vec![None; Self::INITIAL_SIZE],
            count: 0,
            size_mask: Self::INITIAL_SIZE - 1,
        }
    }

    fn with_capacity(cap: usize) -> Self {
        // Round up to next power of 2
        let size = cap.next_power_of_two().max(Self::INITIAL_SIZE);
        Self {
            buckets: vec![None; size],
            count: 0,
            size_mask: size - 1,
        }
    }

    /// Find existing string or return insertion index
    /// Returns: Ok(StringId) if found, Err(insert_index) if not found
    #[inline]
    fn find_or_insert_index<F>(&self, hash: u64, compare: F) -> Result<StringId, usize>
    where
        F: Fn(StringId) -> bool,
    {
        let mut idx = (hash as usize) & self.size_mask;
        let start_idx = idx;

        loop {
            match &self.buckets[idx] {
                None => {
                    // Empty slot - string not found, can insert here
                    return Err(idx);
                }
                Some((stored_hash, id)) => {
                    // Check hash first (fast rejection), then compare content
                    if *stored_hash == hash && compare(*id) {
                        return Ok(*id);
                    }
                }
            }
            // Linear probing
            idx = (idx + 1) & self.size_mask;
            if idx == start_idx {
                // Table is full (shouldn't happen with proper load factor)
                panic!("String intern table is full");
            }
        }
    }

    /// Insert a new string (caller must ensure it doesn't exist)
    #[inline]
    fn insert(&mut self, hash: u64, id: StringId, idx: usize) {
        self.buckets[idx] = Some((hash, id));
        self.count += 1;
    }

    /// Check if resize is needed and perform it
    fn maybe_resize<F>(&mut self, get_string_hash: F)
    where
        F: Fn(StringId) -> u64,
    {
        let threshold = ((self.buckets.len() as f64) * Self::LOAD_FACTOR) as usize;
        if self.count < threshold {
            return;
        }

        // Double the size
        let new_size = self.buckets.len() * 2;
        let new_mask = new_size - 1;
        let mut new_buckets = vec![None; new_size];

        // Rehash all entries
        for bucket in self.buckets.iter() {
            if let Some((hash, id)) = bucket {
                let mut idx = (*hash as usize) & new_mask;
                // Find empty slot (linear probing)
                while new_buckets[idx].is_some() {
                    idx = (idx + 1) & new_mask;
                }
                new_buckets[idx] = Some((*hash, *id));
            }
        }

        self.buckets = new_buckets;
        self.size_mask = new_mask;
    }

    /// Remove a string from the table (for GC)
    fn remove(&mut self, hash: u64, id: StringId) {
        let mut idx = (hash as usize) & self.size_mask;
        let start_idx = idx;

        loop {
            match &self.buckets[idx] {
                None => return, // Not found
                Some((stored_hash, stored_id)) => {
                    if *stored_hash == hash && *stored_id == id {
                        // Found - remove it
                        // Note: With linear probing, we need to handle deletion carefully
                        // Simple approach: mark as deleted (tombstone) or rehash following entries
                        // For simplicity, we'll just clear and let rehash fix it on resize
                        self.buckets[idx] = None;
                        self.count -= 1;
                        // Rehash subsequent entries to maintain probe chain
                        self.rehash_after_delete(idx);
                        return;
                    }
                }
            }
            idx = (idx + 1) & self.size_mask;
            if idx == start_idx {
                return; // Not found (wrapped around)
            }
        }
    }

    /// Rehash entries after deletion to maintain probe chains
    fn rehash_after_delete(&mut self, deleted_idx: usize) {
        let mut idx = (deleted_idx + 1) & self.size_mask;

        while let Some((hash, id)) = self.buckets[idx] {
            // Check if this entry needs to be moved
            let natural_idx = (hash as usize) & self.size_mask;

            // If the entry's natural position is "before" the deleted slot
            // in the probe sequence, it might need to move
            if self.should_move(natural_idx, deleted_idx, idx) {
                self.buckets[deleted_idx] = Some((hash, id));
                self.buckets[idx] = None;
                // Continue rehashing from this newly emptied slot
                self.rehash_after_delete(idx);
                return;
            }

            idx = (idx + 1) & self.size_mask;
            if idx == deleted_idx {
                break;
            }
        }
    }

    /// Check if entry at `current` with natural index `natural` should move to `target`
    fn should_move(&self, natural: usize, target: usize, current: usize) -> bool {
        // Entry should move if target is between natural and current in probe order
        if natural <= current {
            // No wraparound in probe sequence
            target >= natural && target < current
        } else {
            // Probe sequence wrapped around
            target >= natural || target < current
        }
    }

    fn shrink_to_fit(&mut self) {
        // Could implement shrinking, but usually not needed
    }

    fn clear(&mut self) {
        self.buckets.fill(None);
        self.count = 0;
    }
}

impl ObjectPool {
    pub fn new() -> Self {
        let mut pool = Self {
            strings: Pool::with_capacity(256),
            tables: BoxPool::with_capacity(64),
            functions: Pool::with_capacity(32),
            upvalues: Pool::with_capacity(32),
            userdata: Pool::new(),
            threads: BoxPool::with_capacity(8),
            string_intern: StringInternTable::with_capacity(256),
            max_intern_length: 64, // Strings <= 64 bytes are interned
            // Placeholder values - will be initialized below
            tm_index: StringId(0),
            tm_newindex: StringId(0),
            tm_call: StringId(0),
            tm_tostring: StringId(0),
            tm_len: StringId(0),
            tm_pairs: StringId(0),
            tm_ipairs: StringId(0),
            tm_gc: StringId(0),
            tm_close: StringId(0),
            tm_mode: StringId(0),
            tm_name: StringId(0),
            tm_eq: StringId(0),
            tm_lt: StringId(0),
            tm_le: StringId(0),
            tm_add: StringId(0),
            tm_sub: StringId(0),
            tm_mul: StringId(0),
            tm_div: StringId(0),
            tm_mod: StringId(0),
            tm_pow: StringId(0),
            tm_unm: StringId(0),
            tm_idiv: StringId(0),
            tm_band: StringId(0),
            tm_bor: StringId(0),
            tm_bxor: StringId(0),
            tm_bnot: StringId(0),
            tm_shl: StringId(0),
            tm_shr: StringId(0),
            tm_concat: StringId(0),
            tm_metatable: StringId(0),
            str_suspended: StringId(0),
            str_running: StringId(0),
            str_normal: StringId(0),
            str_dead: StringId(0),
        };

        // Pre-create all metamethod name strings (like Lua's luaT_init)
        // These strings are interned and will never be collected
        pool.tm_index = pool.create_string("__index");
        pool.tm_newindex = pool.create_string("__newindex");
        pool.tm_call = pool.create_string("__call");
        pool.tm_tostring = pool.create_string("__tostring");
        pool.tm_len = pool.create_string("__len");
        pool.tm_pairs = pool.create_string("__pairs");
        pool.tm_ipairs = pool.create_string("__ipairs");
        pool.tm_gc = pool.create_string("__gc");
        pool.tm_close = pool.create_string("__close");
        pool.tm_mode = pool.create_string("__mode");
        pool.tm_name = pool.create_string("__name");
        pool.tm_eq = pool.create_string("__eq");
        pool.tm_lt = pool.create_string("__lt");
        pool.tm_le = pool.create_string("__le");
        pool.tm_add = pool.create_string("__add");
        pool.tm_sub = pool.create_string("__sub");
        pool.tm_mul = pool.create_string("__mul");
        pool.tm_div = pool.create_string("__div");
        pool.tm_mod = pool.create_string("__mod");
        pool.tm_pow = pool.create_string("__pow");
        pool.tm_unm = pool.create_string("__unm");
        pool.tm_idiv = pool.create_string("__idiv");
        pool.tm_band = pool.create_string("__band");
        pool.tm_bor = pool.create_string("__bor");
        pool.tm_bxor = pool.create_string("__bxor");
        pool.tm_bnot = pool.create_string("__bnot");
        pool.tm_shl = pool.create_string("__shl");
        pool.tm_shr = pool.create_string("__shr");
        pool.tm_concat = pool.create_string("__concat");
        pool.tm_metatable = pool.create_string("__metatable");
        
        // Pre-create coroutine status strings
        pool.str_suspended = pool.create_string("suspended");
        pool.str_running = pool.create_string("running");
        pool.str_normal = pool.create_string("normal");
        pool.str_dead = pool.create_string("dead");

        // Fix all metamethod name strings - they should never be collected
        // (like Lua's luaC_fix in luaT_init)
        pool.fix_string(pool.tm_index);
        pool.fix_string(pool.tm_newindex);
        pool.fix_string(pool.tm_call);
        pool.fix_string(pool.tm_tostring);
        pool.fix_string(pool.tm_len);
        pool.fix_string(pool.tm_pairs);
        pool.fix_string(pool.tm_ipairs);
        pool.fix_string(pool.tm_gc);
        pool.fix_string(pool.tm_close);
        pool.fix_string(pool.tm_mode);
        pool.fix_string(pool.tm_name);
        pool.fix_string(pool.tm_eq);
        pool.fix_string(pool.tm_lt);
        pool.fix_string(pool.tm_le);
        pool.fix_string(pool.tm_add);
        pool.fix_string(pool.tm_sub);
        pool.fix_string(pool.tm_mul);
        pool.fix_string(pool.tm_div);
        pool.fix_string(pool.tm_mod);
        pool.fix_string(pool.tm_pow);
        pool.fix_string(pool.tm_unm);
        pool.fix_string(pool.tm_idiv);
        pool.fix_string(pool.tm_band);
        pool.fix_string(pool.tm_bor);
        pool.fix_string(pool.tm_bxor);
        pool.fix_string(pool.tm_bnot);
        pool.fix_string(pool.tm_shl);
        pool.fix_string(pool.tm_shr);
        pool.fix_string(pool.tm_concat);
        pool.fix_string(pool.tm_metatable);
        pool.fix_string(pool.str_suspended);
        pool.fix_string(pool.str_running);
        pool.fix_string(pool.str_normal);
        pool.fix_string(pool.str_dead);

        pool
    }

    /// Get pre-cached metamethod StringId by TM enum value
    /// This is the fast path for metamethod lookup in hot code
    /// TMS enum from ltm.h:
    /// TM_INDEX=0, TM_NEWINDEX=1, TM_GC=2, TM_MODE=3, TM_LEN=4, TM_EQ=5,
    /// TM_ADD=6, TM_SUB=7, TM_MUL=8, TM_MOD=9, TM_POW=10, TM_DIV=11,
    /// TM_IDIV=12, TM_BAND=13, TM_BOR=14, TM_BXOR=15, TM_SHL=16, TM_SHR=17,
    /// TM_UNM=18, TM_BNOT=19, TM_LT=20, TM_LE=21, TM_CONCAT=22, TM_CALL=23
    #[inline]
    pub fn get_binop_tm(&self, tm: u8) -> StringId {
        match tm {
            0 => self.tm_index,
            1 => self.tm_newindex,
            2 => self.tm_gc,
            3 => self.tm_mode,
            4 => self.tm_len,
            5 => self.tm_eq,
            6 => self.tm_add,
            7 => self.tm_sub,
            8 => self.tm_mul,
            9 => self.tm_mod,
            10 => self.tm_pow,
            11 => self.tm_div,
            12 => self.tm_idiv,
            13 => self.tm_band,
            14 => self.tm_bor,
            15 => self.tm_bxor,
            16 => self.tm_shl,
            17 => self.tm_shr,
            18 => self.tm_unm,
            19 => self.tm_bnot,
            20 => self.tm_lt,
            21 => self.tm_le,
            22 => self.tm_concat,
            23 => self.tm_call,
            24 => self.tm_close,
            _ => self.tm_index, // Fallback to __index
        }
    }

    // ==================== String Operations ====================

    /// Create or intern a string (Lua-style with proper hash collision handling)
    #[inline]
    pub fn create_string(&mut self, s: &str) -> StringId {
        let len = s.len();
        let hash = Self::hash_string(s);

        // Intern short strings for deduplication
        if len <= self.max_intern_length {
            // Use closure to compare string content (handles hash collisions correctly)
            let compare = |id: StringId| -> bool {
                self.strings
                    .get(id.0)
                    .map(|gs| gs.data.as_str() == s)
                    .unwrap_or(false)
            };

            match self.string_intern.find_or_insert_index(hash, compare) {
                Ok(existing_id) => {
                    // Found existing string with same content
                    return existing_id;
                }
                Err(insert_idx) => {
                    // Not found, create new interned string with pre-computed hash
                    let gc_string = GcString {
                        header: GcHeader::default(),
                        data: LuaString::with_hash(s.to_string(), hash),
                    };
                    let id = StringId(self.strings.alloc(gc_string));
                    self.string_intern.insert(hash, id, insert_idx);

                    // Check if resize needed (pass dummy closure since we just inserted)
                    self.string_intern.maybe_resize(|_| hash);

                    return id;
                }
            }
        } else {
            // Long strings are not interned, but still use pre-computed hash
            let gc_string = GcString {
                header: GcHeader::default(),
                data: LuaString::with_hash(s.to_string(), hash),
            };
            StringId(self.strings.alloc(gc_string))
        }
    }

    /// Create string from owned String (avoids clone if not interned)
    #[inline]
    pub fn create_string_owned(&mut self, s: String) -> StringId {
        let len = s.len();
        let hash = Self::hash_string(&s);

        if len <= self.max_intern_length {
            // Use closure to compare string content
            let compare = |id: StringId| -> bool {
                self.strings
                    .get(id.0)
                    .map(|gs| gs.data.as_str() == s.as_str())
                    .unwrap_or(false)
            };

            match self.string_intern.find_or_insert_index(hash, compare) {
                Ok(existing_id) => {
                    // Found existing string - drop the owned string
                    return existing_id;
                }
                Err(insert_idx) => {
                    // Not found, create new interned string with owned data and pre-computed hash
                    let gc_string = GcString {
                        header: GcHeader::default(),
                        data: LuaString::with_hash(s, hash),
                    };
                    let id = StringId(self.strings.alloc(gc_string));
                    self.string_intern.insert(hash, id, insert_idx);

                    // Check if resize needed
                    self.string_intern.maybe_resize(|_| hash);

                    return id;
                }
            }
        } else {
            // Long strings use pre-computed hash
            let gc_string = GcString {
                header: GcHeader::default(),
                data: LuaString::with_hash(s, hash),
            };
            StringId(self.strings.alloc(gc_string))
        }
    }

    /// Fast hash function - use byte-level mixing for better distribution
    /// Especially for strings with common prefixes like "key1", "key2", etc.
    #[inline(always)]
    fn hash_string(s: &str) -> u64 {
        // Use a simple but effective hash: FNV-1a variant
        // This has better distribution for similar strings than FxHash
        let bytes = s.as_bytes();
        let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
        for &byte in bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3); // FNV prime
        }
        hash
    }

    #[inline(always)]
    pub fn get_string(&self, id: StringId) -> Option<&LuaString> {
        self.strings.get(id.0).map(|gs| &gs.data)
    }

    #[inline(always)]
    pub fn get_string_str(&self, id: StringId) -> Option<&str> {
        self.strings.get(id.0).map(|gs| gs.data.as_str())
    }

    /// Create a substring from an existing string (optimized for string.sub)
    /// Returns the original string ID if the range covers the entire string.
    /// This avoids allocating a new String until we know it's actually needed.
    #[inline]
    pub fn create_substring(&mut self, source_id: StringId, start: usize, end: usize) -> StringId {
        // First phase: get substring info and check intern table
        let intern_result = {
            let Some(gs) = self.strings.get(source_id.0) else {
                return self.create_string("");
            };
            let s = gs.data.as_str();

            // Clamp indices
            let start = start.min(s.len());
            let end = end.min(s.len());

            if start >= end {
                return self.create_string("");
            }

            // Fast path: return original if full range
            if start == 0 && end == s.len() {
                return source_id;
            }

            let slice = &s[start..end];
            let len = slice.len();
            let hash = Self::hash_string(slice);

            // For short strings, check intern table first before allocating
            if len <= self.max_intern_length {
                let compare = |id: StringId| -> bool {
                    self.strings
                        .get(id.0)
                        .map(|gs| gs.data.as_str() == slice)
                        .unwrap_or(false)
                };

                match self.string_intern.find_or_insert_index(hash, compare) {
                    Ok(existing_id) => {
                        // Found existing interned string! No allocation needed!
                        return existing_id;
                    }
                    Err(insert_idx) => {
                        // Need to allocate - save slice content and insert index
                        Some((slice.to_string(), hash, insert_idx))
                    }
                }
            } else {
                // Long strings - just need to allocate
                Some((slice.to_string(), hash, usize::MAX))
            }
        };

        // Second phase: allocate and insert (if needed)
        if let Some((substring, hash, insert_idx)) = intern_result {
            if insert_idx != usize::MAX {
                // Short string - insert into intern table at saved index
                let gc_string = GcString {
                    header: GcHeader::default(),
                    data: LuaString::with_hash(substring, hash),
                };
                let id = StringId(self.strings.alloc(gc_string));
                self.string_intern.insert(hash, id, insert_idx);
                self.string_intern.maybe_resize(|_| hash);
                id
            } else {
                // Long string - just allocate
                let gc_string = GcString {
                    header: GcHeader::default(),
                    data: LuaString::with_hash(substring, hash),
                };
                StringId(self.strings.alloc(gc_string))
            }
        } else {
            unreachable!()
        }
    }

    /// Mark a string as fixed (never collected) - like Lua's luaC_fix()
    /// Used for metamethod names and other permanent strings
    #[inline]
    pub fn fix_string(&mut self, id: StringId) {
        if let Some(gs) = self.strings.get_mut(id.0) {
            gs.header.set_fixed();
            gs.header.make_black(); // Always considered marked
        }
    }

    /// Mark a table as fixed (never collected)
    #[inline]
    pub fn fix_table(&mut self, id: TableId) {
        if let Some(gt) = self.tables.get_mut(id.0) {
            gt.header.set_fixed();
            gt.header.make_black();
        }
    }

    // ==================== Table Operations ====================

    #[inline]
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> TableId {
        let gc_table = GcTable {
            header: GcHeader::default(),
            data: LuaTable::new(array_size, hash_size),
        };
        TableId(self.tables.alloc(gc_table))
    }

    #[inline]
    pub fn create_table_default(&mut self) -> TableId {
        let gc_table = GcTable {
            header: GcHeader::default(),
            data: LuaTable::new(0, 0),
        };
        TableId(self.tables.alloc(gc_table))
    }

    #[inline(always)]
    pub fn get_table(&self, id: TableId) -> Option<&LuaTable> {
        self.tables.get(id.0).map(|gt| &gt.data)
    }

    /// Get table without bounds checking (caller must ensure validity)
    /// SAFETY: id must be a valid TableId from create_table
    #[inline(always)]
    pub unsafe fn get_table_unchecked(&self, id: TableId) -> &LuaTable {
        unsafe { &self.tables.get_unchecked(id.0).data }
    }

    #[inline(always)]
    pub fn get_table_mut(&mut self, id: TableId) -> Option<&mut LuaTable> {
        self.tables.get_mut(id.0).map(|gt| &mut gt.data)
    }

    /// Get mutable table without bounds checking (caller must ensure validity)
    /// SAFETY: id must be a valid TableId from create_table
    #[inline(always)]
    pub unsafe fn get_table_mut_unchecked(&mut self, id: TableId) -> &mut LuaTable {
        unsafe { &mut self.tables.get_mut_unchecked(id.0).data }
    }

    #[inline(always)]
    pub fn get_table_gc(&self, id: TableId) -> Option<&GcTable> {
        self.tables.get(id.0)
    }

    #[inline(always)]
    pub fn get_table_gc_mut(&mut self, id: TableId) -> Option<&mut GcTable> {
        self.tables.get_mut(id.0)
    }

    // ==================== Function Operations ====================

    #[inline]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalue_ids: Vec<UpvalueId>) -> FunctionId {
        let gc_func = GcFunction {
            header: GcHeader::default(),
            chunk,
            upvalues: upvalue_ids,
        };
        FunctionId(self.functions.alloc(gc_func))
    }

    #[inline(always)]
    pub fn get_function(&self, id: FunctionId) -> Option<&GcFunction> {
        self.functions.get(id.0)
    }

    /// Get function without bounds checking (caller must ensure validity)
    /// SAFETY: id must be a valid FunctionId from create_function
    #[inline(always)]
    pub unsafe fn get_function_unchecked(&self, id: FunctionId) -> &GcFunction {
        unsafe { self.functions.get_unchecked(id.0) }
    }

    #[inline(always)]
    pub fn get_function_mut(&mut self, id: FunctionId) -> Option<&mut GcFunction> {
        self.functions.get_mut(id.0)
    }

    // ==================== Upvalue Operations ====================

    #[inline]
    pub fn create_upvalue_open(&mut self, stack_index: usize) -> UpvalueId {
        let gc_uv = GcUpvalue {
            header: GcHeader::default(),
            state: UpvalueState::Open { stack_index },
        };
        UpvalueId(self.upvalues.alloc(gc_uv))
    }

    #[inline]
    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> UpvalueId {
        let gc_uv = GcUpvalue {
            header: GcHeader::default(),
            state: UpvalueState::Closed(value),
        };
        UpvalueId(self.upvalues.alloc(gc_uv))
    }

    #[inline(always)]
    pub fn get_upvalue(&self, id: UpvalueId) -> Option<&GcUpvalue> {
        self.upvalues.get(id.0)
    }

    /// Get upvalue without bounds checking
    /// SAFETY: id must be a valid UpvalueId
    #[inline(always)]
    pub unsafe fn get_upvalue_unchecked(&self, id: UpvalueId) -> &GcUpvalue {
        unsafe { self.upvalues.get_unchecked(id.0) }
    }

    #[inline(always)]
    pub fn get_upvalue_mut(&mut self, id: UpvalueId) -> Option<&mut GcUpvalue> {
        self.upvalues.get_mut(id.0)
    }

    /// Get mutable upvalue without bounds checking
    /// SAFETY: id must be a valid UpvalueId
    #[inline(always)]
    pub unsafe fn get_upvalue_mut_unchecked(&mut self, id: UpvalueId) -> &mut GcUpvalue {
        unsafe { self.upvalues.get_mut_unchecked(id.0) }
    }

    // ==================== Userdata Operations ====================

    #[inline]
    pub fn create_userdata(&mut self, userdata: LuaUserdata) -> UserdataId {
        UserdataId(self.userdata.alloc(userdata))
    }

    #[inline(always)]
    pub fn get_userdata(&self, id: UserdataId) -> Option<&LuaUserdata> {
        self.userdata.get(id.0)
    }

    #[inline(always)]
    pub fn get_userdata_mut(&mut self, id: UserdataId) -> Option<&mut LuaUserdata> {
        self.userdata.get_mut(id.0)
    }

    // ==================== Thread Operations ====================

    #[inline]
    pub fn create_thread(&mut self, thread: LuaThread) -> ThreadId {
        let gc_thread = GcThread {
            header: GcHeader::default(),
            data: thread,
        };
        ThreadId(self.threads.alloc(gc_thread))
    }

    #[inline(always)]
    pub fn get_thread(&self, id: ThreadId) -> Option<&LuaThread> {
        self.threads.get(id.0).map(|gt| &gt.data)
    }

    #[inline(always)]
    pub fn get_thread_mut(&mut self, id: ThreadId) -> Option<&mut LuaThread> {
        self.threads.get_mut(id.0).map(|gt| &mut gt.data)
    }

    #[inline(always)]
    pub fn get_thread_gc(&self, id: ThreadId) -> Option<&GcThread> {
        self.threads.get(id.0)
    }

    #[inline(always)]
    pub fn get_thread_gc_mut(&mut self, id: ThreadId) -> Option<&mut GcThread> {
        self.threads.get_mut(id.0)
    }

    // ==================== GC Support ====================

    /// Clear all mark bits before GC mark phase (make all objects white)
    pub fn clear_marks(&mut self) {
        for (_, gs) in self.strings.iter_mut() {
            gs.header.make_white(0);
        }
        for (_, gt) in self.tables.iter_mut() {
            gt.header.make_white(0);
        }
        for (_, gf) in self.functions.iter_mut() {
            gf.header.make_white(0);
        }
        for (_, gu) in self.upvalues.iter_mut() {
            gu.header.make_white(0);
        }
        for (_, gth) in self.threads.iter_mut() {
            gth.header.make_white(0);
        }
    }

    /// Sweep phase: free all unmarked (white) objects
    pub fn sweep(&mut self) {
        // Collect IDs to free (can't free while iterating)
        // White objects are unmarked and should be collected
        let strings_to_free: Vec<u32> = self
            .strings
            .iter()
            .filter(|(_, gs)| gs.header.is_white())
            .map(|(id, _)| id)
            .collect();
        let tables_to_free: Vec<u32> = self
            .tables
            .iter()
            .filter(|(_, gt)| gt.header.is_white())
            .map(|(id, _)| id)
            .collect();
        let functions_to_free: Vec<u32> = self
            .functions
            .iter()
            .filter(|(_, gf)| gf.header.is_white())
            .map(|(id, _)| id)
            .collect();
        let upvalues_to_free: Vec<u32> = self
            .upvalues
            .iter()
            .filter(|(_, gu)| gu.header.is_white())
            .map(|(id, _)| id)
            .collect();
        let threads_to_free: Vec<u32> = self
            .threads
            .iter()
            .filter(|(_, gth)| gth.header.is_white())
            .map(|(id, _)| id)
            .collect();

        // Free collected IDs
        for id in strings_to_free {
            // Remove from intern table if interned
            if let Some(gs) = self.strings.get(id) {
                let hash = Self::hash_string(gs.data.as_str());
                self.string_intern.remove(hash, StringId(id));
            }
            self.strings.free(id);
        }
        for id in tables_to_free {
            self.tables.free(id);
        }
        for id in functions_to_free {
            self.functions.free(id);
        }
        for id in upvalues_to_free {
            self.upvalues.free(id);
        }
        for id in threads_to_free {
            self.threads.free(id);
        }
    }

    pub fn shrink_to_fit(&mut self) {
        self.strings.shrink_to_fit();
        self.tables.shrink_to_fit();
        self.functions.shrink_to_fit();
        self.upvalues.shrink_to_fit();
        self.threads.shrink_to_fit();
        self.string_intern.shrink_to_fit();
    }

    // ==================== Remove Operations (for GC) ====================

    #[inline]
    pub fn remove_string(&mut self, id: StringId) {
        self.strings.free(id.0);
    }

    #[inline]
    pub fn remove_table(&mut self, id: TableId) {
        self.tables.free(id.0);
    }

    #[inline]
    pub fn remove_function(&mut self, id: FunctionId) {
        self.functions.free(id.0);
    }

    #[inline]
    pub fn remove_upvalue(&mut self, id: UpvalueId) {
        self.upvalues.free(id.0);
    }

    #[inline]
    pub fn remove_userdata(&mut self, id: UserdataId) {
        self.userdata.free(id.0);
    }

    #[inline]
    pub fn remove_thread(&mut self, id: ThreadId) {
        self.threads.free(id.0);
    }

    // ==================== Statistics ====================

    #[inline]
    pub fn string_count(&self) -> usize {
        self.strings.len()
    }
    #[inline]
    pub fn table_count(&self) -> usize {
        self.tables.len()
    }
    #[inline]
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }
    #[inline]
    pub fn upvalue_count(&self) -> usize {
        self.upvalues.len()
    }
    #[inline]
    pub fn userdata_count(&self) -> usize {
        self.userdata.len()
    }
    #[inline]
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }
}

impl Default for ObjectPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_basic() {
        let mut arena: Arena<i32> = Arena::new();

        let id1 = arena.alloc(42);
        let id2 = arena.alloc(100);

        assert_eq!(arena.get(id1).copied(), Some(42));
        assert_eq!(arena.get(id2).copied(), Some(100));

        // Free id1
        arena.free(id1);
        assert!(!arena.is_valid(id1));

        // Allocate should reuse id1's slot
        let id3 = arena.alloc(200);
        assert_eq!(id3, id1);
        assert_eq!(arena.get(id3).copied(), Some(200));
    }

    #[test]
    fn test_arena_iteration() {
        let mut arena: Arena<i32> = Arena::new();

        arena.alloc(1);
        arena.alloc(2);
        let id3 = arena.alloc(3);
        arena.alloc(4);

        // Free middle element
        arena.free(id3);

        // Should iterate over 3 elements (1, 2, 4)
        let values: Vec<i32> = arena.iter().map(|(_, v)| *v).collect();
        assert_eq!(values, vec![1, 2, 4]);
    }

    #[test]
    fn test_string_interning() {
        let mut pool = ObjectPool::new();

        let id1 = pool.create_string("hello");
        let id2 = pool.create_string("hello");

        // Same string should return same ID
        assert_eq!(id1, id2);

        let id3 = pool.create_string("world");
        assert_ne!(id1, id3);

        // Verify content
        assert_eq!(pool.get_string_str(id1), Some("hello"));
        assert_eq!(pool.get_string_str(id3), Some("world"));
    }

    #[test]
    fn test_table_operations() {
        let mut pool = ObjectPool::new();

        let tid = pool.create_table(4, 4);

        // Modify table
        if let Some(table) = pool.get_table_mut(tid) {
            table.raw_set(LuaValue::integer(1), LuaValue::integer(42));
        }

        // Read back
        if let Some(table) = pool.get_table(tid) {
            assert_eq!(
                table.raw_get(&LuaValue::integer(1)),
                Some(LuaValue::integer(42))
            );
        }
    }

    #[test]
    fn test_object_ids_size() {
        // Verify all IDs are compact 4 bytes
        assert_eq!(std::mem::size_of::<StringId>(), 4);
        assert_eq!(std::mem::size_of::<TableId>(), 4);
        assert_eq!(std::mem::size_of::<FunctionId>(), 4);
        assert_eq!(std::mem::size_of::<UpvalueId>(), 4);
        assert_eq!(std::mem::size_of::<UserdataId>(), 4);
        assert_eq!(std::mem::size_of::<ThreadId>(), 4);
    }

    #[test]
    fn test_string_interning_many_strings() {
        // Test that many different strings with potential hash collisions
        // are all stored correctly
        let mut pool = ObjectPool::new();
        let mut ids = Vec::new();

        // Create 1000 different strings
        for i in 0..1000 {
            let s = format!("string_{}", i);
            let id = pool.create_string(&s);
            ids.push((s, id));
        }

        // Verify all strings are stored correctly
        for (s, id) in &ids {
            let stored = pool.get_string_str(*id);
            assert_eq!(
                stored,
                Some(s.as_str()),
                "String '{}' not stored correctly",
                s
            );
        }

        // Verify interning works - same string should return same ID
        for (s, id) in &ids {
            let id2 = pool.create_string(s);
            assert_eq!(*id, id2, "Interning failed for '{}'", s);
        }
    }

    #[test]
    fn test_string_interning_similar_strings() {
        // Test strings that might have similar hashes
        let mut pool = ObjectPool::new();

        let strings = vec![
            "a", "b", "c", "aa", "ab", "ba", "bb", "aaa", "aab", "aba", "abb", "baa", "bab", "bba",
            "bbb", "test", "Test", "TEST", "tEsT", "hello", "Hello", "HELLO", "hElLo",
        ];

        let mut ids = Vec::new();
        for s in &strings {
            ids.push(pool.create_string(s));
        }

        // All IDs should be unique (different strings)
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(
                    ids[i], ids[j],
                    "Different strings '{}' and '{}' got same ID",
                    strings[i], strings[j]
                );
            }
        }

        // Verify content
        for (i, s) in strings.iter().enumerate() {
            assert_eq!(pool.get_string_str(ids[i]), Some(*s));
        }
    }
}
