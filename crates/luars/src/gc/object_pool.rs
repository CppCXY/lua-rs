// Object Pool V3 - Simplified high-performance design
//
// Key Design Principles:
// 1. All IDs are u32 indices into Vec storage
// 2. Small objects (String, Function, Upvalue) use Vec<Option<T>>
// 3. Large objects (Table, Thread) use Vec<Option<Box<T>>> to avoid copy on resize
// 4. No chunking overhead - direct Vec indexing for O(1) access
// 5. Free list for slot reuse
// 6. GC headers embedded in objects for mark-sweep

use crate::gc::gc_object::{CachedUpvalue, FunctionBody};
use crate::lua_value::{Chunk, LuaUpvalue, LuaUserdata};
use crate::lua_vm::{CFunction, LuaState, TmKind};
use crate::{
    FunctionId, GcFunction, GcHeader, GcString, GcTable, GcThread, GcUpvalue, GcUserdata, LuaTable,
    LuaValue, StringId, TableId, ThreadId, Upvalue, UpvalueId, UserdataId,
};
use std::collections::HashMap;
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

// Keep Arena as an alias for backward compatibility
pub type Arena<T> = Pool<T>;

// ============ String Interner (Complete Interning) ============

/// Complete string interner - ALL strings are interned for maximum performance
/// - Same content always returns same StringId
/// - StringId equality = content equality (no string comparison needed)
/// - O(1) hash lookup for new strings
/// - GC can collect unused strings via mark-sweep
struct StringInterner {
    // StringId -> String data (Vec-based for O(1) access)
    strings: Pool<GcString>,

    // Content -> StringId mapping for deduplication
    // Key is (hash, start_idx) where start_idx is index into strings pool
    // This avoids storing string content twice
    map: HashMap<u64, Vec<u32>>, // hash -> list of StringIds with that hash
}

impl StringInterner {
    fn new() -> Self {
        Self {
            strings: Pool::with_capacity(256),
            map: HashMap::with_capacity(256),
        }
    }

    /// Intern a string - returns existing StringId if already interned, creates new otherwise
    fn intern(&mut self, s: &str) -> (LuaValue, bool) {
        self.intern_owned(s.to_string())
    }

    /// Intern an owned string (avoids clone)
    fn intern_owned(&mut self, s: String) -> (LuaValue, bool) {
        let hash = Self::hash_string(&s);

        // Check if already interned
        if let Some(ids) = self.map.get(&hash) {
            for &id in ids {
                if let Some(gs) = self.strings.get(id) {
                    if gs.data.as_str() == s.as_str() {
                        let str_id = StringId(id);
                        let ptr = gs.data.as_ref() as *const String;
                        // Found! Drop the owned string
                        return (LuaValue::string(str_id, ptr), false);
                    }
                }
            }
        }

        // Not found - use owned string directly
        let gc_string = GcString {
            header: GcHeader::default(),
            data: Box::new(s),
        };
        let ptr = gc_string.data.as_ref() as *const String;
        let id = self.strings.alloc(gc_string);
        let str_id = StringId(id);
        self.map.entry(hash).or_insert_with(Vec::new).push(id);

        (LuaValue::string(str_id, ptr), true)
    }

    /// Get string by ID
    #[inline(always)]
    fn get(&self, id: StringId) -> Option<&GcString> {
        self.strings.get(id.0)
    }

    #[inline(always)]
    fn get_mut(&mut self, id: StringId) -> Option<&mut GcString> {
        self.strings.get_mut(id.0)
    }

    /// Fast hash function - FNV-1a for good distribution
    #[inline(always)]
    fn hash_string(s: &str) -> u64 {
        let bytes = s.as_bytes();
        let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
        for &byte in bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3); // FNV prime
        }
        hash
    }

    /// Remove dead strings (called by GC)
    fn remove_dead(&mut self, id: StringId) {
        let hash = if let Some(gs) = self.strings.get(id.0) {
            Self::hash_string(&gs.data)
        } else {
            return; // Already removed
        };
        self.strings.free(id.0);

        // Remove from map
        if let Some(ids) = self.map.get_mut(&hash) {
            ids.retain(|&i| i != id.0);
            if ids.is_empty() {
                self.map.remove(&hash);
            }
        }
    }

    /// Iterate over all strings
    fn iter(&self) -> impl Iterator<Item = (u32, &GcString)> {
        self.strings.iter()
    }

    /// Iterate over all strings (mutable)
    fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut GcString)> {
        self.strings.iter_mut()
    }
}

// ============ Object Pool V3 ============

/// High-performance object pool for the Lua VM
/// - Small objects (String, Function, Upvalue) use Pool<T> with direct Vec storage
/// - Large objects (Table, Thread) use BoxPool<T> to avoid copy on resize
/// - ALL strings are interned via StringInterner for O(1) equality checks
pub struct ObjectPool {
    strings: StringInterner, // Private - use create_string() to intern
    pub tables: Pool<GcTable>,
    pub functions: Pool<GcFunction>,
    pub upvalues: Pool<GcUpvalue>,
    pub userdata: Pool<GcUserdata>,
    pub threads: Pool<GcThread>,

    // Pre-cached metamethod name StringIds (like Lua's G(L)->tmname[])
    // These are created at initialization and never collected
    // Stored as StringId to avoid repeated hash lookup in hot paths
    pub tm_index: LuaValue,     // "__index"
    pub tm_newindex: LuaValue,  // "__newindex"
    pub tm_call: LuaValue,      // "__call"
    pub tm_tostring: LuaValue,  // "__tostring"
    pub tm_len: LuaValue,       // "__len"
    pub tm_pairs: LuaValue,     // "__pairs"
    pub tm_ipairs: LuaValue,    // "__ipairs"
    pub tm_gc: LuaValue,        // "__gc"
    pub tm_close: LuaValue,     // "__close"
    pub tm_mode: LuaValue,      // "__mode"
    pub tm_name: LuaValue,      // "__name"
    pub tm_eq: LuaValue,        // "__eq"
    pub tm_lt: LuaValue,        // "__lt"
    pub tm_le: LuaValue,        // "__le"
    pub tm_add: LuaValue,       // "__add"
    pub tm_sub: LuaValue,       // "__sub"
    pub tm_mul: LuaValue,       // "__mul"
    pub tm_div: LuaValue,       // "__div"
    pub tm_mod: LuaValue,       // "__mod"
    pub tm_pow: LuaValue,       // "__pow"
    pub tm_unm: LuaValue,       // "__unm"
    pub tm_idiv: LuaValue,      // "__idiv"
    pub tm_band: LuaValue,      // "__band"
    pub tm_bor: LuaValue,       // "__bor"
    pub tm_bxor: LuaValue,      // "__bxor"
    pub tm_bnot: LuaValue,      // "__bnot"
    pub tm_shl: LuaValue,       // "__shl"
    pub tm_shr: LuaValue,       // "__shr"
    pub tm_concat: LuaValue,    // "__concat"
    pub tm_metatable: LuaValue, // "__metatable"

    // Pre-cached coroutine status strings for fast coroutine.status
    pub str_suspended: LuaValue, // "suspended"
    pub str_running: LuaValue,   // "running"
    pub str_normal: LuaValue,    // "normal"
    pub str_dead: LuaValue,      // "dead"
}

impl ObjectPool {
    pub fn new() -> Self {
        let mut pool = Self {
            strings: StringInterner::new(),
            tables: Pool::with_capacity(64),
            functions: Pool::with_capacity(32),
            upvalues: Pool::with_capacity(32),
            userdata: Pool::new(),
            threads: Pool::with_capacity(8),
            // Placeholder values - will be initialized below
            tm_index: LuaValue::nil(),
            tm_newindex: LuaValue::nil(),
            tm_call: LuaValue::nil(),
            tm_tostring: LuaValue::nil(),
            tm_len: LuaValue::nil(),
            tm_pairs: LuaValue::nil(),
            tm_ipairs: LuaValue::nil(),
            tm_gc: LuaValue::nil(),
            tm_close: LuaValue::nil(),
            tm_mode: LuaValue::nil(),
            tm_name: LuaValue::nil(),
            tm_eq: LuaValue::nil(),
            tm_lt: LuaValue::nil(),
            tm_le: LuaValue::nil(),
            tm_add: LuaValue::nil(),
            tm_sub: LuaValue::nil(),
            tm_mul: LuaValue::nil(),
            tm_div: LuaValue::nil(),
            tm_mod: LuaValue::nil(),
            tm_pow: LuaValue::nil(),
            tm_unm: LuaValue::nil(),
            tm_idiv: LuaValue::nil(),
            tm_band: LuaValue::nil(),
            tm_bor: LuaValue::nil(),
            tm_bxor: LuaValue::nil(),
            tm_bnot: LuaValue::nil(),
            tm_shl: LuaValue::nil(),
            tm_shr: LuaValue::nil(),
            tm_concat: LuaValue::nil(),
            tm_metatable: LuaValue::nil(),
            str_suspended: LuaValue::nil(),
            str_running: LuaValue::nil(),
            str_normal: LuaValue::nil(),
            str_dead: LuaValue::nil(),
        };

        // Pre-create all metamethod name strings (like Lua's luaT_init)
        // These strings are interned and will never be collected
        pool.tm_index = pool.create_string("__index").0;
        pool.tm_newindex = pool.create_string("__newindex").0;
        pool.tm_call = pool.create_string("__call").0;
        pool.tm_tostring = pool.create_string("__tostring").0;
        pool.tm_len = pool.create_string("__len").0;
        pool.tm_pairs = pool.create_string("__pairs").0;
        pool.tm_ipairs = pool.create_string("__ipairs").0;
        pool.tm_gc = pool.create_string("__gc").0;
        pool.tm_close = pool.create_string("__close").0;
        pool.tm_mode = pool.create_string("__mode").0;
        pool.tm_name = pool.create_string("__name").0;
        pool.tm_eq = pool.create_string("__eq").0;
        pool.tm_lt = pool.create_string("__lt").0;
        pool.tm_le = pool.create_string("__le").0;
        pool.tm_add = pool.create_string("__add").0;
        pool.tm_sub = pool.create_string("__sub").0;
        pool.tm_mul = pool.create_string("__mul").0;
        pool.tm_div = pool.create_string("__div").0;
        pool.tm_mod = pool.create_string("__mod").0;
        pool.tm_pow = pool.create_string("__pow").0;
        pool.tm_unm = pool.create_string("__unm").0;
        pool.tm_idiv = pool.create_string("__idiv").0;
        pool.tm_band = pool.create_string("__band").0;
        pool.tm_bor = pool.create_string("__bor").0;
        pool.tm_bxor = pool.create_string("__bxor").0;
        pool.tm_bnot = pool.create_string("__bnot").0;
        pool.tm_shl = pool.create_string("__shl").0;
        pool.tm_shr = pool.create_string("__shr").0;
        pool.tm_concat = pool.create_string("__concat").0;
        pool.tm_metatable = pool.create_string("__metatable").0;

        // Pre-create coroutine status strings
        pool.str_suspended = pool.create_string("suspended").0;
        pool.str_running = pool.create_string("running").0;
        pool.str_normal = pool.create_string("normal").0;
        pool.str_dead = pool.create_string("dead").0;

        // Fix all metamethod name strings - they should never be collected
        // (like Lua's luaC_fix in luaT_init)
        pool.fix_string(pool.tm_index.as_string_id().unwrap());
        pool.fix_string(pool.tm_newindex.as_string_id().unwrap());
        pool.fix_string(pool.tm_call.as_string_id().unwrap());
        pool.fix_string(pool.tm_tostring.as_string_id().unwrap());
        pool.fix_string(pool.tm_len.as_string_id().unwrap());
        pool.fix_string(pool.tm_pairs.as_string_id().unwrap());
        pool.fix_string(pool.tm_ipairs.as_string_id().unwrap());
        pool.fix_string(pool.tm_gc.as_string_id().unwrap());
        pool.fix_string(pool.tm_close.as_string_id().unwrap());
        pool.fix_string(pool.tm_mode.as_string_id().unwrap());
        pool.fix_string(pool.tm_name.as_string_id().unwrap());
        pool.fix_string(pool.tm_eq.as_string_id().unwrap());
        pool.fix_string(pool.tm_lt.as_string_id().unwrap());
        pool.fix_string(pool.tm_le.as_string_id().unwrap());
        pool.fix_string(pool.tm_add.as_string_id().unwrap());
        pool.fix_string(pool.tm_sub.as_string_id().unwrap());
        pool.fix_string(pool.tm_mul.as_string_id().unwrap());
        pool.fix_string(pool.tm_div.as_string_id().unwrap());
        pool.fix_string(pool.tm_mod.as_string_id().unwrap());
        pool.fix_string(pool.tm_pow.as_string_id().unwrap());
        pool.fix_string(pool.tm_unm.as_string_id().unwrap());
        pool.fix_string(pool.tm_idiv.as_string_id().unwrap());
        pool.fix_string(pool.tm_band.as_string_id().unwrap());
        pool.fix_string(pool.tm_bor.as_string_id().unwrap());
        pool.fix_string(pool.tm_bxor.as_string_id().unwrap());
        pool.fix_string(pool.tm_bnot.as_string_id().unwrap());
        pool.fix_string(pool.tm_shl.as_string_id().unwrap());
        pool.fix_string(pool.tm_shr.as_string_id().unwrap());
        pool.fix_string(pool.tm_concat.as_string_id().unwrap());
        pool.fix_string(pool.tm_metatable.as_string_id().unwrap());
        pool.fix_string(pool.str_suspended.as_string_id().unwrap());
        pool.fix_string(pool.str_running.as_string_id().unwrap());
        pool.fix_string(pool.str_normal.as_string_id().unwrap());
        pool.fix_string(pool.str_dead.as_string_id().unwrap());

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
    pub fn get_tm_value(&self, tm: TmKind) -> LuaValue {
        match tm {
            TmKind::Index => self.tm_index,
            TmKind::NewIndex => self.tm_newindex,
            TmKind::Gc => self.tm_gc,
            TmKind::Mode => self.tm_mode,
            TmKind::Len => self.tm_len,
            TmKind::Eq => self.tm_eq,
            TmKind::Add => self.tm_add,
            TmKind::Sub => self.tm_sub,
            TmKind::Mul => self.tm_mul,
            TmKind::Mod => self.tm_mod,
            TmKind::Pow => self.tm_pow,
            TmKind::Div => self.tm_div,
            TmKind::IDiv => self.tm_idiv,
            TmKind::Band => self.tm_band,
            TmKind::Bor => self.tm_bor,
            TmKind::Bxor => self.tm_bxor,
            TmKind::Shl => self.tm_shl,
            TmKind::Shr => self.tm_shr,
            TmKind::Unm => self.tm_unm,
            TmKind::Bnot => self.tm_bnot,
            TmKind::Lt => self.tm_lt,
            TmKind::Le => self.tm_le,
            TmKind::Concat => self.tm_concat,
            TmKind::Call => self.tm_call,
            TmKind::Close => self.tm_close,
            _ => self.tm_index, // Fallback to __index
        }
    }

    #[inline]
    pub fn get_tm_value_by_str(&self, tm_str: &str) -> LuaValue {
        match tm_str {
            "__index" => self.tm_index,
            "__newindex" => self.tm_newindex,
            "__gc" => self.tm_gc,
            "__mode" => self.tm_mode,
            "__len" => self.tm_len,
            "__eq" => self.tm_eq,
            "__add" => self.tm_add,
            "__sub" => self.tm_sub,
            "__mul" => self.tm_mul,
            "__mod" => self.tm_mod,
            "__pow" => self.tm_pow,
            "__div" => self.tm_div,
            "__idiv" => self.tm_idiv,
            "__band" => self.tm_band,
            "__bor" => self.tm_bor,
            "__bxor" => self.tm_bxor,
            "__shl" => self.tm_shl,
            "__shr" => self.tm_shr,
            "__unm" => self.tm_unm,
            "__bnot" => self.tm_bnot,
            "__lt" => self.tm_lt,
            "__le" => self.tm_le,
            "__concat" => self.tm_concat,
            "__call" => self.tm_call,
            "__close" => self.tm_close,
            "__tostring" => self.tm_tostring,
            _ => self.tm_index, // Fallback to __index
        }
    }

    // ==================== String Operations ====================

    /// Create or intern a string (Lua-style with proper hash collision handling)
    /// Returns (StringId, is_new) where is_new indicates if a new string was created
    #[inline]
    /// Create string (COMPLETE INTERNING - all strings)
    /// Returns (StringId, is_new) where is_new indicates if a new string was created
    pub fn create_string(&mut self, s: &str) -> (LuaValue, bool) {
        self.strings.intern(s)
    }

    /// Create string from owned String (avoids clone if already interned)
    /// Returns (StringId, is_new) where is_new indicates if a new string was created
    pub fn create_string_owned(&mut self, s: String) -> (LuaValue, bool) {
        self.strings.intern_owned(s)
    }

    #[inline(always)]
    pub fn get_string(&self, id: StringId) -> Option<&str> {
        self.strings.get(id).map(|gs| gs.data.as_ref().as_str())
    }

    #[inline(always)]
    pub fn get_string_value(&self, id: StringId) -> Option<LuaValue> {
        let gs = self.strings.get(id)?;
        let ptr = gs.data.as_ref() as *const String;
        Some(LuaValue::string(id, ptr))
    }

    #[inline(always)]
    pub(crate) fn get_string_gc_mut(&mut self, id: StringId) -> Option<&mut GcString> {
        self.strings.get_mut(id)
    }

    /// Create a substring from an existing string (optimized for string.sub)
    /// Returns the original string ID if the range covers the entire string.
    /// With complete interning, substrings are automatically deduplicated.
    #[inline]
    pub fn create_substring(&mut self, s_value: LuaValue, start: usize, end: usize) -> LuaValue {
        let string_id = match s_value.as_string_id() {
            Some(id) => id,
            None => return self.create_string("").0, // Not a string, return empty
        };
        // Extract substring info first
        let substring = {
            let Some(gs) = self.strings.get(string_id) else {
                return self.create_string("").0;
            };
            let s = gs.data.as_str();

            // Clamp indices
            let start = start.min(s.len());
            let end = end.min(s.len());

            if start >= end {
                return self.create_string("").0;
            }

            // Fast path: return original if full range
            if start == 0 && end == s.len() {
                return s_value;
            }

            // Copy substring to avoid borrowing issue
            s[start..end].to_string()
        };

        // Intern the substring - will be deduplicated if it already exists
        self.create_string_owned(substring).0
    }

    /// Mark a string as fixed (never collected) - like Lua's luaC_fix()
    /// Used for metamethod names and other permanent strings
    #[inline]
    pub fn fix_string(&mut self, id: StringId) {
        if let Some(gs) = self.strings.get_mut(id) {
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

    // ==================== Iteration (for GC) ====================

    /// Iterate over all strings (for GC marking/sweeping)
    pub fn iter_strings(&self) -> impl Iterator<Item = (u32, &GcString)> {
        self.strings.iter()
    }

    /// Iterate over all strings (mutable, for GC marking)
    pub fn iter_strings_mut(&mut self) -> impl Iterator<Item = (u32, &mut GcString)> {
        self.strings.iter_mut()
    }

    // ==================== Table Operations ====================

    #[inline]
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> LuaValue {
        let gc_table = GcTable {
            header: GcHeader::default(),
            data: Box::new(LuaTable::new(array_size as u32, hash_size as u32)),
        };
        let ptr = gc_table.data.as_ref() as *const LuaTable;
        let table_id = TableId(self.tables.alloc(gc_table));
        LuaValue::table(table_id, ptr)
    }

    #[inline]
    pub fn create_table_default(&mut self) -> TableId {
        let gc_table = GcTable {
            header: GcHeader::default(),
            data: Box::new(LuaTable::new(0, 0)),
        };
        TableId(self.tables.alloc(gc_table))
    }

    #[inline(always)]
    pub fn get_table(&self, id: TableId) -> Option<&LuaTable> {
        self.tables.get(id.0).map(|gt| gt.data.as_ref())
    }

    #[inline(always)]
    pub fn get_table_value(&self, id: TableId) -> Option<LuaValue> {
        let table = self.tables.get(id.0)?;
        let ptr = table.data.as_ref() as *const LuaTable;
        Some(LuaValue::table(id, ptr))
    }

    #[inline(always)]
    pub fn get_table_mut(&mut self, id: TableId) -> Option<&mut LuaTable> {
        self.tables.get_mut(id.0).map(|gt| gt.data.as_mut())
    }

    // ==================== Function Operations ====================

    /// Create a Lua function (closure with bytecode chunk)
    /// Now caches upvalue pointers for direct access
    #[inline]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalue_ids: Vec<UpvalueId>) -> LuaValue {
        // Build cached upvalues with direct pointers
        let cached_upvalues: Vec<CachedUpvalue> = upvalue_ids
            .into_iter()
            .map(|id| {
                let ptr = self
                    .upvalues
                    .get(id.0)
                    .map(|uv| uv.data.as_ref() as *const Upvalue)
                    .unwrap_or(std::ptr::null());
                CachedUpvalue::new(id, ptr)
            })
            .collect();

        let gc_func = GcFunction {
            header: GcHeader::default(),
            data: Box::new(FunctionBody::Lua(chunk, cached_upvalues)),
        };
        let ptr = gc_func.data.as_ref() as *const FunctionBody;
        let id = FunctionId(self.functions.alloc(gc_func));
        LuaValue::function(id, ptr)
    }

    /// Create a C closure (native function with upvalues)
    /// Now caches upvalue pointers for direct access
    #[inline]
    pub fn create_c_closure(&mut self, func: CFunction, upvalue_ids: Vec<UpvalueId>) -> LuaValue {
        // Build cached upvalues with direct pointers
        let cached_upvalues: Vec<CachedUpvalue> = upvalue_ids
            .into_iter()
            .map(|id| {
                let ptr = self
                    .upvalues
                    .get(id.0)
                    .map(|uv| uv.data.as_ref() as *const Upvalue)
                    .unwrap_or(std::ptr::null());
                CachedUpvalue::new(id, ptr)
            })
            .collect();

        let gc_func = GcFunction {
            header: GcHeader::default(),
            data: Box::new(FunctionBody::CClosure(func, cached_upvalues)),
        };
        let ptr = gc_func.data.as_ref() as *const FunctionBody;
        let id = FunctionId(self.functions.alloc(gc_func));
        LuaValue::function(id, ptr)
    }

    #[inline(always)]
    pub(crate) fn get_function(&self, id: FunctionId) -> Option<&GcFunction> {
        self.functions.get(id.0)
    }

    // ==================== Upvalue Operations ====================

    /// Create an open upvalue pointing to a stack location
    #[inline]
    pub fn create_upvalue_open(
        &mut self,
        stack_index: usize,
        thread: *const LuaState,
    ) -> UpvalueId {
        let gc_uv = GcUpvalue {
            header: GcHeader::default(),
            data: Box::new(Upvalue {
                stack_index,
                closed_value: LuaValue::nil(),
                is_open: true,
                thread,
            }),
        };
        UpvalueId(self.upvalues.alloc(gc_uv))
    }

    /// Create a closed upvalue with a value
    #[inline]
    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> UpvalueId {
        let gc_uv = GcUpvalue {
            header: GcHeader::default(),
            data: Box::new(Upvalue {
                stack_index: 0,
                closed_value: value,
                is_open: false,
                thread: std::ptr::null(),
            }),
        };
        UpvalueId(self.upvalues.alloc(gc_uv))
    }

    #[inline(always)]
    pub(crate) fn get_upvalue(&self, id: UpvalueId) -> Option<&GcUpvalue> {
        self.upvalues.get(id.0)
    }

    #[inline(always)]
    pub(crate) fn get_upvalue_mut(&mut self, id: UpvalueId) -> Option<&mut GcUpvalue> {
        self.upvalues.get_mut(id.0)
    }

    /// Iterator over all upvalues
    pub fn iter_upvalues(&self) -> impl Iterator<Item = (UpvalueId, &GcUpvalue)> {
        self.upvalues
            .iter()
            .map(|(idx, upval)| (UpvalueId(idx), upval))
    }

    /// Create upvalue from LuaUpvalue
    pub fn create_upvalue(&mut self, upvalue: Rc<LuaUpvalue>) -> UpvalueId {
        // Check if open and get stack index
        let (is_open, stack_index, closed_value) = if upvalue.is_open() {
            (
                true,
                upvalue.get_stack_index().unwrap_or(0),
                LuaValue::nil(),
            )
        } else {
            (
                false,
                0,
                upvalue.get_closed_value().unwrap_or(LuaValue::nil()),
            )
        };

        let gc_uv = GcUpvalue {
            header: GcHeader::default(),
            data: Box::new(Upvalue {
                stack_index,
                closed_value,
                is_open,
                thread: std::ptr::null(),
            }),
        };
        UpvalueId(self.upvalues.alloc(gc_uv))
    }

    // ==================== Userdata Operations ====================

    #[inline]
    pub fn create_userdata(&mut self, userdata: LuaUserdata) -> LuaValue {
        let gc_userdata = GcUserdata {
            header: GcHeader::default(),
            data: Box::new(userdata),
        };
        let ptr = gc_userdata.data.as_ref() as *const LuaUserdata;
        let id = UserdataId(self.userdata.alloc(gc_userdata));
        LuaValue::userdata(id, ptr)
    }

    #[inline(always)]
    pub fn get_userdata_mut(&mut self, id: UserdataId) -> Option<&mut GcUserdata> {
        self.userdata.get_mut(id.0)
    }

    // ==================== Thread Operations ====================

    #[inline]
    pub fn create_thread(&mut self, thread: LuaState) -> LuaValue {
        let box_thread = Box::new(thread);
        let gc_thread = GcThread {
            header: GcHeader::default(),
            data: box_thread,
        };
        let ptr = gc_thread.data.as_ref() as *const LuaState;
        let id = ThreadId(self.threads.alloc(gc_thread));
        let l = self.get_thread_mut(id).unwrap();
        l.set_thread_id(id);

        LuaValue::thread(id, ptr)
    }

    #[inline(always)]
    pub fn get_thread_value(&self, id: ThreadId) -> Option<LuaValue> {
        let thread = self.threads.get(id.0)?;
        let ptr = thread.data.as_ref() as *const LuaState;
        Some(LuaValue::thread(id, ptr))
    }

    #[inline(always)]
    pub fn get_thread_mut(&mut self, id: ThreadId) -> Option<&mut LuaState> {
        self.threads.get_mut(id.0).map(|gt| gt.data.as_mut())
    }
    // ==================== GC Support ====================

    /// Clear all mark bits before GC mark phase (make all objects white)
    pub fn clear_marks(&mut self) {
        for (_, gs) in self.iter_strings_mut() {
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
            .iter_strings()
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
            // Remove from intern map - StringInterner handles this
            self.strings.remove_dead(StringId(id));
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
        // StringInterner manages its own internal structures
        self.tables.shrink_to_fit();
        self.functions.shrink_to_fit();
        self.upvalues.shrink_to_fit();
        self.threads.shrink_to_fit();
    }

    // ==================== Remove Operations (for GC) ====================

    #[inline]
    pub fn remove_string(&mut self, id: StringId) {
        self.strings.remove_dead(id);
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
        self.strings.strings.len()
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

        let v1 = pool.create_string("hello").0;
        let v2 = pool.create_string("hello").0;

        // Same string should return same ID
        assert_eq!(v1, v2);
        let v3 = pool.create_string("world").0;
        assert_ne!(v1, v3);

        // Verify content
        assert_eq!(v1.as_str(), Some("hello"));
        assert_eq!(v3.as_str(), Some("world"));
    }

    #[test]
    fn test_table_operations() {
        let mut pool = ObjectPool::new();

        let table_value = pool.create_table(4, 4);
        let table_id = table_value.as_table_id().unwrap();
        // Modify table
        if let Some(table) = pool.get_table_mut(table_id) {
            table.raw_set(&LuaValue::integer(1), LuaValue::integer(42));
        }

        // Read back
        if let Some(table) = pool.get_table(table_id) {
            assert!(table.raw_get(&LuaValue::integer(1)) == Some(LuaValue::integer(42)));
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
            let id = pool.create_string(&s).0;
            ids.push((s, id));
        }

        // Verify all strings are stored correctly
        for (s, id) in &ids {
            let stored = id.as_str();
            assert_eq!(
                stored,
                Some(s.as_str()),
                "String '{}' not stored correctly",
                s
            );
        }

        // Verify interning works - same string should return same ID
        for (s, id) in &ids {
            let id2 = pool.create_string(s).0;
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
            ids.push(pool.create_string(s).0);
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
            assert_eq!(ids[i].as_str(), Some(*s));
        }
    }
}
