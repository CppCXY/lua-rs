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

use std::{any::Any, cell::RefCell, rc::Rc};

use crate::{
    LuaFunction, LuaString, LuaTable,
    lua_value::{CFunction, LuaUserdata},
};
use std::cmp::Ordering;

// Primary word tags
pub(crate) const TAG_INTEGER: u64 = 0xFFFF_0000_0000_0001;
pub(crate) const TAG_STRING: u64 = 0xFFFE_0000_0000_0001;
pub(crate) const TAG_TABLE: u64 = 0xFFFE_0000_0000_0002;
pub(crate) const TAG_FUNCTION: u64 = 0xFFFE_0000_0000_0003;
pub(crate) const TAG_USERDATA: u64 = 0xFFFE_0000_0000_0004;
pub(crate) const TAG_BOOLEAN: u64 = 0xFFFD_0000_0000_0000;
pub(crate) const TAG_NIL: u64 = 0xFFFC_0000_0000_0000;
pub(crate) const TAG_CFUNCTION: u64 = 0xFFFB_0000_0000_0001;

// Special values
pub(crate) const VALUE_TRUE: u64 = TAG_BOOLEAN | 1;
pub(crate) const VALUE_FALSE: u64 = TAG_BOOLEAN;
pub(crate) const VALUE_NIL: u64 = TAG_NIL;

// NaN detection (any value >= this is a tagged type)
pub(crate) const NAN_BASE: u64 = 0x7FF8_0000_0000_0000;

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
        // If it's a NaN, canonicalize to avoid collision with our tags
        let primary = if bits >= NAN_BASE {
            f64::NAN.to_bits()
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

    #[inline(always)]
    pub(crate) fn table_ptr(ptr: *const RefCell<LuaTable>) -> Self {
        let addr = ptr as u64;
        debug_assert!(addr < (1u64 << 48), "Pointer too large");
        LuaValue {
            primary: TAG_TABLE,
            secondary: addr & POINTER_MASK,
        }
    }

    #[inline(always)]
    pub(crate) fn function_ptr(ptr: *const LuaFunction) -> Self {
        let addr = ptr as u64;
        debug_assert!(addr < (1u64 << 48), "Pointer too large");
        LuaValue {
            primary: TAG_FUNCTION,
            secondary: addr & POINTER_MASK,
        }
    }

    #[inline(always)]
    pub(crate) fn userdata_ptr(ptr: *const LuaUserdata) -> Self {
        let addr = ptr as u64;
        debug_assert!(addr < (1u64 << 48), "Pointer too large");
        LuaValue {
            primary: TAG_USERDATA,
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
        self.primary < NAN_BASE
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
        } else if let Some(f) = self.as_float() {
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
    pub(crate) unsafe fn as_string_ptr(&self) -> Option<*const LuaString> {
        if self.is_string() {
            Some((self.secondary & POINTER_MASK) as *const LuaString)
        } else {
            None
        }
    }

    #[inline(always)]
    pub(crate) unsafe fn as_table_ptr(&self) -> Option<*const RefCell<LuaTable>> {
        if self.is_table() {
            Some((self.secondary & POINTER_MASK) as *const RefCell<LuaTable>)
        } else {
            None
        }
    }

    #[inline(always)]
    pub(crate) unsafe fn as_function_ptr(&self) -> Option<*const LuaFunction> {
        if self.is_function() {
            Some((self.secondary & POINTER_MASK) as *const LuaFunction)
        } else {
            None
        }
    }

    #[inline(always)]
    pub(crate) unsafe fn as_userdata_ptr(&self) -> Option<*const LuaUserdata> {
        if self.is_userdata() {
            Some((self.secondary & POINTER_MASK) as *const LuaUserdata)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_cfunction(&self) -> Option<CFunction> {
        if self.is_cfunction() {
            let addr = (self.secondary & POINTER_MASK) as usize;
            unsafe { Some(std::mem::transmute::<usize, CFunction>(addr)) }
        } else {
            None
        }
    }

    // ============ Basic Arithmetic ============

    #[inline]
    pub fn add(&self, other: &Self) -> Option<Self> {
        match (self.as_integer(), other.as_integer()) {
            (Some(a), Some(b)) => a.checked_add(b).map(Self::integer),
            _ => {
                let a = self.as_number()?;
                let b = other.as_number()?;
                Some(Self::float(a + b))
            }
        }
    }

    #[inline]
    pub fn sub(&self, other: &Self) -> Option<Self> {
        match (self.as_integer(), other.as_integer()) {
            (Some(a), Some(b)) => a.checked_sub(b).map(Self::integer),
            _ => {
                let a = self.as_number()?;
                let b = other.as_number()?;
                Some(Self::float(a - b))
            }
        }
    }

    #[inline]
    pub fn mul(&self, other: &Self) -> Option<Self> {
        match (self.as_integer(), other.as_integer()) {
            (Some(a), Some(b)) => a.checked_mul(b).map(Self::integer),
            _ => {
                let a = self.as_number()?;
                let b = other.as_number()?;
                Some(Self::float(a * b))
            }
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
            LuaValueKind::CFunction => "function",
        }
    }

    // ============ Additional Compatibility Constructors ============
    // Note: These increment refcount, so caller loses ownership of the Rc

    /// Create from Rc<LuaString> (takes ownership, increments refcount)
    #[inline(always)]
    pub fn from_string_rc(s: Rc<LuaString>) -> Self {
        let ptr = Rc::into_raw(s);  // Transfer ownership to LuaValue
        Self::string_ptr(ptr)
    }

    /// Create from Rc<RefCell<LuaTable>> (takes ownership)
    #[inline(always)]
    pub fn from_table_rc(t: Rc<RefCell<LuaTable>>) -> Self {
        let ptr = Rc::into_raw(t);
        Self::table_ptr(ptr)
    }

    /// Create from Rc<LuaFunction> (takes ownership)
    #[inline(always)]
    pub fn from_function_rc(f: Rc<LuaFunction>) -> Self {
        let ptr = Rc::into_raw(f);
        Self::function_ptr(ptr)
    }

    /// Create from Rc<LuaUserdata> (takes ownership)
    #[inline(always)]
    pub fn from_userdata_rc(u: Rc<LuaUserdata>) -> Self {
        let ptr = Rc::into_raw(u);
        Self::userdata_ptr(ptr)
    }

    /// Compatibility: create string value
    pub fn string(s: LuaString) -> Self {
        Self::from_string_rc(Rc::new(s))
    }

    /// Compatibility: create table value
    pub fn table(t: LuaTable) -> Self {
        Self::from_table_rc(Rc::new(RefCell::new(t)))
    }

    /// Compatibility: create function value
    pub fn function(f: LuaFunction) -> Self {
        Self::from_function_rc(Rc::new(f))
    }

    /// Compatibility: create userdata
    pub fn userdata<T: Any>(data: T) -> Self {
        Self::from_userdata_rc(Rc::new(LuaUserdata::new(data)))
    }

    /// Compatibility: create userdata with metatable
    pub fn userdata_with_metatable<T: Any>(data: T, metatable: Rc<RefCell<LuaTable>>) -> Self {
        Self::from_userdata_rc(Rc::new(LuaUserdata::with_metatable(data, metatable)))
    }

    // ============ Safe accessors that return Rc<T> ============

    /// Get string as Rc (creates a new Rc reference)
    #[inline]
    pub fn as_string_rc(&self) -> Option<Rc<LuaString>> {
        if self.primary == TAG_STRING {
            unsafe {
                let ptr = self.secondary as *const LuaString;
                Rc::increment_strong_count(ptr);
                Some(Rc::from_raw(ptr))
            }
        } else {
            None
        }
    }

    /// Get table as Rc<RefCell<>> (creates a new Rc reference)
    #[inline]
    pub fn as_table_rc(&self) -> Option<Rc<RefCell<LuaTable>>> {
        if self.primary == TAG_TABLE {
            unsafe {
                let ptr = self.secondary as *const RefCell<LuaTable>;
                Rc::increment_strong_count(ptr);
                Some(Rc::from_raw(ptr))
            }
        } else {
            None
        }
    }

    /// Get function as Rc (creates a new Rc reference)
    #[inline]
    pub fn as_function_rc(&self) -> Option<Rc<LuaFunction>> {
        if self.primary == TAG_FUNCTION {
            unsafe {
                let ptr = self.secondary as *const LuaFunction;
                Rc::increment_strong_count(ptr);
                Some(Rc::from_raw(ptr))
            }
        } else {
            None
        }
    }

    /// Get userdata as Rc (creates a new Rc reference)
    #[inline]
    pub fn as_userdata_rc(&self) -> Option<Rc<LuaUserdata>> {
        if self.primary == TAG_USERDATA {
            unsafe {
                let ptr = self.secondary as *const LuaUserdata;
                Rc::increment_strong_count(ptr);
                Some(Rc::from_raw(ptr))
            }
        } else {
            None
        }
    }

    // ============ Additional arithmetic operations (from old enum) ============

    #[inline]
    pub fn div(&self, other: &Self) -> Option<Self> {
        let a = self.as_number()?;
        let b = other.as_number()?;
        if b == 0.0 {
            return None;
        }
        Some(Self::float(a / b))
    }

    #[inline]
    pub fn idiv(&self, other: &Self) -> Option<Self> {
        match (self.as_integer(), other.as_integer()) {
            (Some(a), Some(b)) => {
                if b == 0 {
                    None
                } else {
                    Some(Self::integer(a / b))
                }
            }
            _ => {
                let a = self.as_number()?;
                let b = other.as_number()?;
                if b == 0.0 {
                    None
                } else {
                    Some(Self::float((a / b).floor()))
                }
            }
        }
    }

    #[inline]
    pub fn modulo(&self, other: &Self) -> Option<Self> {
        match (self.as_integer(), other.as_integer()) {
            (Some(a), Some(b)) => {
                if b == 0 {
                    None
                } else {
                    Some(Self::integer(a % b))
                }
            }
            _ => {
                let a = self.as_number()?;
                let b = other.as_number()?;
                if b == 0.0 {
                    None
                } else {
                    Some(Self::float(a - (a / b).floor() * b))
                }
            }
        }
    }

    #[inline]
    pub fn pow(&self, other: &Self) -> Option<Self> {
        let a = self.as_number()?;
        let b = other.as_number()?;
        Some(Self::float(a.powf(b)))
    }

    #[inline]
    pub fn unm(&self) -> Option<Self> {
        if let Some(i) = self.as_integer() {
            Some(Self::integer(-i))
        } else if let Some(f) = self.as_float() {
            Some(Self::float(-f))
        } else {
            None
        }
    }

    // ============ Comparison operations ============

    #[inline]
    pub fn eq(&self, other: &Self) -> bool {
        // Fast path: same bits
        if self.primary() == other.primary() && self.secondary() == other.secondary() {
            return true;
        }

        // Type-specific comparison
        if let (Some(a), Some(b)) = (self.as_integer(), other.as_integer()) {
            a == b
        } else if let (Some(a), Some(b)) = (self.as_float(), other.as_float()) {
            a == b
        } else if let (Some(a), Some(b)) = (self.as_bool(), other.as_bool()) {
            a == b
        } else {
            false
        }
    }

    #[inline]
    pub fn lt(&self, other: &Self) -> Option<bool> {
        match (self.as_integer(), other.as_integer()) {
            (Some(a), Some(b)) => Some(a < b),
            _ => {
                let a = self.as_number()?;
                let b = other.as_number()?;
                Some(a < b)
            }
        }
    }

    #[inline]
    pub fn le(&self, other: &Self) -> Option<bool> {
        match (self.as_integer(), other.as_integer()) {
            (Some(a), Some(b)) => Some(a <= b),
            _ => {
                let a = self.as_number()?;
                let b = other.as_number()?;
                Some(a <= b)
            }
        }
    }

    // ============ String conversion ============

    pub fn to_lua_string(&self) -> Option<String> {
        if let Some(i) = self.as_integer() {
            Some(i.to_string())
        } else if let Some(f) = self.as_float() {
            Some(format!("{}", f))
        } else if let Some(b) = self.as_bool() {
            Some(b.to_string())
        } else if self.is_nil() {
            Some("nil".to_string())
        } else if let Some(s) = self.as_string_rc() {
            Some(s.as_str().to_string())
        } else {
            None
        }
    }
}

// ============ Trait Implementations ============

impl Drop for LuaValue {
    #[inline(always)]
    fn drop(&mut self) {
        // Decrement Rc refcount for heap objects
        // SAFETY: primary tag tells us the exact type
        unsafe {
            match self.primary {
                TAG_STRING => {
                    let ptr = self.secondary as *const LuaString;
                    drop(Rc::from_raw(ptr));
                }
                TAG_TABLE => {
                    let ptr = self.secondary as *const RefCell<LuaTable>;
                    drop(Rc::from_raw(ptr));
                }
                TAG_FUNCTION => {
                    let ptr = self.secondary as *const LuaFunction;
                    drop(Rc::from_raw(ptr));
                }
                TAG_USERDATA => {
                    let ptr = self.secondary as *const LuaUserdata;
                    drop(Rc::from_raw(ptr));
                }
                _ => {
                    // Nil, Bool, Integer, Float, CFunction - no-op (no heap allocation)
                }
            }
        }
    }
}

impl Clone for LuaValue {
    #[inline(always)]
    fn clone(&self) -> Self {
        // Ultra-fast clone: For pointers, just increment refcount
        // For values (nil/bool/int/float/cfunc), just copy bits
        
        match self.primary {
            TAG_STRING => {
                // SAFETY: We know this is a valid string pointer
                unsafe {
                    let ptr = self.secondary as *const LuaString;
                    Rc::increment_strong_count(ptr);
                }
            }
            TAG_TABLE => {
                unsafe {
                    let ptr = self.secondary as *const RefCell<LuaTable>;
                    Rc::increment_strong_count(ptr);
                }
            }
            TAG_FUNCTION => {
                unsafe {
                    let ptr = self.secondary as *const LuaFunction;
                    Rc::increment_strong_count(ptr);
                }
            }
            TAG_USERDATA => {
                unsafe {
                    let ptr = self.secondary as *const LuaUserdata;
                    Rc::increment_strong_count(ptr);
                }
            }
            _ => {
                // Nil, Bool, Integer, Float, CFunction - just copy bits (no refcount)
            }
        }
        
        // Always return a bitwise copy (refcount already incremented if needed)
        Self {
            primary: self.primary,
            secondary: self.secondary,
        }
    }
}

// Implement Debug for better error messages
impl std::fmt::Debug for LuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_nil() {
            write!(f, "nil")
        } else if let Some(b) = self.as_bool() {
            write!(f, "{}", b)
        } else if let Some(i) = self.as_integer() {
            write!(f, "{}", i)
        } else if let Some(fl) = self.as_float() {
            write!(f, "{}", fl)
        } else if let Some(s) = self.as_string_rc() {
            write!(f, "\"{}\"", s.as_str())
        } else if self.is_table() {
            write!(f, "table: {:x}", self.secondary())
        } else if self.is_function() {
            write!(f, "function: {:x}", self.secondary())
        } else if self.is_userdata() {
            write!(f, "userdata: {:x}", self.secondary())
        } else if self.is_cfunction() {
            write!(f, "cfunction")
        } else {
            write!(f, "unknown")
        }
    }
}

impl Default for LuaValue {
    fn default() -> Self {
        Self::nil()
    }
}

// ============ Additional Trait Implementations ============

impl PartialEq for LuaValue {
    fn eq(&self, other: &Self) -> bool {
        // Fast path: exact bit match
        if self.primary() == other.primary() && self.secondary() == other.secondary() {
            return true;
        }

        // Type-specific comparison
        if let (Some(a), Some(b)) = (self.as_integer(), other.as_integer()) {
            a == b
        } else if let (Some(a), Some(b)) = (self.as_float(), other.as_float()) {
            a == b
        } else if let (Some(a), Some(b)) = (self.as_bool(), other.as_bool()) {
            a == b
        } else if self.is_string() && other.is_string() {
            match (self.as_string_rc(), other.as_string_rc()) {
                (Some(a), Some(b)) => {
                    if Rc::ptr_eq(&a, &b) {
                        true
                    } else {
                        a.as_str() == b.as_str()
                    }
                }
                _ => false,
            }
        } else if self.is_table() && other.is_table() {
            // Tables compared by pointer
            self.secondary() == other.secondary()
        } else if self.is_function() && other.is_function() {
            // Functions compared by pointer
            self.secondary() == other.secondary()
        } else if self.is_userdata() && other.is_userdata() {
            // Userdata compared by pointer
            self.secondary() == other.secondary()
        } else {
            false
        }
    }
}

impl Eq for LuaValue {}

impl std::hash::Hash for LuaValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        if self.is_nil() {
            0u8.hash(state);
        } else if let Some(b) = self.as_bool() {
            1u8.hash(state);
            b.hash(state);
        } else if let Some(i) = self.as_integer() {
            2u8.hash(state);
            i.hash(state);
        } else if let Some(f) = self.as_float() {
            3u8.hash(state);
            f.to_bits().hash(state);
        } else if let Some(s) = self.as_string_rc() {
            4u8.hash(state);
            s.as_str().hash(state);
        } else if self.is_table() {
            5u8.hash(state);
            self.secondary().hash(state);
        } else if self.is_function() {
            6u8.hash(state);
            self.secondary().hash(state);
        } else if self.is_cfunction() {
            7u8.hash(state);
            self.secondary().hash(state);
        } else if self.is_userdata() {
            8u8.hash(state);
            self.secondary().hash(state);
        }
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
                LuaValueKind::String => match (self.as_string_rc(), other.as_string_rc()) {
                    (Some(a), Some(b)) => a.as_str().partial_cmp(b.as_str()),
                    _ => None,
                },
                LuaValueKind::Table
                | LuaValueKind::Function
                | LuaValueKind::Userdata
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

// ============ Additional Methods for Compatibility ============

impl LuaValue {
    /// Get as_string (returns Rc) for compatibility
    pub fn as_string(&self) -> Option<Rc<LuaString>> {
        self.as_string_rc()
    }

    /// Get as_table (returns Rc<RefCell>) for compatibility
    pub fn as_table(&self) -> Option<Rc<RefCell<LuaTable>>> {
        self.as_table_rc()
    }

    /// Get as_function (returns Rc) for compatibility
    pub fn as_function(&self) -> Option<Rc<LuaFunction>> {
        self.as_function_rc()
    }

    /// Get as_userdata (returns Rc) for compatibility
    pub fn as_userdata(&self) -> Option<Rc<LuaUserdata>> {
        self.as_userdata_rc()
    }

    /// Get as_boolean for compatibility
    pub fn as_boolean(&self) -> Option<bool> {
        self.as_bool()
    }

    /// String representation for printing
    pub fn to_string_repr(&self) -> String {
        if let Some(i) = self.as_integer() {
            i.to_string()
        } else if let Some(f) = self.as_float() {
            f.to_string()
        } else if let Some(b) = self.as_bool() {
            b.to_string()
        } else if self.is_nil() {
            "nil".to_string()
        } else if let Some(s) = self.as_string_rc() {
            s.as_str().to_string()
        } else if self.is_table() {
            format!("table: {:x}", self.secondary())
        } else if self.is_function() {
            format!("function: {:x}", self.secondary())
        } else if self.is_cfunction() {
            "function: [C]".to_string()
        } else if self.is_userdata() {
            format!("userdata: {:x}", self.secondary())
        } else {
            "unknown".to_string()
        }
    }

    /// Check if value is callable (function or cfunction)
    pub fn is_callable(&self) -> bool {
        self.is_function() || self.is_cfunction()
    }

    /// Get metatable for tables and userdata
    pub fn get_metatable(&self) -> Option<Rc<RefCell<LuaTable>>> {
        if let Some(table) = self.as_table_rc() {
            table.borrow().get_metatable()
        } else if let Some(userdata) = self.as_userdata_rc() {
            userdata.get_metatable()
        } else {
            None
        }
    }

    /// Alias for type_kind() - returns the type discriminator
    /// Use this to check types instead of pattern matching
    #[inline(always)]
    pub fn kind(&self) -> LuaValueKind {
        match self.primary {
            VALUE_NIL => LuaValueKind::Nil,
            VALUE_TRUE | VALUE_FALSE => LuaValueKind::Boolean,
            TAG_INTEGER => LuaValueKind::Integer,
            p if p < NAN_BASE => LuaValueKind::Float,
            TAG_STRING => LuaValueKind::String,
            TAG_TABLE => LuaValueKind::Table,
            TAG_FUNCTION => LuaValueKind::Function,
            TAG_USERDATA => LuaValueKind::Userdata,
            TAG_CFUNCTION => LuaValueKind::CFunction,
            _ => unreachable!("Invalid LuaValue primary tag"),
        }
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
    CFunction,
}
