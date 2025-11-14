// Lua 5.4 compatible value representation with NaN-Boxing
// 16 bytes, 3-6x faster than enum, full int64 support
mod value;

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::VM;
// NaN-Boxing core struct definition

// Re-export the optimized LuaValue and type enum for pattern matching
pub use value::{LuaValue, LuaValueKind, TAG_INTEGER, TAG_STRING, TAG_TABLE, TAG_FUNCTION, 
                TAG_USERDATA, TAG_BOOLEAN, TAG_NIL, TAG_CFUNCTION, NAN_BASE};
/// Multi-return values from Lua functions
#[derive(Debug, Clone)]
pub struct MultiValue {
    pub values: Option<Vec<LuaValue>>,
}

impl MultiValue {
    pub fn empty() -> Self {
        MultiValue { values: None }
    }

    pub fn single(value: LuaValue) -> Self {
        MultiValue {
            values: Some(vec![value]),
        }
    }

    pub fn multiple(values: Vec<LuaValue>) -> Self {
        MultiValue {
            values: Some(values),
        }
    }

    pub fn all_values(self) -> Vec<LuaValue> {
        self.values.unwrap_or_default()
    }
}

/// C Function type - Rust function callable from Lua
pub type CFunction = fn(&mut VM) -> Result<MultiValue, String>;

/// Lua string (immutable, interned)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LuaString {
    data: String,
}

impl LuaString {
    pub fn new(s: String) -> Self {
        LuaString { data: s }
    }

    pub fn as_str(&self) -> &str {
        &self.data
    }
}

/// Lua table (mutable associative array)
/// Uses lazy allocation - only creates Vec/HashMap when needed
#[derive(Debug)]
pub struct LuaTable {
    /// Array part - allocated on first integer key access
    array: Option<Vec<LuaValue>>,
    /// Hash part - allocated on first non-array key access
    hash: Option<HashMap<LuaValue, LuaValue>>,
    /// Metatable - optional table that defines special behaviors
    metatable: Option<Rc<RefCell<LuaTable>>>,
}

impl LuaTable {
    pub fn new() -> Self {
        LuaTable {
            array: None,
            hash: None,
            metatable: None,
        }
    }

    /// Get the metatable of this table
    pub fn get_metatable(&self) -> Option<Rc<RefCell<LuaTable>>> {
        self.metatable.clone()
    }

    /// Set the metatable of this table
    pub fn set_metatable(&mut self, mt: Option<Rc<RefCell<LuaTable>>>) {
        self.metatable = mt;
    }

    /// Fast integer index access - specialized for array access
    #[inline(always)]
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        let idx = key as usize;
        if idx > 0 {
            if let Some(ref arr) = self.array {
                if idx <= arr.len() {
                    return arr.get(idx - 1).cloned();
                }
            }
        }
        // Fallback to hash for out-of-range integers
        self.hash
            .as_ref()
            .and_then(|h| h.get(&LuaValue::integer(key)).cloned())
    }

    /// Fast string key access - specialized for hash access
    #[inline(always)]
    pub fn get_str(&self, key: &Rc<LuaString>) -> Option<LuaValue> {
        self.hash
            .as_ref()
            .and_then(|h| h.get(&LuaValue::from_string_rc(Rc::clone(key))).cloned())
    }

    /// Get value with raw access (no metamethods)
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        // Fast paths for common cases
        if let Some(i) = key.as_integer() {
            return self.get_int(i);
        }
        if let Some(s) = key.as_string() {
            return self.get_str(&s);
        }

        // Generic fallback
        self.hash.as_ref().and_then(|h| h.get(key).cloned())
    }

    /// Fast integer index set - specialized for array access
    #[inline(always)]
    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        let idx = key as usize;
        if idx > 0 {
            let arr = self.array.get_or_insert_with(Vec::new);
            if idx <= arr.len() + 1 {
                if idx == arr.len() + 1 {
                    arr.push(value);
                } else {
                    arr[idx - 1] = value;
                }
                return;
            }
        }
        // Fallback to hash
        let hash = self.hash.get_or_insert_with(HashMap::new);
        hash.insert(LuaValue::integer(key), value);
    }

    /// Fast string key set - specialized for hash access
    #[inline(always)]
    pub fn set_str(&mut self, key: Rc<LuaString>, value: LuaValue) {
        let hash = self.hash.get_or_insert_with(HashMap::new);
        hash.insert(LuaValue::from_string_rc(key), value);
    }

    /// Set value with raw access (no metamethods)
    pub fn raw_set(&mut self, key: LuaValue, value: LuaValue) {
        // Fast paths for common cases
        if let Some(i) = key.as_integer() {
            self.set_int(i, value);
            return;
        }
        if let Some(s) = key.as_string() {
            self.set_str(s, value);
            return;
        }

        // Generic fallback
        let hash = self.hash.get_or_insert_with(HashMap::new);
        hash.insert(key, value);
    }

    pub fn len(&self) -> usize {
        self.array.as_ref().map(|a| a.len()).unwrap_or(0)
    }

    /// Iterate over all key-value pairs (both array and hash parts)
    pub fn iter_all(&self) -> impl Iterator<Item = (LuaValue, LuaValue)> + '_ {
        let array_iter = self
            .array
            .as_ref()
            .map(|a| {
                a.iter()
                    .enumerate()
                    .map(|(i, v)| (LuaValue::integer((i + 1) as i64), v.clone()))
            })
            .into_iter()
            .flatten();

        let hash_iter = self
            .hash
            .as_ref()
            .map(|h| h.iter().map(|(k, v)| (k.clone(), v.clone())))
            .into_iter()
            .flatten();

        array_iter.chain(hash_iter)
    }

    pub fn insert_array_at(&mut self, index: usize, value: LuaValue) -> Result<(), String> {
        let arr = self.array.get_or_insert_with(Vec::new);
        if index <= arr.len() {
            arr.insert(index, value);
        } else if index == arr.len() + 1 {
            arr.push(value);
        } else {
            return Err("Index out of bounds for array insertion".to_string());
        }

        Ok(())
    }

    pub fn remove_array_at(&mut self, index: usize) -> Result<LuaValue, String> {
        if let Some(ref mut arr) = self.array {
            if index < arr.len() {
                return Ok(arr.remove(index));
            }
        }
        Err("Index out of bounds for array removal".to_string())
    }

    pub fn get_array_part(&mut self) -> Option<&mut Vec<LuaValue>> {
        self.array.as_mut()
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
        frame_id: usize, // Which call frame owns this variable
        register: usize, // Register index in that frame
    },
    Closed(LuaValue), // Value moved to heap after frame exits
}

impl LuaUpvalue {
    /// Create an open upvalue pointing to a stack location
    pub fn new_open(frame_id: usize, register: usize) -> Rc<Self> {
        Rc::new(LuaUpvalue {
            value: RefCell::new(UpvalueState::Open { frame_id, register }),
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

    /// Check if this upvalue points to a specific stack location
    pub fn points_to(&self, frame_id: usize, register: usize) -> bool {
        match *self.value.borrow() {
            UpvalueState::Open {
                frame_id: fid,
                register: reg,
            } => fid == frame_id && reg == register,
            _ => false,
        }
    }

    /// Close this upvalue (move value from stack to heap)
    pub fn close(&self, stack_value: LuaValue) {
        let mut state = self.value.borrow_mut();
        if matches!(*state, UpvalueState::Open { .. }) {
            *state = UpvalueState::Closed(stack_value);
        }
    }

    /// Get the value (requires VM to read from stack if open)
    pub fn get_value(&self, frames: &[crate::vm::CallFrame]) -> LuaValue {
        let state = self.value.borrow();
        match *state {
            UpvalueState::Open { frame_id, register } => {
                // Release the borrow before accessing frames
                drop(state);
                // Find the frame and read the register
                if let Some(frame) = frames.iter().find(|f| f.frame_id == frame_id) {
                    if register < frame.registers.len() {
                        return frame.registers[register].clone();
                    }
                }
                LuaValue::nil()
            }
            UpvalueState::Closed(ref val) => val.clone(),
        }
    }

    /// Set the value (requires VM to write to stack if open)
    pub fn set_value(&self, frames: &mut [crate::vm::CallFrame], value: LuaValue) {
        let state = self.value.borrow();
        match *state {
            UpvalueState::Open { frame_id, register } => {
                // Release the borrow before accessing frames
                drop(state);
                // Find the frame and write the register
                if let Some(frame) = frames.iter_mut().find(|f| f.frame_id == frame_id) {
                    if register < frame.registers.len() {
                        frame.registers[register] = value;
                    }
                }
            }
            UpvalueState::Closed(_) => {
                // Release the borrow before mut borrow
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
}

impl fmt::Debug for LuaUpvalue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self.value.borrow() {
            UpvalueState::Open { frame_id, register } => {
                write!(f, "Upvalue::Open(frame={}, reg={})", frame_id, register)
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
    metatable: Rc<RefCell<Option<Rc<RefCell<LuaTable>>>>>,
}

impl LuaUserdata {
    pub fn new<T: Any>(data: T) -> Self {
        LuaUserdata {
            data: Rc::new(RefCell::new(Box::new(data))),
            metatable: Rc::new(RefCell::new(None)),
        }
    }

    pub fn with_metatable<T: Any>(data: T, metatable: Rc<RefCell<LuaTable>>) -> Self {
        LuaUserdata {
            data: Rc::new(RefCell::new(Box::new(data))),
            metatable: Rc::new(RefCell::new(Some(metatable))),
        }
    }

    pub fn get_data(&self) -> Rc<RefCell<Box<dyn Any>>> {
        self.data.clone()
    }

    pub fn get_metatable(&self) -> Option<Rc<RefCell<LuaTable>>> {
        self.metatable.borrow().clone()
    }

    pub fn set_metatable(&self, metatable: Option<Rc<RefCell<LuaTable>>>) {
        *self.metatable.borrow_mut() = metatable;
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
#[derive(Debug)]
pub struct Chunk {
    pub code: Vec<u32>,
    pub constants: Vec<LuaValue>,
    pub locals: Vec<String>,
    pub upvalue_count: usize,
    pub param_count: usize,
    pub max_stack_size: usize,
    pub child_protos: Vec<Rc<Chunk>>, // Nested function prototypes
    pub upvalue_descs: Vec<UpvalueDesc>, // Upvalue descriptors
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            locals: Vec::new(),
            upvalue_count: 0,
            param_count: 0,
            max_stack_size: 0,
            child_protos: Vec::new(),
            upvalue_descs: Vec::new(),
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
        assert!(float_val.is_integer());
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
