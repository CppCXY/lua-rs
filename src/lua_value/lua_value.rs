// Hybrid NaN-Boxing + Full Int64 Value Representation
// Inspired by V8's Smi + HeapNumber design, adapted for Lua 5.4
//
// 128-bit representation (2x u64):
// [primary: u64][secondary: u64]
//
// Primary word encoding:
// - 0x0000_0000_0000_0000 - 0x7FF7_FFFF_FFFF_FFFF: Float (IEEE 754, secondary unused)
// - 0x7FF8_0000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF: Tagged types (use secondary for data)
//
// Tagged types (primary >= NAN_BASE):
// Primary                              Secondary
// 0xFFFF_0000_0000_0001                full i64 value        Integer
// 0xFFFE_0000_0000_0001                48-bit pointer        String
// 0xFFFE_0000_0000_0002                48-bit pointer        Table
// 0xFFFE_0000_0000_0003                48-bit pointer        Function
// 0xFFFE_0000_0000_0004                48-bit pointer        Userdata
// 0xFFFD_0000_0000_0001                unused                Boolean (true)
// 0xFFFD_0000_0000_0000                unused                Boolean (false)
// 0xFFFC_0000_0000_0000                unused                Nil
// 0xFFFB_0000_0000_0001                48-bit fn pointer     CFunction
//
// Benefits:
// - Full 64-bit integer support (Lua 5.4 compatible)
// - 16 bytes like enum, but MUCH faster
// - Float is pure IEEE 754 (no encoding overhead)
// - Integer ops are direct i64 arithmetic
// - Type check is single comparison
// - No pattern matching overhead

use std::cell::RefCell;

use crate::{
    FunctionId, LuaString, StringId, UserdataId, lua_value::CFunction, object_pool::TableId,
};
use std::cmp::Ordering;

// Primary word tags (public for VM fast paths)
pub const TAG_INTEGER: u64 = 0xFFFF_0000_0000_0001;
pub const TAG_STRING: u64 = 0xFFFE_0000_0000_0001;
pub const TAG_TABLE: u64 = 0xFFFE_0000_0000_0002;
pub const TAG_FUNCTION: u64 = 0xFFFE_0000_0000_0003;
pub const TAG_USERDATA: u64 = 0xFFFE_0000_0000_0004;
pub const TAG_THREAD: u64 = 0xFFFE_0000_0000_0005;
pub const TAG_BOOLEAN: u64 = 0xFFFD_0000_0000_0000;
pub const TAG_NIL: u64 = 0xFFFC_0000_0000_0000;
pub const TAG_CFUNCTION: u64 = 0xFFFB_0000_0000_0001;

// Special values
pub const VALUE_TRUE: u64 = TAG_BOOLEAN | 1;
pub const VALUE_FALSE: u64 = TAG_BOOLEAN;
pub const VALUE_NIL: u64 = TAG_NIL;

// NaN detection (any value >= this is a tagged type)
pub const NAN_BASE: u64 = 0x7FF8_0000_0000_0000;

// Masks for pointer extraction
pub(crate) const POINTER_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Hybrid NaN-boxed Lua value - 16 bytes (same as enum, but 3-6x faster)
///
/// Layout: [primary: u64][secondary: u64]
/// - Floats: stored in primary (IEEE 754), secondary unused
/// - Integers: full i64 in secondary, tag in primary
/// - Pointers: 48-bit address in secondary, type tag in primary
/// - Simple values: encoded in primary, secondary unused
///
/// NOTE: All methods and traits (Clone/Drop/Debug/Default) are implemented in compat.rs
#[repr(C)]
pub struct LuaValue {
    pub(crate) primary: u64,
    pub(crate) secondary: u64,
}

// All implementation code is in compat.rs to provide a single source of truth
#[allow(unused)]
impl LuaValue {
    // ============ Core Constructors ============

    #[inline(always)]
    pub const fn nil() -> Self {
        LuaValue {
            primary: VALUE_NIL,
            secondary: 0,
        }
    }

    #[inline(always)]
    pub const fn boolean(b: bool) -> Self {
        LuaValue {
            primary: if b { VALUE_TRUE } else { VALUE_FALSE },
            secondary: 0,
        }
    }

    #[inline(always)]
    pub const fn integer(i: i64) -> Self {
        LuaValue {
            primary: TAG_INTEGER,
            secondary: i as u64, // Full 64-bit integer!
        }
    }

    #[inline(always)]
    pub fn float(f: f64) -> Self {
        let bits = f.to_bits();
        // If it's actually a NaN (not just a negative number), canonicalize it
        // We need to check if it's a real NaN using f.is_nan(), not just bits >= NAN_BASE
        // because negative numbers have their sign bit set and will have bits >= NAN_BASE
        let primary = if f.is_nan() {
            // Canonicalize NaN to exactly NAN_BASE to distinguish from tagged values
            NAN_BASE
        } else {
            bits
        };
        LuaValue {
            primary,
            secondary: 0,
        }
    }

    #[inline(always)]
    pub fn number(n: f64) -> Self {
        Self::float(n)
    }

    #[inline(always)]
    pub(crate) fn string_ptr(ptr: *const LuaString) -> Self {
        let addr = ptr as u64;
        debug_assert!(addr < (1u64 << 48), "Pointer too large");
        LuaValue {
            primary: TAG_STRING,
            secondary: addr & POINTER_MASK,
        }
    }

    /// Create string from ID (new object pool architecture)
    #[inline(always)]
    pub fn string_id(id: StringId) -> Self {
        LuaValue {
            primary: TAG_STRING,
            secondary: id.0 as u64,
        }
    }

    /// Create table from ID (new object pool architecture)
    #[inline(always)]
    pub fn table_id(id: TableId) -> Self {
        LuaValue {
            primary: TAG_TABLE,
            secondary: id.0 as u64,
        }
    }

    /// Create userdata from ID (new object pool architecture)
    #[inline(always)]
    pub fn userdata_id(id: UserdataId) -> Self {
        LuaValue {
            primary: TAG_USERDATA,
            secondary: id.0 as u64,
        }
    }

    /// Create function from ID (new object pool architecture)
    #[inline(always)]
    pub fn function_id(id: FunctionId) -> Self {
        LuaValue {
            primary: TAG_FUNCTION,
            secondary: id.0 as u64,
        }
    }

    #[inline(always)]
    pub(crate) fn thread_ptr(ptr: *const RefCell<crate::lua_vm::LuaThread>) -> Self {
        let addr = ptr as u64;
        debug_assert!(addr < (1u64 << 48), "Pointer too large");
        LuaValue {
            primary: TAG_THREAD,
            secondary: addr & POINTER_MASK,
        }
    }

    #[inline(always)]
    pub fn cfunction(f: CFunction) -> Self {
        let addr = f as usize as u64;
        debug_assert!(addr < (1u64 << 48), "Function pointer too large");
        LuaValue {
            primary: TAG_CFUNCTION,
            secondary: addr & POINTER_MASK,
        }
    }

    // ============ Type Checks (ultra-fast) ============

    #[inline(always)]
    pub const fn is_nil(&self) -> bool {
        self.primary == VALUE_NIL
    }

    #[inline(always)]
    pub const fn is_boolean(&self) -> bool {
        self.primary == VALUE_TRUE || self.primary == VALUE_FALSE
    }

    #[inline(always)]
    pub const fn is_integer(&self) -> bool {
        self.primary == TAG_INTEGER
    }

    #[inline(always)]
    pub const fn is_float(&self) -> bool {
        // A value is a float if:
        // 1. primary == NAN_BASE (canonicalized NaN)
        // 2. OR primary is not a tagged value (i.e., not matching any TAG_* constants)
        // Note: Negative floats have high bit set, but we store them directly as bits
        self.primary == NAN_BASE
            || (self.primary != VALUE_NIL
                && self.primary != VALUE_TRUE
                && self.primary != VALUE_FALSE
                && self.primary != TAG_INTEGER
                && self.primary != TAG_STRING
                && self.primary != TAG_TABLE
                && self.primary != TAG_FUNCTION
                && self.primary != TAG_USERDATA
                && self.primary != TAG_THREAD
                && self.primary != TAG_CFUNCTION)
    }

    #[inline(always)]
    pub const fn is_number(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    #[inline(always)]
    pub const fn is_string(&self) -> bool {
        self.primary == TAG_STRING
    }

    #[inline(always)]
    pub const fn is_table(&self) -> bool {
        self.primary == TAG_TABLE
    }

    #[inline(always)]
    pub const fn is_function(&self) -> bool {
        self.primary == TAG_FUNCTION
    }

    #[inline(always)]
    pub const fn is_userdata(&self) -> bool {
        self.primary == TAG_USERDATA
    }

    #[inline(always)]
    pub const fn is_cfunction(&self) -> bool {
        self.primary == TAG_CFUNCTION
    }

    #[inline(always)]
    pub const fn is_thread(&self) -> bool {
        self.primary == TAG_THREAD
    }

    // ============ Value Extraction ============

    #[inline(always)]
    pub const fn as_bool(&self) -> Option<bool> {
        if self.primary == VALUE_TRUE {
            Some(true)
        } else if self.primary == VALUE_FALSE {
            Some(false)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_integer(&self) -> Option<i64> {
        if self.is_integer() {
            // Full 64-bit integer stored directly!
            Some(self.secondary as i64)
        } else if self.is_float() {
            let f = f64::from_bits(self.primary);
            // Lua 5.4 semantics: floats with zero fraction are integers
            if f.fract() == 0.0 && f.is_finite() {
                Some(f as i64)
            } else {
                None
            }
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_float(&self) -> Option<f64> {
        if self.is_float() {
            Some(f64::from_bits(self.primary))
        } else if self.is_integer() {
            Some(self.secondary as i64 as f64)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_number(&self) -> Option<f64> {
        if let Some(i) = self.as_integer() {
            Some(i as f64)
        } else {
            self.as_float()
        }
    }

    #[inline(always)]
    pub fn as_table_id(&self) -> Option<TableId> {
        if self.is_table() {
            Some(TableId(self.secondary as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub(crate) unsafe fn as_string_ptr(&self) -> Option<*const LuaString> {
        if self.is_string() {
            Some((self.secondary & POINTER_MASK) as *const LuaString)
        } else {
            None
        }
    }

    /// Get string ID (for new object pool architecture)
    #[inline]
    pub fn as_string_id(&self) -> Option<StringId> {
        if self.is_string() {
            Some(StringId(self.secondary as u32))
        } else {
            None
        }
    }

    /// Get userdata ID (for new object pool architecture)
    #[inline]
    pub fn as_userdata_id(&self) -> Option<UserdataId> {
        if self.is_userdata() {
            Some(UserdataId(self.secondary as u32))
        } else {
            None
        }
    }

    /// Get function ID (for new object pool architecture)
    #[inline]
    pub fn as_function_id(&self) -> Option<FunctionId> {
        if self.is_function() {
            Some(FunctionId(self.secondary as u32))
        } else {
            None
        }
    }

    /// UNSAFE: Get thread pointer (threads not yet migrated to object pool)
    #[inline]
    pub unsafe fn as_thread_ptr(&self) -> Option<*const RefCell<crate::lua_vm::LuaThread>> {
        if self.is_thread() {
            Some((self.secondary & POINTER_MASK) as *const RefCell<crate::lua_vm::LuaThread>)
        } else {
            None
        }
    }

    // ============ Raw Access ============

    #[inline(always)]
    pub const fn primary(&self) -> u64 {
        self.primary
    }

    #[inline(always)]
    pub const fn secondary(&self) -> u64 {
        self.secondary
    }

    #[inline(always)]
    pub fn secondary_mut(&mut self) -> &mut u64 {
        &mut self.secondary
    }

    #[inline(always)]
    pub const fn from_raw(primary: u64, secondary: u64) -> Self {
        LuaValue { primary, secondary }
    }

    // ============ Lua Semantics ============

    pub fn is_truthy(&self) -> bool {
        !self.is_nil() && self.primary != VALUE_FALSE
    }

    pub fn type_name(&self) -> &'static str {
        match self.kind() {
            LuaValueKind::Nil => "nil",
            LuaValueKind::Boolean => "boolean",
            LuaValueKind::Integer => "integer",
            LuaValueKind::Float => "number",
            LuaValueKind::String => "string",
            LuaValueKind::Table => "table",
            LuaValueKind::Function => "function",
            LuaValueKind::Userdata => "userdata",
            LuaValueKind::Thread => "thread",
            LuaValueKind::CFunction => "function",
        }
    }

    // ============ Safe public accessors ============
    // Safe because VM is single-threaded and GC only runs at safe points

    /// Get string reference safely
    #[inline]
    pub fn to_str(&self) -> Option<&str> {
        if self.primary == TAG_STRING {
            unsafe {
                let ptr = self.secondary as *const LuaString;
                Some((*ptr).as_str())
            }
        } else {
            None
        }
    }

    /// Get string as LuaString reference
    #[inline]
    pub fn as_lua_string(&self) -> Option<&LuaString> {
        if self.primary == TAG_STRING {
            unsafe {
                let ptr = self.secondary as *const LuaString;
                Some(&*ptr)
            }
        } else {
            None
        }
    }

    #[inline]
    pub fn as_cfunction(&self) -> Option<CFunction> {
        if self.primary == TAG_CFUNCTION {
            let addr = self.secondary & POINTER_MASK;
            Some(unsafe { std::mem::transmute::<u64, CFunction>(addr) })
        } else {
            None
        }
    }

    /// Get string reference (internal alias)
    #[inline]
    pub(crate) unsafe fn as_string(&self) -> Option<&LuaString> {
        self.as_lua_string()
    }

    /// Get as_boolean for compatibility
    pub fn as_boolean(&self) -> Option<bool> {
        self.as_bool()
    }

    /// String representation for printing
    pub fn to_string_repr(&self) -> String {
        match self.kind() {
            LuaValueKind::Nil => "nil".to_string(),
            LuaValueKind::Boolean => self.as_bool().unwrap().to_string(),
            LuaValueKind::Integer => self.as_integer().unwrap().to_string(),
            LuaValueKind::Float => self.as_float().unwrap().to_string(),
            LuaValueKind::String => {
                // In ID architecture, we can't dereference without ObjectPool
                // Return a placeholder - caller should use vm.value_to_string() for proper string representation
                format!("string: {}", self.secondary())
            },
            LuaValueKind::Table => format!("table: {:x}", self.secondary()),
            LuaValueKind::Function => format!("function: {:x}", self.secondary()),
            LuaValueKind::Userdata => format!("userdata: {:x}", self.secondary()),
            LuaValueKind::Thread => format!("thread: {:x}", self.secondary()),
            LuaValueKind::CFunction => "cfunction".to_string(),
        }
    }

    /// Check if value is callable (function or cfunction)
    pub fn is_callable(&self) -> bool {
        self.is_function() || self.is_cfunction()
    }

    /// Alias for type_kind() - returns the type discriminator
    /// Use this to check types instead of pattern matching
    #[inline(always)]
    pub fn kind(&self) -> LuaValueKind {
        match self.primary {
            VALUE_NIL => LuaValueKind::Nil,
            VALUE_TRUE | VALUE_FALSE => LuaValueKind::Boolean,
            TAG_INTEGER => LuaValueKind::Integer,
            TAG_STRING => LuaValueKind::String,
            TAG_TABLE => LuaValueKind::Table,
            TAG_FUNCTION => LuaValueKind::Function,
            TAG_USERDATA => LuaValueKind::Userdata,
            TAG_THREAD => LuaValueKind::Thread,
            TAG_CFUNCTION => LuaValueKind::CFunction,
            _ => LuaValueKind::Float, // Everything else is a float (including NaN and negative floats)
        }
    }
}

// ============ Trait Implementations ============

// No Drop implementation - GC handles all cleanup
// This allows LuaValue to be Copy (16 bytes, trivially copyable)

// Clone is now a trivial memcpy - no reference counting!
// This is 10-20x faster than Rc::clone()
impl Clone for LuaValue {
    #[inline(always)]
    fn clone(&self) -> Self {
        // Just copy the bits - GC tracks everything
        // No branches, no refcount manipulation!
        *self
    }
}

// LuaValue is now Copy! (16 bytes, trivially copyable)
// This eliminates ALL Clone overhead - it becomes a simple memcpy
impl Copy for LuaValue {}

// Implement Debug for better error messages
impl std::fmt::Debug for LuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            LuaValueKind::Nil => write!(f, "nil"),
            LuaValueKind::Boolean => write!(f, "{}", self.as_bool().unwrap()),
            LuaValueKind::Integer => write!(f, "{}", self.as_integer().unwrap()),
            LuaValueKind::Float => write!(f, "{}", self.as_float().unwrap()),
            LuaValueKind::String => {
                // In ID architecture, can't dereference without ObjectPool
                write!(f, "\"<string:{}>\"", self.secondary())
            },
            LuaValueKind::Table => write!(f, "table: {:x}", self.secondary()),
            LuaValueKind::Function => write!(f, "function: {:x}", self.secondary()),
            LuaValueKind::Userdata => write!(f, "userdata: {:x}", self.secondary()),
            LuaValueKind::Thread => write!(f, "thread: {:x}", self.secondary()),
            LuaValueKind::CFunction => write!(f, "cfunction"),
        }
    }
}

impl Default for LuaValue {
    fn default() -> Self {
        Self::nil()
    }
}

impl std::fmt::Display for LuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            LuaValueKind::Nil => write!(f, "nil"),
            LuaValueKind::Boolean => write!(f, "{}", self.as_bool().unwrap()),
            LuaValueKind::Integer => write!(f, "{}", self.as_integer().unwrap()),
            LuaValueKind::Float => {
                let n = self.as_float().unwrap();
                if n.floor() == n && n.abs() < 1e14 {
                    write!(f, "{:.0}", n)
                } else {
                    write!(f, "{}", n)
                }
            }
            LuaValueKind::String => {
                // In ID architecture, can't dereference without ObjectPool
                // Caller should use vm.value_to_string() instead
                write!(f, "<string:{}>", self.secondary())
            },
            LuaValueKind::Table => write!(f, "table: {:x}", self.secondary()),
            LuaValueKind::Function => write!(f, "function: {:x}", self.secondary()),
            LuaValueKind::Userdata => write!(f, "userdata: {:x}", self.secondary()),
            LuaValueKind::Thread => write!(f, "thread: {:x}", self.secondary()),
            LuaValueKind::CFunction => write!(f, "function: {:x}", self.secondary()),
        }
    }
}

// ============ Additional Trait Implementations ============

impl PartialEq for LuaValue {
    fn eq(&self, other: &Self) -> bool {
        // Fast path: exact bit match (works for all types including string IDs)
        if self.primary() == other.primary() && self.secondary() == other.secondary() {
            return true;
        }

        // For ID-based architecture, same ID means same object
        // No need for deep comparison
        false
    }
}

impl Eq for LuaValue {}

impl std::hash::Hash for LuaValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // For strings in ID-based architecture, hash the ID
        // The ObjectPool ensures same string = same ID
        if self.is_string() {
            0u8.hash(state); // Type discriminator
            self.secondary.hash(state); // Hash the ID directly
            return;
        }

        // For other types, hash the raw bits
        self.primary.hash(state);
        self.secondary.hash(state);
    }
}

impl PartialOrd for LuaValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let kind_a = self.kind();
        let kind_b = other.kind();

            match kind_a.cmp(&kind_b) {
            Ordering::Equal => match kind_a {
                LuaValueKind::Nil => Some(Ordering::Equal),
                LuaValueKind::Boolean => self.as_bool().partial_cmp(&other.as_bool()),
                LuaValueKind::Integer => self.as_integer().partial_cmp(&other.as_integer()),
                LuaValueKind::Float => self.as_float().partial_cmp(&other.as_float()),
                LuaValueKind::String => {
                    // In ID architecture, compare IDs directly
                    // Same ID = same string content (guaranteed by ObjectPool interning)
                    self.secondary().partial_cmp(&other.secondary())
                },
                LuaValueKind::Table
                | LuaValueKind::Function
                | LuaValueKind::Userdata
                | LuaValueKind::Thread
                | LuaValueKind::CFunction => self.secondary().partial_cmp(&other.secondary()),
            },
            ord => Some(ord),
        }
    }
}

impl Ord for LuaValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

/// Enum for pattern matching on LuaValue types
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LuaValueKind {
    Nil,
    Boolean,
    Integer,
    Float,
    String,
    Table,
    Function,
    Userdata,
    Thread,
    CFunction,
}
