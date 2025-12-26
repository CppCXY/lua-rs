// Lua 5.4 compatible value representation
// 16 bytes, no pointer caching, all GC objects accessed via ID
pub mod chunk_serializer;
mod lua_table;
mod lua_thread;
mod lua_value;

use crate::LuaVM;
use crate::lua_vm::LuaResult;
use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::hash::Hasher;
use std::rc::Rc;

// Re-export the optimized LuaValue and type enum for pattern matching
pub use lua_table::LuaTable;
pub use lua_thread::*;
pub use lua_value::{LuaValue, LuaValueKind};

/// Multi-return values from Lua functions
/// OPTIMIZED: Compact enum representation (32 bytes)
/// - Empty: no return values
/// - Single: one value (no heap allocation, most common case)
/// - Many: 2+ values stored in Vec (heap allocation only when needed)
#[derive(Debug, Clone)]
pub enum MultiValue {
    Empty,
    Single(LuaValue),
    Many(Vec<LuaValue>),
}

impl MultiValue {
    #[inline(always)]
    pub fn empty() -> Self {
        MultiValue::Empty
    }

    #[inline(always)]
    pub fn single(value: LuaValue) -> Self {
        MultiValue::Single(value)
    }

    #[inline(always)]
    pub fn two(v1: LuaValue, v2: LuaValue) -> Self {
        MultiValue::Many(vec![v1, v2])
    }

    pub fn multiple(values: Vec<LuaValue>) -> Self {
        match values.len() {
            0 => MultiValue::Empty,
            1 => MultiValue::Single(values.into_iter().next().unwrap()),
            _ => MultiValue::Many(values),
        }
    }

    #[inline(always)]
    pub fn all_values(self) -> Vec<LuaValue> {
        match self {
            MultiValue::Empty => Vec::new(),
            MultiValue::Single(v) => vec![v],
            MultiValue::Many(v) => v,
        }
    }

    /// Get count of return values (optimized, no allocation)
    #[inline(always)]
    pub fn len(&self) -> usize {
        match self {
            MultiValue::Empty => 0,
            MultiValue::Single(_) => 1,
            MultiValue::Many(v) => v.len(),
        }
    }

    /// Check if empty
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        matches!(self, MultiValue::Empty)
    }

    /// Get first value (common case, optimized)
    #[inline(always)]
    pub fn first(&self) -> Option<LuaValue> {
        match self {
            MultiValue::Empty => None,
            MultiValue::Single(v) => Some(*v),
            MultiValue::Many(v) => v.first().copied(),
        }
    }

    /// Get second value
    #[inline(always)]
    pub fn second(&self) -> Option<LuaValue> {
        match self {
            MultiValue::Empty | MultiValue::Single(_) => None,
            MultiValue::Many(v) => v.get(1).copied(),
        }
    }

    /// Get value at index (0-based)
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<LuaValue> {
        match self {
            MultiValue::Empty => None,
            MultiValue::Single(v) => {
                if index == 0 {
                    Some(*v)
                } else {
                    None
                }
            }
            MultiValue::Many(v) => v.get(index).copied(),
        }
    }

    /// Copy values to a slice, filling with nil if needed
    /// Returns the number of values actually copied (before nil fill)
    #[inline(always)]
    pub fn copy_to_slice(&self, dest: &mut [LuaValue]) -> usize {
        let count = self.len().min(dest.len());
        match self {
            MultiValue::Empty => {}
            MultiValue::Single(v) => {
                if !dest.is_empty() {
                    dest[0] = *v;
                }
            }
            MultiValue::Many(v) => {
                for (i, val) in v.iter().take(dest.len()).enumerate() {
                    dest[i] = *val;
                }
            }
        }
        // Fill remaining with nil
        for slot in dest.iter_mut().skip(count) {
            *slot = LuaValue::nil();
        }
        count
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
        // Use FNV-1a hash for consistency with ObjectPool
        let bytes = s.as_bytes();
        let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
        for &byte in bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3); // FNV prime
        }

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
    pub linedefined: usize,           // Line where function starts (0 for main)
    pub lastlinedefined: usize,       // Line where function ends (0 for main)
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
            linedefined: 0,
            lastlinedefined: 0,
        }
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

        // MultiValue should be 24 bytes (Many variant: Vec<LuaValue> 24 bytes, Single is 16+tag)
        // Down from 64 bytes in original struct - 62.5% reduction!
        assert_eq!(size_of::<super::MultiValue>(), 24);
    }
}
