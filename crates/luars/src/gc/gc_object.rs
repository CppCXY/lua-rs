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
    pub unsafe fn get_value_unchecked(&self, current_thread: &LuaState) -> LuaValue {
        unsafe {
            let upval = &*self.ptr;
            if upval.is_open() {
                // Check if upvalue belongs to current thread
                let owner = upval.thread;
                let current_ptr = current_thread as *const LuaState;

                let val = if owner == current_ptr {
                    // Optimized: use current stack
                    *current_thread.stack().get_unchecked(upval.stack_index)
                } else {
                    // Cross-thread access: dereference owner thread to get its stack
                    // SAFETY: If upvalue is open, the owner thread must be alive
                    let owner_ref = &*owner;
                    *owner_ref.stack().get_unchecked(upval.stack_index)
                };

                val
            } else {
                upval.closed_value
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
        // New objects start as BLACK with age G_NEW
        // This ensures they survive the current GC cycle
        // They will be properly marked or turned white at the start of next cycle
        GcHeader {
            marked: (1 << BLACKBIT) | G_NEW,
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
    #[inline(always)]
    pub fn make_white(&mut self, current_white: u8) {
        // Clear color bits, set appropriate white bit, keep age
        let age = self.age();
        self.marked = (1 << (WHITE0BIT + current_white)) | age;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gc<T> {
    pub header: GcHeader,
    pub data: Box<T>, // Use Box to ensure stable pointer even when Pool array grows
}

impl<T> Gc<T> {
    /// Create a new GC object with default header
    pub fn new(data: T) -> Self {
        Gc {
            header: GcHeader::default(),
            data: Box::new(data),
        }
    }

    /// Get raw pointer to data (stable across Pool reallocations)
    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.data.as_ref() as *const T
    }

    /// Get mutable raw pointer to data
    #[inline(always)]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.data.as_mut() as *mut T
    }
}

// ============ GC-managed Objects ============

pub type GcTable = Gc<LuaTable>;

/// Function body - either Lua bytecode or C function
pub enum FunctionBody {
    /// Lua function with bytecode chunk
    /// Now includes cached upvalue pointers for direct access (zero-overhead like Lua C)
    Lua(Rc<Chunk>, Vec<CachedUpvalue>),
    /// C function (native Rust function) with cached upvalues
    CClosure(CFunction, Vec<CachedUpvalue>),
}

pub type GcFunction = Gc<FunctionBody>;

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

    /// Get upvalue IDs (for compatibility)
    #[inline(always)]
    pub fn upvalues(&self) -> Vec<UpvalueId> {
        self.cached_upvalues().iter().map(|cu| cu.id).collect()
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
/// Hybrid design for safety and performance:
/// - When open: uses stack_index (safe, no dangling pointers)
/// - When closed: stores value inline (fast, no indirection)
///
/// Note: We cannot use raw pointers for open upvalues because
/// register_stack may reallocate, invalidating the pointers.
pub struct Upvalue {
    /// The stack index when open (used for accessing stack value)
    pub stack_index: usize,
    /// Storage for closed value
    pub closed_value: LuaValue,
    /// Whether the upvalue is open (still pointing to stack)
    pub is_open: bool,
    /// The thread (LuaState) that owns the stack this upvalue points to (unsafe ptr)
    /// Only valid/used when is_open is true
    pub thread: *const LuaState,
}

pub type GcUpvalue = Gc<Upvalue>;

impl Upvalue {
    /// Check if this upvalue points to the given absolute stack index
    #[inline]
    pub fn points_to_index(&self, index: usize) -> bool {
        self.is_open && self.stack_index == index
    }

    /// Check if this upvalue is open (still points to stack)
    #[inline]
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Close this upvalue with the given value
    #[inline]
    pub fn close(&mut self, value: LuaValue) {
        self.closed_value = value;
        self.is_open = false;
    }

    /// Get the value of a closed upvalue (returns None if still open)
    #[inline]
    pub fn get_closed_value(&self) -> Option<LuaValue> {
        if self.is_open {
            None
        } else {
            Some(self.closed_value)
        }
    }

    /// Get the absolute stack index if this upvalue is open
    #[inline]
    pub fn get_stack_index(&self) -> Option<usize> {
        if self.is_open {
            Some(self.stack_index)
        } else {
            None
        }
    }

    /// Set closed upvalue value directly without checking state
    /// SAFETY: Must only be called when upvalue is in Closed state
    #[inline(always)]
    pub unsafe fn set_closed_value_unchecked(&mut self, value: LuaValue) {
        self.closed_value = value;
    }

    /// Close upvalue with given value (used during stack unwinding)
    #[inline]
    pub unsafe fn close_with_value(&mut self, value: LuaValue) {
        self.closed_value = value;
        self.is_open = false;
    }

    /// Get closed value reference directly without Option
    /// SAFETY: Must only be called when upvalue is in Closed state
    #[inline(always)]
    pub unsafe fn get_closed_value_ref_unchecked(&self) -> &LuaValue {
        &self.closed_value
    }
}

/// String with embedded GC header
pub type GcString = Gc<String>;

/// Thread (coroutine) with embedded GC header
/// TODO: Remove Rc<RefCell> once we have proper pointer-based design
pub type GcThread = Gc<LuaState>;

pub type GcUserdata = Gc<LuaUserdata>;
