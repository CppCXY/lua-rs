// Lua 5.4 compatible value representation
// 16 bytes, no pointer caching, all GC objects accessed via ID
mod lua_table;
mod lua_thread;
mod lua_value;

use crate::LuaVM;
use crate::lua_vm::LuaResult;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::Hasher;
use std::rc::Rc;

// Re-export the optimized LuaValue and type enum for pattern matching
pub use lua_table::LuaTable;
pub use lua_thread::*;
pub use lua_value::{
    ID_MASK,
    LuaValue,
    LuaValueKind,
    NAN_BASE,
    TAG_BOOLEAN,
    TAG_CFUNCTION,
    TAG_FALSE,
    TAG_FLOAT,
    TAG_FUNCTION,
    TAG_INTEGER,
    TAG_MASK,
    TAG_NIL,
    TAG_STRING,
    TAG_TABLE,
    TAG_THREAD,
    TAG_TRUE,
    TAG_UPVALUE,
    TAG_USERDATA,
    // Compatibility aliases
    TYPE_MASK,
    VALUE_FALSE,
    VALUE_NIL,
    VALUE_TRUE,
};

/// Multi-return values from Lua functions
/// OPTIMIZED: Use inline storage for common single-value case
#[derive(Debug, Clone)]
pub struct MultiValue {
    // Inline storage for 0-2 values (covers 99% of cases without heap allocation)
    pub inline: [LuaValue; 2],
    // Count of values stored inline (0, 1, or 2)
    pub inline_count: u8,
    // Only used when > 2 values
    pub overflow: Option<Vec<LuaValue>>,
}

impl MultiValue {
    #[inline(always)]
    pub fn empty() -> Self {
        MultiValue {
            inline: [LuaValue::nil(), LuaValue::nil()],
            inline_count: 0,
            overflow: None,
        }
    }

    #[inline(always)]
    pub fn single(value: LuaValue) -> Self {
        MultiValue {
            inline: [value, LuaValue::nil()],
            inline_count: 1,
            overflow: None,
        }
    }

    #[inline(always)]
    pub fn two(v1: LuaValue, v2: LuaValue) -> Self {
        MultiValue {
            inline: [v1, v2],
            inline_count: 2,
            overflow: None,
        }
    }

    pub fn multiple(values: Vec<LuaValue>) -> Self {
        let len = values.len();
        if len == 0 {
            Self::empty()
        } else if len == 1 {
            Self::single(values.into_iter().next().unwrap())
        } else if len == 2 {
            let mut iter = values.into_iter();
            Self::two(iter.next().unwrap(), iter.next().unwrap())
        } else {
            MultiValue {
                inline: [LuaValue::nil(), LuaValue::nil()],
                inline_count: 0,
                overflow: Some(values),
            }
        }
    }

    #[inline(always)]
    pub fn all_values(self) -> Vec<LuaValue> {
        if let Some(v) = self.overflow {
            v
        } else {
            match self.inline_count {
                0 => Vec::new(),
                1 => vec![self.inline[0]],
                2 => vec![self.inline[0], self.inline[1]],
                _ => Vec::new(),
            }
        }
    }

    /// Get count of return values (optimized, no allocation)
    #[inline(always)]
    pub fn len(&self) -> usize {
        if let Some(ref v) = self.overflow {
            v.len()
        } else {
            self.inline_count as usize
        }
    }

    /// Check if empty
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.inline_count == 0 && self.overflow.is_none()
    }

    /// Get first value (common case, optimized)
    #[inline(always)]
    pub fn first(&self) -> Option<LuaValue> {
        if let Some(ref v) = self.overflow {
            v.first().copied()
        } else if self.inline_count > 0 {
            Some(self.inline[0])
        } else {
            None
        }
    }
}

/// C Function type - Rust function callable from Lua
pub type CFunction = fn(&mut LuaVM) -> LuaResult<MultiValue>;

/// Lua string (immutable, interned with cached hash)
#[derive(Debug, Clone)]
pub struct LuaString {
    hash: u64, // Keep hash first for alignment
    data: String,
}

impl LuaString {
    pub fn new(s: String) -> Self {
        use fxhash::FxHasher;
        use std::hash::Hasher;
        let mut hasher = FxHasher::default();
        hasher.write(s.as_bytes());
        let hash = hasher.finish();

        LuaString { data: s, hash }
    }

    /// Create LuaString with pre-computed hash (avoids double hashing)
    #[inline]
    pub fn with_hash(s: String, hash: u64) -> Self {
        LuaString { data: s, hash }
    }

    pub fn as_str(&self) -> &str {
        &self.data
    }

    #[inline]
    pub fn cached_hash(&self) -> u64 {
        self.hash
    }
}

impl PartialEq for LuaString {
    fn eq(&self, other: &Self) -> bool {
        // Fast path: compare hashes first
        if self.hash != other.hash {
            return false;
        }
        self.data == other.data
    }
}

impl Eq for LuaString {}

impl std::hash::Hash for LuaString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Use cached hash
        state.write_u64(self.hash);
    }
}

/// Lua function
/// Runtime upvalue - can be open (pointing to stack) or closed (owns value)
/// This matches Lua's UpVal implementation
pub struct LuaUpvalue {
    value: RefCell<UpvalueState>,
}

#[derive(Debug)]
enum UpvalueState {
    Open {
        stack_index: usize, // Absolute index in register_stack
    },
    Closed(LuaValue), // Value moved to heap after frame exits
}

impl LuaUpvalue {
    /// Create an open upvalue pointing to a stack location (absolute index)
    pub fn new_open(stack_index: usize) -> Rc<Self> {
        Rc::new(LuaUpvalue {
            value: RefCell::new(UpvalueState::Open { stack_index }),
        })
    }

    /// Create an open upvalue with frame base + register (computes absolute index)
    pub fn new_open_relative(base_ptr: usize, register: usize) -> Rc<Self> {
        Rc::new(LuaUpvalue {
            value: RefCell::new(UpvalueState::Open {
                stack_index: base_ptr + register,
            }),
        })
    }

    /// Create a closed upvalue with an owned value
    pub fn new_closed(value: LuaValue) -> Rc<Self> {
        Rc::new(LuaUpvalue {
            value: RefCell::new(UpvalueState::Closed(value)),
        })
    }

    /// Check if this upvalue is open
    pub fn is_open(&self) -> bool {
        matches!(*self.value.borrow(), UpvalueState::Open { .. })
    }

    /// Check if this upvalue points to a specific stack location (absolute index)
    pub fn points_to_index(&self, index: usize) -> bool {
        match *self.value.borrow() {
            UpvalueState::Open { stack_index } => stack_index == index,
            _ => false,
        }
    }

    /// Get the stack index if open (for comparison during close)
    pub fn get_stack_index(&self) -> Option<usize> {
        match *self.value.borrow() {
            UpvalueState::Open { stack_index } => Some(stack_index),
            _ => None,
        }
    }

    /// Close this upvalue (move value from stack to heap)
    pub fn close(&self, stack_value: LuaValue) {
        let mut state = self.value.borrow_mut();
        if matches!(*state, UpvalueState::Open { .. }) {
            *state = UpvalueState::Closed(stack_value);
        }
    }

    /// Get the value (requires register_stack if open)
    pub fn get_value(&self, register_stack: &[LuaValue]) -> LuaValue {
        let state = self.value.borrow();
        match *state {
            UpvalueState::Open { stack_index } => {
                if stack_index < register_stack.len() {
                    register_stack[stack_index]
                } else {
                    LuaValue::nil()
                }
            }
            UpvalueState::Closed(ref val) => val.clone(),
        }
    }

    /// Set the value (requires register_stack if open)
    pub fn set_value(&self, register_stack: &mut [LuaValue], value: LuaValue) {
        let state = self.value.borrow();
        match *state {
            UpvalueState::Open { stack_index } => {
                drop(state);
                if stack_index < register_stack.len() {
                    register_stack[stack_index] = value;
                }
            }
            UpvalueState::Closed(_) => {
                drop(state);
                *self.value.borrow_mut() = UpvalueState::Closed(value);
            }
        }
    }

    /// Get the closed value for GC marking (returns None if open)
    pub fn get_closed_value(&self) -> Option<LuaValue> {
        match *self.value.borrow() {
            UpvalueState::Closed(ref val) => Some(val.clone()),
            _ => None,
        }
    }

    /// Fast path: Try to get value directly if closed (avoids frame lookup)
    /// Returns None if the upvalue is open
    #[inline(always)]
    pub fn try_get_closed(&self) -> Option<LuaValue> {
        if let Ok(state) = self.value.try_borrow() {
            match *state {
                UpvalueState::Closed(ref val) => Some(*val),
                _ => None,
            }
        } else {
            None
        }
    }
}

impl fmt::Debug for LuaUpvalue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self.value.borrow() {
            UpvalueState::Open { stack_index } => {
                write!(f, "Upvalue::Open(idx={})", stack_index)
            }
            UpvalueState::Closed(ref val) => {
                write!(f, "Upvalue::Closed({:?})", val)
            }
        }
    }
}

/// Userdata - arbitrary Rust data with optional metatable
#[derive(Clone)]
pub struct LuaUserdata {
    data: Rc<RefCell<Box<dyn Any>>>,
    metatable: LuaValue,
}

impl LuaUserdata {
    pub fn new<T: Any>(data: T) -> Self {
        LuaUserdata {
            data: Rc::new(RefCell::new(Box::new(data))),
            metatable: LuaValue::nil(),
        }
    }

    pub fn with_metatable<T: Any>(data: T, metatable: LuaValue) -> Self {
        LuaUserdata {
            data: Rc::new(RefCell::new(Box::new(data))),
            metatable,
        }
    }

    pub fn get_data(&self) -> Rc<RefCell<Box<dyn Any>>> {
        self.data.clone()
    }

    pub fn get_metatable(&self) -> LuaValue {
        self.metatable
    }

    pub fn set_metatable(&mut self, metatable: LuaValue) {
        self.metatable = metatable;
    }
}

impl fmt::Debug for LuaUserdata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Userdata({:p})", self.data.as_ptr())
    }
}

pub struct LuaFunction {
    pub chunk: Rc<Chunk>,
    pub upvalues: Vec<Rc<LuaUpvalue>>,
}

/// Upvalue descriptor
#[derive(Debug, Clone)]
pub struct UpvalueDesc {
    pub is_local: bool, // true if captures parent local, false if captures parent upvalue
    pub index: u32,     // index in parent's register or upvalue array
}

/// Compiled chunk (bytecode + metadata)
#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<u32>,
    pub constants: Vec<LuaValue>,
    pub locals: Vec<String>,
    pub upvalue_count: usize,
    pub param_count: usize,
    pub is_vararg: bool, // Whether function uses ... (varargs)
    pub max_stack_size: usize,
    pub child_protos: Vec<Rc<Chunk>>, // Nested function prototypes
    pub upvalue_descs: Vec<UpvalueDesc>, // Upvalue descriptors
    pub source_name: Option<String>,  // Source file/chunk name for debugging
    pub line_info: Vec<u32>,          // Line number for each instruction (for debug)
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            locals: Vec::new(),
            upvalue_count: 0,
            param_count: 0,
            is_vararg: false,
            max_stack_size: 0,
            child_protos: Vec::new(),
            upvalue_descs: Vec::new(),
            source_name: None,
            line_info: Vec::new(),
        }
    }
}

/// String interning pool for short strings (Lua 5.4 optimization)
/// Short strings (≤ LUAI_MAXSHORTLEN) are interned to save memory and speed up comparisons
pub struct StringPool {
    /// Maximum length for short strings (typically 40 bytes in Lua)
    max_short_len: usize,
    /// Interned short strings - key is the string content, value is Rc
    pool: HashMap<String, Rc<LuaString>>,
}

impl StringPool {
    /// Create a new string pool with default max short length
    pub fn new() -> Self {
        Self::with_max_len(40)
    }

    /// Create a new string pool with custom max short length
    pub fn with_max_len(max_short_len: usize) -> Self {
        StringPool {
            max_short_len,
            pool: HashMap::new(),
        }
    }

    /// Intern a string. If it's short and already exists, return the cached version.
    /// Otherwise create a new string.
    pub fn intern(&mut self, s: String) -> Rc<LuaString> {
        if s.len() <= self.max_short_len {
            // Short string: check pool first
            if let Some(existing) = self.pool.get(&s) {
                return Rc::clone(existing);
            }

            // Not in pool: create and insert
            let lua_str = Rc::new(LuaString::new(s.clone()));
            self.pool.insert(s, Rc::clone(&lua_str));
            lua_str
        } else {
            // Long string: don't intern, create directly
            Rc::new(LuaString::new(s))
        }
    }

    /// Get statistics about the string pool
    pub fn stats(&self) -> (usize, usize) {
        let count = self.pool.len();
        let bytes: usize = self.pool.keys().map(|s| s.len()).sum();
        (count, bytes)
    }

    /// Clear the string pool (useful for testing or memory cleanup)
    pub fn clear(&mut self) {
        self.pool.clear();
    }
}

impl Default for StringPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod string_pool_tests {
    use super::*;

    #[test]
    fn test_short_string_interning() {
        let mut pool = StringPool::new();

        let s1 = pool.intern("hello".to_string());
        let s2 = pool.intern("hello".to_string());

        // Should be the same Rc instance
        assert!(Rc::ptr_eq(&s1, &s2));
        assert_eq!(pool.stats().0, 1); // Only 1 unique string
    }

    #[test]
    fn test_long_string_no_interning() {
        let mut pool = StringPool::with_max_len(10);

        let long_str = "a".repeat(50);
        let s1 = pool.intern(long_str.clone());
        let s2 = pool.intern(long_str);

        // Long strings are NOT interned
        assert!(!Rc::ptr_eq(&s1, &s2));
        assert_eq!(pool.stats().0, 0); // No strings in pool
    }

    #[test]
    fn test_multiple_short_strings() {
        let mut pool = StringPool::new();

        let _ = pool.intern("foo".to_string());
        let _ = pool.intern("bar".to_string());
        let _ = pool.intern("foo".to_string()); // Duplicate
        let _ = pool.intern("baz".to_string());

        assert_eq!(pool.stats().0, 3); // 3 unique strings
    }
}

#[cfg(test)]
mod value_tests {
    use super::*;

    #[test]
    fn test_integer_float_distinction() {
        let int_val = LuaValue::integer(42);
        let float_val = LuaValue::number(42.0);

        assert!(int_val.is_integer());
        assert!(!int_val.is_float());
        assert!(!float_val.is_integer()); // 42.0 is a float, not an integer
        assert!(float_val.is_float());

        // Both are numbers
        assert!(int_val.is_number());
        assert!(float_val.is_number());
    }

    #[test]
    fn test_integer_float_conversion() {
        let int_val = LuaValue::integer(42);
        let float_val = LuaValue::number(42.5);

        // Integer can convert to float via as_float
        assert_eq!(int_val.as_float(), Some(42.0));

        // Float with fraction cannot convert to integer
        assert_eq!(float_val.as_integer(), None);

        // Float without fraction can convert to integer
        let exact_float = LuaValue::number(42.0);
        assert_eq!(exact_float.as_integer(), Some(42));
    }

    #[test]
    fn test_as_number_unified() {
        let int_val = LuaValue::integer(42);
        let float_val = LuaValue::number(3.14);

        // as_number works for both
        assert_eq!(int_val.as_number(), Some(42.0));
        assert_eq!(float_val.as_number(), Some(3.14));
    }

    #[test]
    fn test_value_size() {
        use std::mem::size_of;

        // LuaValue should be 16 bytes (enum discriminant + largest variant)
        assert_eq!(size_of::<LuaValue>(), 16);
    }
}
