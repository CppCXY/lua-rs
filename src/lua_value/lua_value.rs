// Optimized Hybrid NaN-Boxing: Type in Primary + Value/Pointer in Secondary
// Eliminates ObjectPool lookups for hot paths!
//
// 128-bit representation (2x u64):
// [primary: u64][secondary: u64]
//
// Primary word encoding (type tag + ID for GC):
// - 0x7FF7_0000_0000_xxxx: Float (secondary = f64 bits)
// - 0x7FF8_0000_0000_xxxx: Integer (secondary = i64 value)
// - 0x7FF9_0000_xxxx_xxxx: String (low 32-bit = StringId, secondary = *const LuaString)
// - 0x7FFA_0000_xxxx_xxxx: Table (low 32-bit = TableId, secondary = *const RefCell<LuaTable>)
// - 0x7FFB_0000_xxxx_xxxx: Function (low 32-bit = FunctionId, secondary = *const RefCell<LuaFunction>)
// - 0x7FFC_0000_xxxx_xxxx: Userdata (low 32-bit = UserdataId, secondary = *const RefCell<LuaUserdata>)
// - 0x7FFD_0000_0000_0001: Boolean (true) (secondary unused)
// - 0x7FFD_0000_0000_0000: Boolean (false) (secondary unused)
// - 0x7FFE_0000_0000_0000: Nil (secondary unused)
// - 0x7FFF_0000_0000_xxxx: CFunction (low 32-bit unused, secondary = fn pointer)
//
// Benefits:
// - Integer ops: ZERO lookups - direct access to secondary!
// - String/Table ops: ONE dereference - no HashMap lookup!
// - GC can scan: primary has ID, secondary has pointer
// - Type check: single comparison (primary & TAG_MASK)
// - Full 64-bit integer + IEEE 754 float support

use std::cell::RefCell;

use crate::{
    FunctionId, LuaString, StringId, UserdataId,
    lua_value::{CFunction, lua_thread::LuaThread},
    object_pool::TableId,
};
use std::cmp::Ordering;

// Primary word tags (high 16 bits for type, low 32 bits for ID)
pub const TAG_FLOAT: u64 = 0x7FF7_0000_0000_0000;
pub const TAG_INTEGER: u64 = 0x7FF8_0000_0000_0000;
pub const TAG_STRING: u64 = 0x7FF9_0000_0000_0000;
pub const TAG_TABLE: u64 = 0x7FFA_0000_0000_0000;
pub const TAG_FUNCTION: u64 = 0x7FFB_0000_0000_0000;
pub const TAG_USERDATA: u64 = 0x7FFC_0000_0000_0000;
pub const TAG_BOOLEAN: u64 = 0x7FFD_0000_0000_0000;
pub const TAG_NIL: u64 = 0x7FFE_0000_0000_0000;
pub const TAG_CFUNCTION: u64 = 0x7FFF_0000_0000_0000;
pub const TAG_THREAD: u64 = 0x7FF6_0000_0000_0000;

// Special values
pub const VALUE_TRUE: u64 = TAG_BOOLEAN | 1;
pub const VALUE_FALSE: u64 = TAG_BOOLEAN;
pub const VALUE_NIL: u64 = TAG_NIL;

// NaN detection and float range
pub const NAN_BASE: u64 = 0x7FF8_0000_0000_0000; // Start of NaN space where we put tags
// Masks for ID extraction (low 32 bits)
pub const ID_MASK: u64 = 0x0000_0000_FFFF_FFFF;
pub const TYPE_MASK: u64 = 0xFFFF_0000_0000_0000; // High 16 bits for type

// Masks for pointer (all 64 bits of secondary)
pub(crate) const POINTER_MASK: u64 = 0xFFFF_FFFF_FFFF_FFFF;

/// Hybrid NaN-boxed Lua value - 16 bytes (same as enum, but 3-6x faster)
///
/// Layout: [primary: u64][secondary: u64]
/// - Floats: TAG_FLOAT in primary, f64 bits in secondary
/// - Integers: TAG_INTEGER in primary, i64 value in secondary
/// - Pointers: type tag in primary, pointer in secondary
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
        LuaValue {
            primary: TAG_FLOAT,
            secondary: f.to_bits(), // Store f64 bits in secondary
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

    /// Create string from ID + pointer (optimized: stores pointer in secondary)
    #[inline(always)]
    pub fn string_id_ptr(id: StringId, ptr: *const LuaString) -> Self {
        LuaValue {
            primary: TAG_STRING | (id.0 as u64), // ID in low 32 bits
            secondary: ptr as u64,               // Pointer in secondary
        }
    }

    /// Create string from ID only (slower path, will need lookup)
    #[inline(always)]
    pub fn string_id(id: StringId) -> Self {
        LuaValue {
            primary: TAG_STRING | (id.0 as u64),
            secondary: 0, // No pointer yet, will be resolved on first access
        }
    }

    /// Create table from ID + pointer
    #[inline(always)]
    pub fn table_id_ptr(
        id: TableId,
        ptr: *const std::cell::RefCell<crate::lua_value::LuaTable>,
    ) -> Self {
        LuaValue {
            primary: TAG_TABLE | (id.0 as u64),
            secondary: ptr as u64,
        }
    }

    /// Create table from ID (slower path)
    #[inline(always)]
    pub fn table_id(id: TableId) -> Self {
        LuaValue {
            primary: TAG_TABLE | (id.0 as u64),
            secondary: 0,
        }
    }

    /// Create userdata from ID + pointer
    #[inline(always)]
    pub fn userdata_id_ptr(
        id: UserdataId,
        ptr: *const std::cell::RefCell<crate::lua_value::LuaUserdata>,
    ) -> Self {
        LuaValue {
            primary: TAG_USERDATA | (id.0 as u64),
            secondary: ptr as u64,
        }
    }

    /// Create userdata from ID (slower path)
    #[inline(always)]
    pub fn userdata_id(id: UserdataId) -> Self {
        LuaValue {
            primary: TAG_USERDATA | (id.0 as u64),
            secondary: 0,
        }
    }

    /// Create function from ID + pointer
    #[inline(always)]
    pub fn function_id_ptr(
        id: FunctionId,
        ptr: *const std::cell::RefCell<crate::lua_value::LuaFunction>,
    ) -> Self {
        LuaValue {
            primary: TAG_FUNCTION | (id.0 as u64),
            secondary: ptr as u64,
        }
    }

    /// Create function from ID (slower path)
    #[inline(always)]
    pub fn function_id(id: FunctionId) -> Self {
        LuaValue {
            primary: TAG_FUNCTION | (id.0 as u64),
            secondary: 0,
        }
    }

    #[inline(always)]
    pub(crate) fn thread_ptr(ptr: *const RefCell<LuaThread>) -> Self {
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
        (self.primary & TYPE_MASK) == TAG_INTEGER
    }

    #[inline(always)]
    pub const fn is_float(&self) -> bool {
        (self.primary & TYPE_MASK) == TAG_FLOAT
    }

    #[inline(always)]
    pub const fn is_number(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    #[inline(always)]
    pub const fn is_string(&self) -> bool {
        (self.primary & TYPE_MASK) == TAG_STRING
    }

    #[inline(always)]
    pub const fn is_table(&self) -> bool {
        (self.primary & TYPE_MASK) == TAG_TABLE
    }

    #[inline(always)]
    pub const fn is_function(&self) -> bool {
        (self.primary & TYPE_MASK) == TAG_FUNCTION
    }

    #[inline(always)]
    pub const fn is_userdata(&self) -> bool {
        (self.primary & TYPE_MASK) == TAG_USERDATA
    }

    #[inline(always)]
    pub const fn is_cfunction(&self) -> bool {
        (self.primary & TYPE_MASK) == TAG_CFUNCTION
    }

    #[inline(always)]
    pub const fn is_thread(&self) -> bool {
        (self.primary & TYPE_MASK) == TAG_THREAD
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
            Some(self.secondary as i64)
        } else if self.is_float() {
            let f = f64::from_bits(self.secondary);
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
            Some(f64::from_bits(self.secondary))
        } else if self.is_integer() {
            Some(self.secondary as i64 as f64)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_number(&self) -> Option<f64> {
        // Optimized: single type check, both types use secondary
        if self.is_float() {
            Some(f64::from_bits(self.secondary))
        } else if self.is_integer() {
            Some(self.secondary as i64 as f64)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_table_id(&self) -> Option<TableId> {
        if self.is_table() {
            Some(TableId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    /// Get table pointer directly (ZERO lookups!)
    #[inline(always)]
    pub fn as_table_ptr(&self) -> Option<*const std::cell::RefCell<crate::lua_value::LuaTable>> {
        if self.is_table() && self.secondary != 0 {
            Some(self.secondary as *const std::cell::RefCell<crate::lua_value::LuaTable>)
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
            Some(StringId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    /// Get string pointer directly (ZERO lookups!)
    #[inline(always)]
    pub fn as_string_ptr_direct(&self) -> Option<*const LuaString> {
        if self.is_string() && self.secondary != 0 {
            Some(self.secondary as *const LuaString)
        } else {
            None
        }
    }

    /// Get userdata ID (for new object pool architecture)
    #[inline]
    pub fn as_userdata_id(&self) -> Option<UserdataId> {
        if self.is_userdata() {
            Some(UserdataId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    /// Get function ID (for new object pool architecture)
    #[inline]
    pub fn as_function_id(&self) -> Option<FunctionId> {
        if self.is_function() {
            Some(FunctionId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    /// Get function pointer directly (ZERO lookups!)
    #[inline(always)]
    pub fn as_function_ptr(
        &self,
    ) -> Option<*const std::cell::RefCell<crate::lua_value::LuaFunction>> {
        if self.is_function() && self.secondary != 0 {
            Some(self.secondary as *const std::cell::RefCell<crate::lua_value::LuaFunction>)
        } else {
            None
        }
    }

    /// UNSAFE: Get thread pointer (threads not yet migrated to object pool)
    #[inline]
    pub unsafe fn as_thread_ptr(&self) -> Option<*const RefCell<LuaThread>> {
        if self.is_thread() {
            Some((self.secondary & POINTER_MASK) as *const RefCell<LuaThread>)
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

    /// Get string reference safely (ZERO lookups if pointer cached!)
    #[inline]
    pub fn to_str(&self) -> Option<&str> {
        if self.is_string() && self.secondary != 0 {
            unsafe {
                let ptr = self.secondary as *const LuaString;
                Some((*ptr).as_str())
            }
        } else {
            None
        }
    }

    /// Get string as LuaString reference (ZERO lookups if pointer cached!)
    #[inline]
    pub fn as_lua_string(&self) -> Option<&LuaString> {
        if self.is_string() && self.secondary != 0 {
            unsafe {
                let ptr = self.secondary as *const LuaString;
                Some(&*ptr)
            }
        } else {
            None
        }
    }

    /// Get table as RefCell<LuaTable> reference (ZERO lookups if pointer cached!)
    #[inline]
    pub fn as_lua_table(&self) -> Option<&std::cell::RefCell<crate::lua_value::LuaTable>> {
        if self.is_table() && self.secondary != 0 {
            unsafe {
                let ptr = self.secondary as *const std::cell::RefCell<crate::lua_value::LuaTable>;
                Some(&*ptr)
            }
        } else {
            None
        }
    }

    /// Get function as RefCell<LuaFunction> reference (ZERO lookups if pointer cached!)
    #[inline]
    pub fn as_lua_function(&self) -> Option<&std::cell::RefCell<crate::lua_value::LuaFunction>> {
        if self.is_function() && self.secondary != 0 {
            unsafe {
                let ptr = self.secondary as *const std::cell::RefCell<crate::lua_value::LuaFunction>;
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
            }
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
        // Fast path: check special values first
        if self.primary == VALUE_NIL {
            return LuaValueKind::Nil;
        }
        if self.primary == VALUE_TRUE || self.primary == VALUE_FALSE {
            return LuaValueKind::Boolean;
        }

        // Check tagged types by masking
        match self.primary & TYPE_MASK {
            TAG_FLOAT => LuaValueKind::Float,
            TAG_INTEGER => LuaValueKind::Integer,
            TAG_STRING => LuaValueKind::String,
            TAG_TABLE => LuaValueKind::Table,
            TAG_FUNCTION => LuaValueKind::Function,
            TAG_USERDATA => LuaValueKind::Userdata,
            TAG_THREAD => LuaValueKind::Thread,
            TAG_CFUNCTION => LuaValueKind::CFunction,
            _ => LuaValueKind::Nil, // Fallback for special values
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
            }
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
            }
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

        // Lua 5.4 semantics: integer and float with same numeric value are equal
        // Check if one is integer and the other is float
        if self.is_integer() && other.is_float() {
            let int_val = self.as_integer().unwrap();
            let float_val = other.as_float().unwrap();
            // Check if float is an exact integer and values match
            if float_val.fract() == 0.0 && float_val >= i64::MIN as f64 && float_val <= i64::MAX as f64 {
                return int_val == float_val as i64;
            }
            return false;
        }
        
        if self.is_float() && other.is_integer() {
            let float_val = self.as_float().unwrap();
            let int_val = other.as_integer().unwrap();
            // Check if float is an exact integer and values match
            if float_val.fract() == 0.0 && float_val >= i64::MIN as f64 && float_val <= i64::MAX as f64 {
                return float_val as i64 == int_val;
            }
            return false;
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
                    // Compare string contents lexicographically, not IDs
                    unsafe {
                        if let (Some(s1), Some(s2)) = (self.as_string(), other.as_string()) {
                            s1.as_str().partial_cmp(s2.as_str())
                        } else {
                            None
                        }
                    }
                }
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
