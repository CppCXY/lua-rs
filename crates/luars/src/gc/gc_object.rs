// ============ GC Header ============

use std::rc::Rc;

use crate::{Chunk, LuaString, LuaTable, LuaValue, UpvalueId, lua_value::LuaThread};

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
#[derive(Clone, Copy)]
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

// Legacy field accessors for compatibility
impl GcHeader {
    #[inline(always)]
    pub fn get_fixed(&self) -> bool {
        self.is_fixed()
    }
}

// ============ GC-managed Objects ============

/// Table with embedded GC header
pub struct GcTable {
    pub header: GcHeader,
    pub data: LuaTable,
}

/// C Function type - Rust function callable from Lua
pub type CFunction =
    fn(&mut crate::lua_vm::LuaVM) -> crate::lua_vm::LuaResult<crate::lua_value::MultiValue>;

/// Function body - either Lua bytecode or C function
pub enum FunctionBody {
    /// Lua function with bytecode chunk
    Lua(Rc<Chunk>),
    /// C function (native Rust function) - no upvalues
    C(CFunction),
    /// C closure with single inline upvalue (fast path for common case)
    /// Used by coroutine.wrap, ipairs iterator, etc.
    CClosureInline1(CFunction, LuaValue),
}

/// Unified function with embedded GC header
/// Supports both Lua closures and C closures (with upvalues)
pub struct GcFunction {
    pub header: GcHeader,
    pub body: FunctionBody,
    pub upvalues: Vec<UpvalueId>, // Upvalue IDs - used for Lua closures and C closures with >1 upvalue
}

impl GcFunction {
    /// Check if this is a C function (any C variant)
    #[inline(always)]
    pub fn is_c_function(&self) -> bool {
        matches!(
            self.body,
            FunctionBody::C(_) | FunctionBody::CClosureInline1(_, _)
        )
    }

    /// Check if this is a Lua function
    #[inline(always)]
    pub fn is_lua_function(&self) -> bool {
        matches!(self.body, FunctionBody::Lua(_))
    }

    /// Get the chunk if this is a Lua function
    #[inline(always)]
    pub fn chunk(&self) -> Option<&Rc<Chunk>> {
        match &self.body {
            FunctionBody::Lua(chunk) => Some(chunk),
            _ => None,
        }
    }

    /// Get the chunk reference for Lua functions (panics if C function)
    /// Use this in contexts where we know it's a Lua function
    #[inline(always)]
    pub fn lua_chunk(&self) -> &Rc<Chunk> {
        match &self.body {
            FunctionBody::Lua(chunk) => chunk,
            _ => panic!("Called lua_chunk() on a C function"),
        }
    }

    /// Get the C function pointer if this is any C function variant
    #[inline(always)]
    pub fn c_function(&self) -> Option<CFunction> {
        match &self.body {
            FunctionBody::C(f) => Some(*f),
            FunctionBody::CClosureInline1(f, _) => Some(*f),
            FunctionBody::Lua(_) => None,
        }
    }

    /// Get inline upvalue 1 for CClosureInline1
    #[inline(always)]
    pub fn inline_upvalue1(&self) -> Option<LuaValue> {
        match &self.body {
            FunctionBody::CClosureInline1(_, uv) => Some(*uv),
            _ => None,
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
pub struct GcUpvalue {
    pub header: GcHeader,
    /// The stack index when open (used for accessing stack value)
    pub stack_index: usize,
    /// Storage for closed value
    pub closed_value: LuaValue,
    /// Whether the upvalue is open (still pointing to stack)
    pub is_open: bool,
}

impl GcUpvalue {
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

    /// Get closed value reference directly without Option
    /// SAFETY: Must only be called when upvalue is in Closed state
    #[inline(always)]
    pub unsafe fn get_closed_value_ref_unchecked(&self) -> &LuaValue {
        &self.closed_value
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
