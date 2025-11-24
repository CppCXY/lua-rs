// Object Pool Architecture for Lua VM
// Unified management for String, Table, and Userdata using ID-based indexing
// This avoids reference counting and allows proper GC integration

use crate::lua_value::{self, LuaUserdata};
use crate::{LuaFunction, LuaString, LuaTable};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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
    // Object storage - 所有类型都用 Rc 包装以防止 HashMap rehash 导致指针失效
    strings: HashMap<StringId, Rc<LuaString>>,
    tables: HashMap<TableId, Rc<RefCell<LuaTable>>>,
    userdata: HashMap<UserdataId, Rc<RefCell<LuaUserdata>>>,
    functions: HashMap<FunctionId, Rc<RefCell<lua_value::LuaFunction>>>,

    // ID generators
    next_string_id: StringId,
    next_table_id: TableId,
    next_userdata_id: UserdataId,
    next_function_id: FunctionId,

    // String interning table (hash -> id mapping)
    // For strings ≤ 64 bytes, we intern them for memory efficiency
    string_intern: HashMap<u64, StringId>,
    max_intern_length: usize,
}

impl ObjectPool {
    pub fn new() -> Self {
        ObjectPool {
            strings: HashMap::with_capacity(128),
            tables: HashMap::with_capacity(16),
            userdata: HashMap::with_capacity(0),
            functions: HashMap::with_capacity(64),
            next_string_id: StringId(1), // 0 reserved for null/invalid
            next_table_id: TableId(1),
            next_userdata_id: UserdataId(1),
            next_function_id: FunctionId(1),
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
                if let Some(existing) = self.strings.get(&id) {
                    if existing.as_str() == s {
                        return id;
                    }
                }
            }

            // Create new interned string
            let id = self.next_string_id;
            self.next_string_id = id.next();

            let lua_string = Rc::new(LuaString::new(s.to_string()));
            self.strings.insert(id, lua_string);
            self.string_intern.insert(hash, id);

            id
        } else {
            // Long string - no interning
            let id = self.next_string_id;
            self.next_string_id = id.next();

            let lua_string = Rc::new(LuaString::new(s.to_string()));
            self.strings.insert(id, lua_string);

            id
        }
    }

    /// Get string by ID
    #[inline]
    pub fn get_string(&self, id: StringId) -> Option<&Rc<LuaString>> {
        self.strings.get(&id)
    }

    /// Remove string (called by GC)
    pub fn remove_string(&mut self, id: StringId) -> Option<Rc<LuaString>> {
        if let Some(string) = self.strings.remove(&id) {
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
    pub fn create_table(&mut self) -> TableId {
        let id = self.next_table_id;
        self.next_table_id = id.next();

        self.tables
            .insert(id, Rc::new(RefCell::new(LuaTable::new())));

        id
    }

    /// Get table by ID
    #[inline]
    pub fn get_table(&self, id: TableId) -> Option<&Rc<RefCell<LuaTable>>> {
        self.tables.get(&id)
    }

    /// Remove table (called by GC)
    pub fn remove_table(&mut self, id: TableId) -> Option<Rc<RefCell<LuaTable>>> {
        self.tables.remove(&id)
    }

    // ============ Userdata Operations ============

    /// Create new userdata
    pub fn create_userdata(&mut self, data: LuaUserdata) -> UserdataId {
        let id = self.next_userdata_id;
        self.next_userdata_id = id.next();

        self.userdata.insert(id, Rc::new(RefCell::new(data)));

        id
    }

    /// Get userdata by ID
    #[inline]
    pub fn get_userdata(&self, id: UserdataId) -> Option<&Rc<RefCell<LuaUserdata>>> {
        self.userdata.get(&id)
    }

    /// Get mutable userdata by ID (actually returns &Rc<RefCell<>> - mutate via borrow_mut)
    #[inline]
    pub fn get_userdata_mut(&mut self, id: UserdataId) -> Option<&Rc<RefCell<LuaUserdata>>> {
        self.userdata.get(&id)
    }

    /// Remove userdata (called by GC)
    pub fn remove_userdata(&mut self, id: UserdataId) -> Option<Rc<RefCell<LuaUserdata>>> {
        self.userdata.remove(&id)
    }

    // ============ Function Operations ============

    /// Create a new function
    pub fn create_function(&mut self, func: LuaFunction) -> FunctionId {
        let id = self.next_function_id;
        self.next_function_id = id.next();

        self.functions
            .insert(id, std::rc::Rc::new(RefCell::new(func)));
        id
    }

    /// Get function by ID
    #[inline]
    pub fn get_function(&self, id: FunctionId) -> Option<&std::rc::Rc<RefCell<LuaFunction>>> {
        self.functions.get(&id)
    }

    /// Remove function (called by GC)
    pub fn remove_function(&mut self, id: FunctionId) -> Option<std::rc::Rc<RefCell<LuaFunction>>> {
        self.functions.remove(&id)
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
