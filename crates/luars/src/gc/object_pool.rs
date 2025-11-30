// Object Pool V2 - High-performance single-threaded design
//
// Key Design Principles:
// 1. LuaValueV2 stores type tag + object ID (no pointers - Vec may relocate)
// 2. All GC objects accessed via ID lookup in Arena
// 3. ChunkedArena uses fixed-size chunks - never reallocates existing data!
// 4. No Rc/RefCell overhead - direct access via &mut self
// 5. GC headers embedded in objects for mark-sweep
//
// Memory Layout:
// - ChunkedArena stores objects in fixed-size chunks (Box<[Option<T>; CHUNK_SIZE]>)
// - Each chunk is allocated once and never moved
// - New chunks are added as needed, existing chunks stay in place
// - This eliminates Vec resize overhead and improves cache locality

use crate::lua_value::{Chunk, LuaThread, LuaUserdata};
use crate::{LuaString, LuaTable, LuaValue};
use std::hash::Hash;
use std::rc::Rc;

// ============ GC Header ============

/// GC object header - embedded in every GC-managed object
/// Kept minimal (2 bytes) to reduce memory overhead
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GcHeader {
    pub marked: bool,
    pub age: u8, // For generational GC
}

// ============ Object IDs ============
// These are just indices into the Arena storage
// They are small (4 bytes) and can be embedded in LuaValueV2

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct StringId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct TableId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct FunctionId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct UpvalueId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct UserdataId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct ThreadId(pub u32);

// ============ GC-managed Objects ============

/// Table with embedded GC header
pub struct GcTable {
    pub header: GcHeader,
    pub data: LuaTable,
}

/// Lua function with embedded GC header
pub struct GcFunction {
    pub header: GcHeader,
    pub chunk: Rc<Chunk>,
    pub upvalues: Vec<UpvalueId>, // Upvalue IDs, not Rc
}

/// Upvalue state - uses absolute stack index like Lua C implementation
#[derive(Debug, Clone)]
pub enum UpvalueState {
    Open { stack_index: usize },
    Closed(LuaValue),
}

/// Upvalue with embedded GC header
pub struct GcUpvalue {
    pub header: GcHeader,
    pub state: UpvalueState,
}

impl GcUpvalue {
    /// Check if this upvalue points to the given absolute stack index
    #[inline]
    pub fn points_to_index(&self, index: usize) -> bool {
        matches!(&self.state, UpvalueState::Open { stack_index } if *stack_index == index)
    }

    /// Check if this upvalue is open (still points to stack)
    #[inline]
    pub fn is_open(&self) -> bool {
        matches!(&self.state, UpvalueState::Open { .. })
    }

    /// Close this upvalue with the given value
    #[inline]
    pub fn close(&mut self, value: LuaValue) {
        self.state = UpvalueState::Closed(value);
    }

    /// Get the value of a closed upvalue (returns None if still open)
    #[inline]
    pub fn get_closed_value(&self) -> Option<LuaValue> {
        match &self.state {
            UpvalueState::Closed(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Get the absolute stack index if this upvalue is open
    #[inline]
    pub fn get_stack_index(&self) -> Option<usize> {
        match &self.state {
            UpvalueState::Open { stack_index } => Some(*stack_index),
            _ => None,
        }
    }
}

/// String with embedded GC header
pub struct GcString {
    pub header: GcHeader,
    pub data: LuaString,
}

/// Thread (coroutine) with embedded GC header
pub struct GcThread {
    pub header: GcHeader,
    pub data: LuaThread,
}

// ============ Chunked Arena Storage ============

/// Chunk size for arena storage (power of 2 for fast division)
const CHUNK_SIZE: usize = 256;
const CHUNK_MASK: usize = CHUNK_SIZE - 1;
const CHUNK_SHIFT: u32 = 8; // log2(256)

/// High-performance chunked arena that NEVER reallocates existing data
/// - Uses fixed-size chunks stored in a Vec of Box pointers
/// - When Vec of chunks grows, only the pointer array moves, not the data
/// - Each chunk is a Box<[Option<T>; CHUNK_SIZE]> - stable address
/// - Free list enables O(1) allocation after initial growth
pub struct Arena<T> {
    chunks: Vec<Box<[Option<T>; CHUNK_SIZE]>>,
    free_list: Vec<u32>,
    next_id: u32,
    count: usize,
}

impl<T> Arena<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            free_list: Vec::new(),
            next_id: 0,
            count: 0,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        let num_chunks = (cap + CHUNK_SIZE - 1) / CHUNK_SIZE;
        Self {
            chunks: Vec::with_capacity(num_chunks),
            free_list: Vec::with_capacity(cap / 8),
            next_id: 0,
            count: 0,
        }
    }

    /// Create a new empty chunk
    #[inline]
    fn new_chunk() -> Box<[Option<T>; CHUNK_SIZE]> {
        // Use MaybeUninit to avoid initializing 256 Option<T>s one by one
        // This is safe because Option<T> with None is just zeros for most T
        let mut chunk: Box<[Option<T>; CHUNK_SIZE]> = Box::new(std::array::from_fn(|_| None));
        chunk
    }

    /// Allocate a new object and return its ID
    #[inline]
    pub fn alloc(&mut self, value: T) -> u32 {
        self.count += 1;

        if let Some(free_id) = self.free_list.pop() {
            // Reuse a free slot
            let chunk_idx = (free_id >> CHUNK_SHIFT) as usize;
            let slot_idx = (free_id as usize) & CHUNK_MASK;
            self.chunks[chunk_idx][slot_idx] = Some(value);
            return free_id;
        }

        // Allocate new slot
        let id = self.next_id;
        let chunk_idx = (id >> CHUNK_SHIFT) as usize;
        let slot_idx = (id as usize) & CHUNK_MASK;

        // Add new chunk if needed
        if chunk_idx >= self.chunks.len() {
            self.chunks.push(Self::new_chunk());
        }

        self.chunks[chunk_idx][slot_idx] = Some(value);
        self.next_id += 1;
        id
    }

    /// Get immutable reference by ID
    #[inline(always)]
    pub fn get(&self, id: u32) -> Option<&T> {
        let chunk_idx = (id >> CHUNK_SHIFT) as usize;
        let slot_idx = (id as usize) & CHUNK_MASK;
        self.chunks
            .get(chunk_idx)
            .and_then(|chunk| chunk[slot_idx].as_ref())
    }

    /// Get reference by ID without bounds checking (caller must ensure validity)
    /// SAFETY: id must be a valid index returned from alloc() and not freed
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, id: u32) -> &T {
        let chunk_idx = (id >> CHUNK_SHIFT) as usize;
        let slot_idx = (id as usize) & CHUNK_MASK;
        unsafe {
            self.chunks
                .get_unchecked(chunk_idx)
                .get_unchecked(slot_idx)
                .as_ref()
                .unwrap_unchecked()
        }
    }

    /// Get mutable reference by ID
    #[inline(always)]
    pub fn get_mut(&mut self, id: u32) -> Option<&mut T> {
        let chunk_idx = (id >> CHUNK_SHIFT) as usize;
        let slot_idx = (id as usize) & CHUNK_MASK;
        self.chunks
            .get_mut(chunk_idx)
            .and_then(|chunk| chunk[slot_idx].as_mut())
    }

    /// Get mutable reference by ID without bounds checking (caller must ensure validity)
    /// SAFETY: id must be a valid index returned from alloc() and not freed
    #[inline(always)]
    pub unsafe fn get_mut_unchecked(&mut self, id: u32) -> &mut T {
        let chunk_idx = (id >> CHUNK_SHIFT) as usize;
        let slot_idx = (id as usize) & CHUNK_MASK;
        unsafe {
            self.chunks
                .get_unchecked_mut(chunk_idx)
                .get_unchecked_mut(slot_idx)
                .as_mut()
                .unwrap_unchecked()
        }
    }

    /// Free a slot (mark for reuse)
    #[inline]
    pub fn free(&mut self, id: u32) {
        let chunk_idx = (id >> CHUNK_SHIFT) as usize;
        let slot_idx = (id as usize) & CHUNK_MASK;
        if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
            if chunk[slot_idx].is_some() {
                chunk[slot_idx] = None;
                self.free_list.push(id);
                self.count -= 1;
            }
        }
    }

    /// Check if a slot is occupied
    #[inline(always)]
    pub fn is_valid(&self, id: u32) -> bool {
        let chunk_idx = (id >> CHUNK_SHIFT) as usize;
        let slot_idx = (id as usize) & CHUNK_MASK;
        self.chunks
            .get(chunk_idx)
            .map(|chunk| chunk[slot_idx].is_some())
            .unwrap_or(false)
    }

    /// Current number of live objects
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Iterate over all live objects
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.chunks.iter().enumerate().flat_map(|(chunk_idx, chunk)| {
            chunk.iter().enumerate().filter_map(move |(slot_idx, opt)| {
                opt.as_ref()
                    .map(|v| (((chunk_idx as u32) << CHUNK_SHIFT) | (slot_idx as u32), v))
            })
        })
    }

    /// Iterate over all live objects mutably
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T)> {
        self.chunks
            .iter_mut()
            .enumerate()
            .flat_map(|(chunk_idx, chunk)| {
                chunk
                    .iter_mut()
                    .enumerate()
                    .filter_map(move |(slot_idx, opt)| {
                        opt.as_mut()
                            .map(|v| (((chunk_idx as u32) << CHUNK_SHIFT) | (slot_idx as u32), v))
                    })
            })
    }

    /// Shrink internal storage (only shrinks chunk vector capacity, not individual chunks)
    pub fn shrink_to_fit(&mut self) {
        self.chunks.shrink_to_fit();
        self.free_list.shrink_to_fit();
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        for chunk in &mut self.chunks {
            for slot in chunk.iter_mut() {
                *slot = None;
            }
        }
        self.free_list.clear();
        self.next_id = 0;
        self.count = 0;
    }
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ============ Object Pool V2 ============

/// High-performance object pool for the Lua VM
/// All objects are stored in typed arenas and accessed by ID
pub struct ObjectPoolV2 {
    pub strings: Arena<GcString>,
    pub tables: Arena<GcTable>,
    pub functions: Arena<GcFunction>,
    pub upvalues: Arena<GcUpvalue>,
    pub userdata: Arena<LuaUserdata>,
    pub threads: Arena<GcThread>,

    // String interning table using Lua-style open addressing
    // Key: (hash, StringId) pairs in a flat array for cache efficiency
    // Uses linear probing with string content comparison for collision handling
    string_intern: StringInternTable,
    max_intern_length: usize,
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

impl ObjectPoolV2 {
    pub fn new() -> Self {
        Self {
            strings: Arena::with_capacity(256),
            tables: Arena::with_capacity(64),
            functions: Arena::with_capacity(32),
            upvalues: Arena::with_capacity(32),
            userdata: Arena::new(),
            threads: Arena::with_capacity(8),
            string_intern: StringInternTable::with_capacity(256),
            max_intern_length: 64, // Strings <= 64 bytes are interned
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

    #[inline(always)]
    fn hash_string(s: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    #[inline(always)]
    pub fn get_string(&self, id: StringId) -> Option<&LuaString> {
        self.strings.get(id.0).map(|gs| &gs.data)
    }

    #[inline(always)]
    pub fn get_string_str(&self, id: StringId) -> Option<&str> {
        self.strings.get(id.0).map(|gs| gs.data.as_str())
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

    /// Clear all mark bits before GC mark phase
    pub fn clear_marks(&mut self) {
        for (_, gs) in self.strings.iter_mut() {
            gs.header.marked = false;
        }
        for (_, gt) in self.tables.iter_mut() {
            gt.header.marked = false;
        }
        for (_, gf) in self.functions.iter_mut() {
            gf.header.marked = false;
        }
        for (_, gu) in self.upvalues.iter_mut() {
            gu.header.marked = false;
        }
        for (_, gth) in self.threads.iter_mut() {
            gth.header.marked = false;
        }
    }

    /// Sweep phase: free all unmarked objects
    pub fn sweep(&mut self) {
        // Collect IDs to free (can't free while iterating)
        let strings_to_free: Vec<u32> = self
            .strings
            .iter()
            .filter(|(_, gs)| !gs.header.marked)
            .map(|(id, _)| id)
            .collect();
        let tables_to_free: Vec<u32> = self
            .tables
            .iter()
            .filter(|(_, gt)| !gt.header.marked)
            .map(|(id, _)| id)
            .collect();
        let functions_to_free: Vec<u32> = self
            .functions
            .iter()
            .filter(|(_, gf)| !gf.header.marked)
            .map(|(id, _)| id)
            .collect();
        let upvalues_to_free: Vec<u32> = self
            .upvalues
            .iter()
            .filter(|(_, gu)| !gu.header.marked)
            .map(|(id, _)| id)
            .collect();
        let threads_to_free: Vec<u32> = self
            .threads
            .iter()
            .filter(|(_, gth)| !gth.header.marked)
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

impl Default for ObjectPoolV2 {
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
        let mut pool = ObjectPoolV2::new();

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
        let mut pool = ObjectPoolV2::new();

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
        // Verify IDs are compact
        assert_eq!(std::mem::size_of::<StringId>(), 4);
        assert_eq!(std::mem::size_of::<TableId>(), 4);
        assert_eq!(std::mem::size_of::<FunctionId>(), 4);
        assert_eq!(std::mem::size_of::<UpvalueId>(), 4);
    }

    #[test]
    fn test_string_interning_many_strings() {
        // Test that many different strings with potential hash collisions
        // are all stored correctly
        let mut pool = ObjectPoolV2::new();
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
        let mut pool = ObjectPoolV2::new();

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
