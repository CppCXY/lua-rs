// ============ GC Header ============

use std::rc::Rc;

use crate::{
    Chunk, LuaTable, LuaValue, UpvalueId,
    lua_value::LuaUserdata,
    lua_vm::{CFunction, LuaState},
};

/// Cached upvalue - stores both ID and direct pointer for fast access
/// Mimics Lua C's cl->upvals[i] which is a direct pointer to UpValue
#[derive(Clone, Copy, Debug)]
pub struct CachedUpvalue {
    pub id: UpvalueId,
    /// Direct pointer to the Upvalue object for fast access
    /// SAFETY: This pointer is valid as long as the Upvalue exists in ObjectPool
    /// The Upvalue is kept alive by the GC as long as this function exists
    pub ptr: *const Upvalue,
}

unsafe impl Send for CachedUpvalue {}
unsafe impl Sync for CachedUpvalue {}

impl CachedUpvalue {
    #[inline(always)]
    pub fn new(id: UpvalueId, ptr: *const Upvalue) -> Self {
        Self { id, ptr }
    }

    /// Get the upvalue value directly through the cached pointer
    /// SAFETY: Caller must ensure the pointer is still valid
    /// current_thread: The thread attempting to read the upvalue (used for stack access optimization)
    #[inline(always)]
    pub fn get_value(&self, l: &LuaState) -> LuaValue {
        unsafe {
            let upval = &*self.ptr;
            match upval {
                Upvalue::Open(stack_index) => {
                    // Directly access the stack value
                    l.stack()[*stack_index].clone()
                }
                Upvalue::Closed(val) => val.clone(),
            }
        }
    }

    #[inline(always)]
    pub fn set_value(&self, l: &mut LuaState, value: LuaValue) {
        unsafe {
            let upval = &mut *(self.ptr as *mut Upvalue);
            match upval {
                Upvalue::Open(stack_index) => {
                    // Set value directly on the stack
                    l.stack_mut()[*stack_index] = value;
                }
                Upvalue::Closed(val) => {
                    // Update closed value
                    *val = value;
                }
            }
        }
    }
}

// Object ages for generational GC (like Lua 5.4)
// Uses 3 bits (0-7)
pub const G_NEW: u8 = 0; // Created in current cycle
pub const G_SURVIVAL: u8 = 1; // Created in previous cycle (survived one minor)
pub const G_OLD0: u8 = 2; // Marked old by forward barrier in this cycle
pub const G_OLD1: u8 = 3; // First full cycle as old
pub const G_OLD: u8 = 4; // Really old object (not to be visited in minor)
pub const G_TOUCHED1: u8 = 5; // Old object touched this cycle
pub const G_TOUCHED2: u8 = 6; // Old object touched in previous cycle

// Color bits
pub const WHITE0BIT: u8 = 3; // Object is white (type 0)
pub const WHITE1BIT: u8 = 4; // Object is white (type 1)
pub const BLACKBIT: u8 = 5; // Object is black
pub const FIXEDBIT: u8 = 6; // Object is fixed (never collected)

pub const WHITEBITS: u8 = (1 << WHITE0BIT) | (1 << WHITE1BIT);
pub const AGEBITS: u8 = 0x07; // Bits 0-2 for age

/// GC object header - embedded in every GC-managed object
/// Based on Lua 5.4's CommonHeader design
///
/// Bit layout of `marked` field:
/// - Bits 0-2: Age (G_NEW, G_SURVIVAL, G_OLD0, G_OLD1, G_OLD, G_TOUCHED1, G_TOUCHED2)
/// - Bit 3: WHITE0 (current white in even cycles)
/// - Bit 4: WHITE1 (current white in odd cycles)  
/// - Bit 5: BLACK (fully marked)
/// - Bit 6: FIXED (never collected)
/// - Bit 7: Reserved
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct GcHeader {
    pub marked: u8, // Color and age bits combined
}

impl Default for GcHeader {
    fn default() -> Self {
        // Temp default - should be overridden with current_white immediately
        // after creation (see GcHeader::new() for proper initialization)
        GcHeader {
            marked: G_NEW, // Neutral initial state
        }
    }
}

impl GcHeader {
    /// Create a new header with given white bit and age
    #[inline(always)]
    pub fn new(current_white: u8) -> Self {
        GcHeader {
            marked: (1 << (WHITE0BIT + current_white)) | G_NEW,
        }
    }

    /// Get object age
    #[inline(always)]
    pub fn age(&self) -> u8 {
        self.marked & AGEBITS
    }

    /// Set object age
    #[inline(always)]
    pub fn set_age(&mut self, age: u8) {
        self.marked = (self.marked & !AGEBITS) | (age & AGEBITS);
    }

    /// Check if object is white (either white0 or white1)
    #[inline(always)]
    pub fn is_white(&self) -> bool {
        (self.marked & WHITEBITS) != 0
    }

    /// Check if object is black
    #[inline(always)]
    pub fn is_black(&self) -> bool {
        (self.marked & (1 << BLACKBIT)) != 0
    }

    /// Check if object is gray (neither white nor black)
    #[inline(always)]
    pub fn is_gray(&self) -> bool {
        (self.marked & (WHITEBITS | (1 << BLACKBIT))) == 0
    }

    /// Check if object is fixed (never collected)
    #[inline(always)]
    pub fn is_fixed(&self) -> bool {
        (self.marked & (1 << FIXEDBIT)) != 0
    }

    /// Set object as fixed
    #[inline(always)]
    pub fn set_fixed(&mut self) {
        self.marked |= 1 << FIXEDBIT;
    }

    /// Check if object is old (age > G_SURVIVAL)
    #[inline(always)]
    pub fn is_old(&self) -> bool {
        self.age() > G_SURVIVAL
    }

    /// Make object white with given current_white (0 or 1)
    /// For new objects, use with is_new=true to set G_NEW age
    #[inline(always)]
    pub fn make_white(&mut self, current_white: u8) {
        // For new objects, luaC_white(g) just returns (currentwhite & WHITEBITS)
        // which means setting only the white bit without age bits
        // According to lgc.h, new objects are created with white color only
        self.marked = (1 << (WHITE0BIT + current_white)) | G_NEW;
    }

    /// Make object gray (clear all color bits)
    #[inline(always)]
    pub fn make_gray(&mut self) {
        self.marked &= !(WHITEBITS | (1 << BLACKBIT));
    }

    /// Make object black (from non-white state)
    #[inline(always)]
    pub fn make_black(&mut self) {
        self.marked = (self.marked & !WHITEBITS) | (1 << BLACKBIT);
    }

    /// Check if object is dead (has the "other" white)
    #[inline(always)]
    pub fn is_dead(&self, other_white: u8) -> bool {
        (self.marked & (1 << (WHITE0BIT + other_white))) != 0
    }

    /// Check if object has a specific white bit set
    #[inline(always)]
    pub fn has_white_bit(&self, white_bit: u8) -> bool {
        (self.marked & (1 << (WHITE0BIT + white_bit))) != 0
    }

    /// Make object OLD0 (first old generation)
    #[inline(always)]
    pub fn make_old0(&mut self) {
        self.set_age(G_OLD0);
    }

    /// Make object TOUCHED1 (old object modified in this cycle)
    #[inline(always)]
    pub fn make_touched1(&mut self) {
        self.set_age(G_TOUCHED1);
    }

    /// Make object OLD (fully old)
    #[inline(always)]
    pub fn make_old(&mut self) {
        self.set_age(G_OLD);
    }

    // Legacy compatibility
    #[inline(always)]
    pub fn is_marked(&self) -> bool {
        !self.is_white()
    }

    #[inline(always)]
    pub fn set_marked(&mut self, marked: bool) {
        if marked {
            self.make_black();
        } else {
            self.make_white(0);
        }
    }
}

// ============ GC-managed Objects ============
pub enum GcPtrObject {
    String(Box<String>),
    Table(Box<LuaTable>),
    Function(Box<FunctionBody>),
    Upvalue(Box<Upvalue>),
    Thread(Box<LuaState>),
    Userdata(Box<LuaUserdata>),
    Binary(Box<Vec<u8>>),
}

impl GcPtrObject {
    /// Get type tag of this object
    #[inline(always)]
    pub fn as_str_ptr(&self) -> Option<*const String> {
        match self {
            GcPtrObject::String(s) => Some(s.as_ref() as *const String),
            _ => None,
        }
    }

    pub fn as_table_ptr(&self) -> Option<*const LuaTable> {
        match self {
            GcPtrObject::Table(t) => Some(t.as_ref() as *const LuaTable),
            _ => None,
        }
    }

    pub fn as_function_ptr(&self) -> Option<*const FunctionBody> {
        match self {
            GcPtrObject::Function(f) => Some(f.as_ref() as *const FunctionBody),
            _ => None,
        }
    }

    pub fn as_upvalue_ptr(&self) -> Option<*const Upvalue> {
        match self {
            GcPtrObject::Upvalue(u) => Some(u.as_ref() as *const Upvalue),
            _ => None,
        }
    }

    pub fn as_thread_ptr(&self) -> Option<*const LuaState> {
        match self {
            GcPtrObject::Thread(t) => Some(t.as_ref() as *const LuaState),
            _ => None,
        }
    }

    pub fn as_userdata_ptr(&self) -> Option<*const LuaUserdata> {
        match self {
            GcPtrObject::Userdata(u) => Some(u.as_ref() as *const LuaUserdata),
            _ => None,
        }
    }

    pub fn as_binary_ptr(&self) -> Option<*const Vec<u8>> {
        match self {
            GcPtrObject::Binary(b) => Some(b.as_ref() as *const Vec<u8>),
            _ => None,
        }
    }
}

pub struct GcObject {
    pub header: GcHeader,
    pub ptr: GcPtrObject,
}

impl GcObject {
    /// Create a new GC object with default header
    pub fn new(ptr: GcPtrObject) -> Self {
        GcObject {
            header: GcHeader::default(),
            ptr,
        }
    }
}

/// Function body - either Lua bytecode or C function
pub enum FunctionBody {
    /// Lua function with bytecode chunk
    /// Now includes cached upvalue pointers for direct access (zero-overhead like Lua C)
    Lua(Rc<Chunk>, Vec<CachedUpvalue>),
    /// C function (native Rust function) with cached upvalues
    CClosure(CFunction, Vec<CachedUpvalue>),
}

impl FunctionBody {
    /// Check if this is a C function (any C variant)
    #[inline(always)]
    pub fn is_c_function(&self) -> bool {
        matches!(self, FunctionBody::CClosure(_, _))
    }

    /// Check if this is a Lua function
    #[inline(always)]
    pub fn is_lua_function(&self) -> bool {
        matches!(self, FunctionBody::Lua(_, _))
    }

    /// Get the chunk if this is a Lua function
    #[inline(always)]
    pub fn chunk(&self) -> Option<&Rc<Chunk>> {
        match &self {
            FunctionBody::Lua(chunk, _) => Some(chunk),
            _ => None,
        }
    }

    /// Get the C function pointer if this is any C function variant
    #[inline(always)]
    pub fn c_function(&self) -> Option<CFunction> {
        match &self {
            FunctionBody::CClosure(f, _) => Some(*f),
            FunctionBody::Lua(_, _) => None,
        }
    }

    /// Get cached upvalues (direct pointers for fast access)
    #[inline(always)]
    pub fn cached_upvalues(&self) -> &Vec<CachedUpvalue> {
        match &self {
            FunctionBody::CClosure(_, uv) => uv,
            FunctionBody::Lua(_, uv) => uv,
        }
    }
    
    /// Get mutable access to cached upvalues for updating pointers
    #[inline(always)]
    pub fn cached_upvalues_mut(&mut self) -> &mut Vec<CachedUpvalue> {
        match self {
            FunctionBody::CClosure(_, uv) => uv,
            FunctionBody::Lua(_, uv) => uv,
        }
    }
}

/// Upvalue with embedded GC header
///
/// Mimics Lua 5.5's UpVal structure with v.p pointer optimization:
/// - v.p always points to the actual value location (stack or u.value)
/// - When open: v.p points to stack[stack_index]
/// - When closed: v.p points to closed_value
///
/// This eliminates the branch in get/set operations, matching Lua C performance
pub enum Upvalue {
    Open(usize),
    Closed(LuaValue),
}

impl Upvalue {
    /// Check if this upvalue points to the given absolute stack index
    #[inline]
    pub fn points_to_index(&self, index: usize) -> bool {
        match self {
            Upvalue::Open(i) => *i == index,
            Upvalue::Closed(_) => false,
        }
    }

    /// Check if this upvalue is open (still points to stack)
    #[inline]
    pub fn is_open(&self) -> bool {
        matches!(self, Upvalue::Open(_))
    }

    /// Close this upvalue with the given value
    #[inline]
    pub fn close(&mut self, value: LuaValue) {
        *self = Upvalue::Closed(value);
    }

    /// Get the value of a closed upvalue (returns None if still open)
    #[inline]
    pub fn get_closed_value(&self) -> Option<LuaValue> {
        match self {
            Upvalue::Closed(val) => Some(val.clone()),
            Upvalue::Open(_) => None,
        }
    }

    /// Get the absolute stack index if this upvalue is open
    #[inline]
    pub fn get_stack_index(&self) -> Option<usize> {
        match self {
            Upvalue::Open(i) => Some(*i),
            Upvalue::Closed(_) => None,
        }
    }
}

/// Simple Vec-based pool for small objects
/// - Direct O(1) indexing with no chunking overhead
/// - Free list for slot reuse
/// - Objects stored inline in Vec
pub struct GcPool {
    gc_list: Vec<Option<GcObject>>,
    free_list: Vec<u32>,
    count: usize,
}

impl GcPool {
    #[inline]
    pub fn new() -> Self {
        Self {
            gc_list: Vec::new(),
            free_list: Vec::new(),
            count: 0,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            gc_list: Vec::with_capacity(cap),
            free_list: Vec::with_capacity(cap / 8),
            count: 0,
        }
    }

    /// Allocate a new object and return its ID
    #[inline]
    pub fn alloc(&mut self, value: GcObject) -> u32 {
        self.count += 1;

        if let Some(free_id) = self.free_list.pop() {
            self.gc_list[free_id as usize] = Some(value);
            return free_id;
        }

        let id = self.gc_list.len() as u32;
        self.gc_list.push(Some(value));
        id
    }

    /// Get immutable reference by ID
    #[inline(always)]
    pub fn get(&self, id: u32) -> Option<&GcObject> {
        self.gc_list.get(id as usize).and_then(|opt| opt.as_ref())
    }

    /// Get mutable reference by ID
    #[inline(always)]
    pub fn get_mut(&mut self, id: u32) -> Option<&mut GcObject> {
        self.gc_list
            .get_mut(id as usize)
            .and_then(|opt| opt.as_mut())
    }

    /// Free a slot (mark for reuse)
    #[inline]
    pub fn free(&mut self, id: u32) {
        if let Some(slot) = self.gc_list.get_mut(id as usize) {
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
        while self.gc_list.last().map_or(false, |v| v.is_none()) {
            self.gc_list.pop();
        }
        // Remove free list entries that are now out of bounds
        let max_valid = self.gc_list.len() as u32;
        self.free_list.retain(|&id| id < max_valid);
    }

    /// Check if a slot is occupied
    #[inline(always)]
    pub fn is_valid(&self, id: u32) -> bool {
        self.gc_list
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
    pub fn iter(&self) -> impl Iterator<Item = (u32, &GcObject)> {
        self.gc_list
            .iter()
            .enumerate()
            .filter_map(|(id, opt)| opt.as_ref().map(|v| (id as u32, v)))
    }

    /// Iterate over all live objects mutably
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut GcObject)> {
        self.gc_list
            .iter_mut()
            .enumerate()
            .filter_map(|(id, opt)| opt.as_mut().map(|v| (id as u32, v)))
    }

    /// Shrink internal storage
    pub fn shrink_to_fit(&mut self) {
        self.gc_list.shrink_to_fit();
        self.free_list.shrink_to_fit();
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        self.gc_list.clear();
        self.free_list.clear();
        self.count = 0;
    }
}

impl Default for GcPool {
    fn default() -> Self {
        Self::new()
    }
}
