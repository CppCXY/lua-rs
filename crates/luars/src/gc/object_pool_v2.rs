// Object Pool V2 - High-performance single-threaded design
// 
// Key Design Principles:
// 1. LuaValueV2 stores type tag + object ID (no pointers - Vec may relocate)
// 2. All GC objects accessed via ID lookup in Arena
// 3. Arena uses Vec<Option<T>> with free list for O(1) alloc/free
// 4. No Rc/RefCell overhead - direct access via &mut self
// 5. GC headers embedded in objects for mark-sweep
//
// Memory Layout:
// - Arena<T> stores objects in Vec<Option<T>>
// - None = free slot (reusable via free list)
// - Free list tracks available slots for O(1) allocation

use crate::lua_value::{LuaUserdata, Chunk};
use crate::{LuaString, LuaTable, LuaValue};
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

// ============ GC Header ============

/// GC object header - embedded in every GC-managed object
/// Kept minimal (2 bytes) to reduce memory overhead
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GcHeader {
    pub marked: bool,
    pub age: u8,   // For generational GC
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
    pub upvalues: Vec<UpvalueId>,  // Upvalue IDs, not Rc
}

/// Upvalue state
#[derive(Debug, Clone)]
pub enum UpvalueState {
    Open { frame_id: usize, register: usize },
    Closed(LuaValue),
}

/// Upvalue with embedded GC header
pub struct GcUpvalue {
    pub header: GcHeader,
    pub state: UpvalueState,
}

/// String with embedded GC header
pub struct GcString {
    pub header: GcHeader,
    pub data: LuaString,
}

// ============ Arena Storage ============

/// Type-safe arena for storing GC objects
/// Uses Option<T> internally to mark free slots
/// Free list enables O(1) allocation after initial growth
pub struct Arena<T> {
    storage: Vec<Option<T>>,
    free_list: Vec<u32>,
    count: usize,
}

impl<T> Arena<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            storage: Vec::new(),
            free_list: Vec::new(),
            count: 0,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            storage: Vec::with_capacity(cap),
            free_list: Vec::with_capacity(cap / 8),
            count: 0,
        }
    }

    /// Allocate a new object and return its ID
    #[inline]
    pub fn alloc(&mut self, value: T) -> u32 {
        self.count += 1;
        
        if let Some(free_id) = self.free_list.pop() {
            // Reuse a free slot
            self.storage[free_id as usize] = Some(value);
            free_id
        } else {
            // Append new slot
            let id = self.storage.len() as u32;
            self.storage.push(Some(value));
            id
        }
    }

    /// Get immutable reference by ID
    #[inline(always)]
    pub fn get(&self, id: u32) -> Option<&T> {
        self.storage.get(id as usize).and_then(|opt| opt.as_ref())
    }

    /// Get mutable reference by ID
    #[inline(always)]
    pub fn get_mut(&mut self, id: u32) -> Option<&mut T> {
        self.storage.get_mut(id as usize).and_then(|opt| opt.as_mut())
    }

    /// Free a slot (mark for reuse)
    #[inline]
    pub fn free(&mut self, id: u32) {
        if let Some(slot) = self.storage.get_mut(id as usize) {
            if slot.is_some() {
                *slot = None;
                self.free_list.push(id);
                self.count -= 1;
            }
        }
    }

    /// Check if a slot is occupied
    #[inline(always)]
    pub fn is_valid(&self, id: u32) -> bool {
        self.storage.get(id as usize).map(|opt| opt.is_some()).unwrap_or(false)
    }

    /// Current number of live objects
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Iterate over all live objects
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.storage.iter().enumerate().filter_map(|(i, opt)| {
            opt.as_ref().map(|v| (i as u32, v))
        })
    }

    /// Iterate over all live objects mutably
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T)> {
        self.storage.iter_mut().enumerate().filter_map(|(i, opt)| {
            opt.as_mut().map(|v| (i as u32, v))
        })
    }

    /// Shrink internal storage
    pub fn shrink_to_fit(&mut self) {
        self.storage.shrink_to_fit();
        self.free_list.shrink_to_fit();
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        self.storage.clear();
        self.free_list.clear();
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
    
    // String interning table: hash -> StringId
    string_intern: HashMap<u64, StringId>,
    max_intern_length: usize,
}

impl ObjectPoolV2 {
    pub fn new() -> Self {
        Self {
            strings: Arena::with_capacity(256),
            tables: Arena::with_capacity(64),
            functions: Arena::with_capacity(32),
            upvalues: Arena::with_capacity(32),
            userdata: Arena::new(),
            string_intern: HashMap::with_capacity(256),
            max_intern_length: 64,  // Strings <= 64 bytes are interned
        }
    }

    // ==================== String Operations ====================

    /// Create or intern a string
    #[inline]
    pub fn create_string(&mut self, s: &str) -> StringId {
        let len = s.len();

        // Intern short strings for deduplication
        if len <= self.max_intern_length {
            let hash = self.hash_string(s);

            // Check if already interned
            if let Some(&id) = self.string_intern.get(&hash) {
                // Verify (hash collision possible)
                if let Some(gs) = self.strings.get(id.0) {
                    if gs.data.as_str() == s {
                        return id;
                    }
                }
            }

            // Create new interned string
            let gc_string = GcString {
                header: GcHeader::default(),
                data: LuaString::new(s.to_string()),
            };
            let id = StringId(self.strings.alloc(gc_string));
            self.string_intern.insert(hash, id);
            id
        } else {
            // Long strings are not interned
            let gc_string = GcString {
                header: GcHeader::default(),
                data: LuaString::new(s.to_string()),
            };
            StringId(self.strings.alloc(gc_string))
        }
    }

    /// Create string from owned String (avoids clone if not interned)
    #[inline]
    pub fn create_string_owned(&mut self, s: String) -> StringId {
        let len = s.len();

        if len <= self.max_intern_length {
            let hash = self.hash_string(&s);

            if let Some(&id) = self.string_intern.get(&hash) {
                if let Some(gs) = self.strings.get(id.0) {
                    if gs.data.as_str() == s {
                        return id;
                    }
                }
            }

            let gc_string = GcString {
                header: GcHeader::default(),
                data: LuaString::new(s),
            };
            let id = StringId(self.strings.alloc(gc_string));
            self.string_intern.insert(hash, id);
            id
        } else {
            let gc_string = GcString {
                header: GcHeader::default(),
                data: LuaString::new(s),
            };
            StringId(self.strings.alloc(gc_string))
        }
    }

    #[inline(always)]
    fn hash_string(&self, s: &str) -> u64 {
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

    #[inline(always)]
    pub fn get_table_mut(&mut self, id: TableId) -> Option<&mut LuaTable> {
        self.tables.get_mut(id.0).map(|gt| &mut gt.data)
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

    #[inline(always)]
    pub fn get_function_mut(&mut self, id: FunctionId) -> Option<&mut GcFunction> {
        self.functions.get_mut(id.0)
    }

    // ==================== Upvalue Operations ====================

    #[inline]
    pub fn create_upvalue_open(&mut self, frame_id: usize, register: usize) -> UpvalueId {
        let gc_uv = GcUpvalue {
            header: GcHeader::default(),
            state: UpvalueState::Open { frame_id, register },
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
    }

    /// Sweep phase: free all unmarked objects
    pub fn sweep(&mut self) {
        // Collect IDs to free (can't free while iterating)
        let strings_to_free: Vec<u32> = self.strings.iter()
            .filter(|(_, gs)| !gs.header.marked)
            .map(|(id, _)| id)
            .collect();
        let tables_to_free: Vec<u32> = self.tables.iter()
            .filter(|(_, gt)| !gt.header.marked)
            .map(|(id, _)| id)
            .collect();
        let functions_to_free: Vec<u32> = self.functions.iter()
            .filter(|(_, gf)| !gf.header.marked)
            .map(|(id, _)| id)
            .collect();
        let upvalues_to_free: Vec<u32> = self.upvalues.iter()
            .filter(|(_, gu)| !gu.header.marked)
            .map(|(id, _)| id)
            .collect();

        // Free collected IDs
        for id in strings_to_free {
            // Remove from intern table if interned
            // (optimization: could track which strings are interned)
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
    }

    pub fn shrink_to_fit(&mut self) {
        self.strings.shrink_to_fit();
        self.tables.shrink_to_fit();
        self.functions.shrink_to_fit();
        self.upvalues.shrink_to_fit();
        self.string_intern.shrink_to_fit();
    }

    // ==================== Statistics ====================

    #[inline]
    pub fn string_count(&self) -> usize { self.strings.len() }
    #[inline]
    pub fn table_count(&self) -> usize { self.tables.len() }
    #[inline]
    pub fn function_count(&self) -> usize { self.functions.len() }
    #[inline]
    pub fn upvalue_count(&self) -> usize { self.upvalues.len() }
    #[inline]
    pub fn userdata_count(&self) -> usize { self.userdata.len() }
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
            assert_eq!(table.raw_get(&LuaValue::integer(1)), Some(LuaValue::integer(42)));
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
}
