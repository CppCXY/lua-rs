// Object Pool Architecture for Lua VM
// Unified management for String, Table, and Userdata using ID-based indexing
// This avoids reference counting and allows proper GC integration

use crate::lua_value::{self, LuaUserdata};
use crate::{LuaFunction, LuaString, LuaTable};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Slot-based storage with free list for O(1) allocation and deallocation
struct SlotVec<T> {
    slots: Vec<Option<T>>,
    free_list: Vec<u32>,
    count: usize,
}

#[allow(unused)]
impl<T> SlotVec<T> {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
            count: 0,
        }
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            free_list: Vec::with_capacity(capacity / 4),
            count: 0,
        }
    }

    /// O(1) insertion - reuse free slot or append new slot
    #[inline]
    fn insert(&mut self, value: T) -> u32 {
        self.count += 1;
        
        if let Some(free_id) = self.free_list.pop() {
            self.slots[free_id as usize] = Some(value);
            free_id
        } else {
            let id = self.slots.len() as u32;
            self.slots.push(Some(value));
            id
        }
    }

    /// O(1) lookup - direct array indexing
    #[inline]
    fn get(&self, id: u32) -> Option<&T> {
        self.slots.get(id as usize).and_then(|slot| slot.as_ref())
    }

    /// O(1) removal - mark as free and add to free list
    #[inline]
    fn remove(&mut self, id: u32) -> Option<T> {
        if let Some(slot) = self.slots.get_mut(id as usize) {
            if let Some(value) = slot.take() {
                self.free_list.push(id);
                self.count -= 1;
                return Some(value);
            }
        }
        None
    }

    #[inline]
    fn len(&self) -> usize {
        self.count
    }

    /// Shrink memory after GC
    fn shrink_to_fit(&mut self) {
        if self.free_list.len() < self.slots.len() / 4 {
            self.free_list.shrink_to_fit();
            return;
        }

        while let Some(None) = self.slots.last() {
            let removed_id = self.slots.len() - 1;
            self.slots.pop();
            
            if let Some(pos) = self.free_list.iter().rposition(|&id| id as usize == removed_id) {
                self.free_list.swap_remove(pos);
            }
        }
        
        self.slots.shrink_to_fit();
        self.free_list.shrink_to_fit();
    }
}

/// Object IDs - u32 is enough for most use cases (4 billion objects)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StringId(pub u32);

impl StringId {
    pub fn to_u32(self) -> u32 {
        self.0
    }

    pub fn next(self) -> Self {
        StringId(self.0 + 1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TableId(pub u32);

impl TableId {
    pub fn to_u32(self) -> u32 {
        self.0
    }

    pub fn next(self) -> Self {
        TableId(self.0 + 1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UserdataId(pub u32);

impl UserdataId {
    pub fn to_u32(self) -> u32 {
        self.0
    }

    pub fn next(self) -> Self {
        UserdataId(self.0 + 1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FunctionId(pub u32);

impl FunctionId {
    pub fn to_u32(self) -> u32 {
        self.0
    }

    pub fn next(self) -> Self {
        FunctionId(self.0 + 1)
    }
}

/// Object Pool for all heap-allocated Lua objects
pub struct ObjectPool {
    strings: SlotVec<Rc<LuaString>>,
    tables: SlotVec<Rc<RefCell<LuaTable>>>,
    userdata: SlotVec<Rc<RefCell<LuaUserdata>>>,
    functions: SlotVec<Rc<RefCell<lua_value::LuaFunction>>>,

    // String interning table (hash -> id mapping)
    // For strings ≤ 64 bytes, we intern them for memory efficiency
    string_intern: HashMap<u64, StringId>,
    max_intern_length: usize,
}

impl ObjectPool {
    pub fn new() -> Self {
        ObjectPool {
            strings: SlotVec::with_capacity(128),
            tables: SlotVec::with_capacity(16),
            userdata: SlotVec::with_capacity(0),
            functions: SlotVec::with_capacity(64),
            string_intern: HashMap::with_capacity(128),
            max_intern_length: 64,
        }
    }

    // ============ String Operations ============

    /// Create or intern a string
    pub fn create_string(&mut self, s: &str) -> StringId {
        let len = s.len();

        // Intern short strings
        if len <= self.max_intern_length {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            s.hash(&mut hasher);
            let hash = hasher.finish();

            // Check intern table
            if let Some(&id) = self.string_intern.get(&hash) {
                // Verify content (hash collision check)
                if let Some(existing) = self.strings.get(id.0) {
                    if existing.as_str() == s {
                        return id;
                    }
                }
            }

            // Create new interned string
            let lua_string = Rc::new(LuaString::new(s.to_string()));
            let slot_id = self.strings.insert(lua_string);
            let id = StringId(slot_id);
            self.string_intern.insert(hash, id);

            id
        } else {
            // Long string - no interning
            let lua_string = Rc::new(LuaString::new(s.to_string()));
            let slot_id = self.strings.insert(lua_string);
            StringId(slot_id)
        }
    }

    /// Get string by ID
    #[inline]
    pub fn get_string(&self, id: StringId) -> Option<&Rc<LuaString>> {
        self.strings.get(id.0)
    }

    /// Remove string (called by GC)
    pub fn remove_string(&mut self, id: StringId) -> Option<Rc<LuaString>> {
        if let Some(string) = self.strings.remove(id.0) {
            // Also remove from intern table if present
            if string.as_str().len() <= self.max_intern_length {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};

                let mut hasher = DefaultHasher::new();
                string.as_str().hash(&mut hasher);
                let hash = hasher.finish();

                if let Some(&intern_id) = self.string_intern.get(&hash) {
                    if intern_id == id {
                        self.string_intern.remove(&hash);
                    }
                }
            }
            Some(string)
        } else {
            None
        }
    }

    // ============ Table Operations ============

    /// Create a new table
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> TableId {
        let table = Rc::new(RefCell::new(LuaTable::new(array_size, hash_size)));
        let slot_id = self.tables.insert(table);
        TableId(slot_id)
    }

    /// Get table by ID
    #[inline]
    pub fn get_table(&self, id: TableId) -> Option<&Rc<RefCell<LuaTable>>> {
        self.tables.get(id.0)
    }

    /// Remove table (called by GC)
    pub fn remove_table(&mut self, id: TableId) -> Option<Rc<RefCell<LuaTable>>> {
        self.tables.remove(id.0)
    }

    // ============ Userdata Operations ============

    /// Create new userdata
    pub fn create_userdata(&mut self, data: LuaUserdata) -> UserdataId {
        let slot_id = self.userdata.insert(Rc::new(RefCell::new(data)));
        UserdataId(slot_id)
    }

    /// Get userdata by ID
    #[inline]
    pub fn get_userdata(&self, id: UserdataId) -> Option<&Rc<RefCell<LuaUserdata>>> {
        self.userdata.get(id.0)
    }

    /// Get mutable userdata by ID (actually returns &Rc<RefCell<>> - mutate via borrow_mut)
    #[inline]
    pub fn get_userdata_mut(&mut self, id: UserdataId) -> Option<&Rc<RefCell<LuaUserdata>>> {
        self.userdata.get(id.0)
    }

    /// Remove userdata (called by GC)
    pub fn remove_userdata(&mut self, id: UserdataId) -> Option<Rc<RefCell<LuaUserdata>>> {
        self.userdata.remove(id.0)
    }

    // ============ Function Operations ============

    /// Create a new function
    pub fn create_function(&mut self, func: LuaFunction) -> FunctionId {
        let slot_id = self.functions.insert(Rc::new(RefCell::new(func)));
        FunctionId(slot_id)
    }

    /// Get function by ID
    #[inline]
    pub fn get_function(&self, id: FunctionId) -> Option<&std::rc::Rc<RefCell<LuaFunction>>> {
        self.functions.get(id.0)
    }

    /// Remove function (called by GC)
    pub fn remove_function(&mut self, id: FunctionId) -> Option<std::rc::Rc<RefCell<LuaFunction>>> {
        self.functions.remove(id.0)
    }

    // ============ Statistics ============

    pub fn string_count(&self) -> usize {
        self.strings.len()
    }

    pub fn table_count(&self) -> usize {
        self.tables.len()
    }

    pub fn userdata_count(&self) -> usize {
        self.userdata.len()
    }

    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    pub fn interned_string_count(&self) -> usize {
        self.string_intern.len()
    }

    // ============ GC Support ============

    /// Shrink all hash maps to fit actual size (called after GC)
    /// This reclaims memory from deleted entries and improves lookup performance
    pub fn shrink_to_fit(&mut self) {
        self.strings.shrink_to_fit();
        self.tables.shrink_to_fit();
        self.userdata.shrink_to_fit();
        self.functions.shrink_to_fit();
        self.string_intern.shrink_to_fit();
    }
}

impl Default for ObjectPool {
    fn default() -> Self {
        Self::new()
    }
}
