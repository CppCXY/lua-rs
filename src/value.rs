// Lua value representation using NaN-boxing for compact memory layout
// This allows storing all Lua types in a single 64-bit word

use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;

// NaN-boxing constants
// IEEE 754 double: sign(1) exponent(11) mantissa(52)
// NaN: exponent = 0x7FF, mantissa != 0
// We use the quiet NaN space: 0x7FF8_0000_0000_0000 - 0x7FFF_FFFF_FFFF_FFFF
const QNAN: u64 = 0x7FF8_0000_0000_0000;
const TAG_NIL: u64 = 0x0001;
const TAG_FALSE: u64 = 0x0002;
const TAG_TRUE: u64 = 0x0003;
const TAG_STRING: u64 = 0x0004;
const TAG_TABLE: u64 = 0x0005;
const TAG_FUNCTION: u64 = 0x0006;
#[allow(dead_code)]
const TAG_USERDATA: u64 = 0x0007;

// Pointer mask for extracting pointer from tagged value
const POINTER_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Compact Lua value (8 bytes)
/// Uses NaN-boxing to store all types in a single u64
pub struct LuaValue(u64);

// Manually implement Clone to properly handle Rc reference counts
impl Clone for LuaValue {
    fn clone(&self) -> Self {
        if self.is_string() {
            let ptr = (self.0 & POINTER_MASK) as *const LuaString;
            unsafe {
                Rc::increment_strong_count(ptr);
            }
        } else if self.is_table() {
            let ptr = (self.0 & POINTER_MASK) as *const RefCell<LuaTable>;
            unsafe {
                Rc::increment_strong_count(ptr);
            }
        } else if self.is_function() {
            let ptr = (self.0 & POINTER_MASK) as *const LuaFunction;
            unsafe {
                Rc::increment_strong_count(ptr);
            }
        }
        LuaValue(self.0)
    }
}

impl LuaValue {
    // Constructors
    pub fn nil() -> Self {
        LuaValue(QNAN | TAG_NIL)
    }

    pub fn boolean(b: bool) -> Self {
        if b {
            LuaValue(QNAN | TAG_TRUE)
        } else {
            LuaValue(QNAN | TAG_FALSE)
        }
    }

    pub fn number(n: f64) -> Self {
        LuaValue(n.to_bits())
    }

    pub fn string(s: LuaString) -> Self {
        let ptr = Rc::into_raw(Rc::new(s)) as u64;
        LuaValue(QNAN | TAG_STRING | (ptr & POINTER_MASK))
    }

    pub fn table(t: LuaTable) -> Self {
        let ptr = Rc::into_raw(Rc::new(RefCell::new(t))) as u64;
        LuaValue(QNAN | TAG_TABLE | (ptr & POINTER_MASK))
    }

    pub fn function(f: LuaFunction) -> Self {
        let ptr = Rc::into_raw(Rc::new(f)) as u64;
        LuaValue(QNAN | TAG_FUNCTION | (ptr & POINTER_MASK))
    }

    // Type checks
    pub fn is_nil(&self) -> bool {
        self.0 == (QNAN | TAG_NIL)
    }

    pub fn is_boolean(&self) -> bool {
        (self.0 & (QNAN | 0xFFFE)) == (QNAN | TAG_FALSE)
    }

    pub fn is_number(&self) -> bool {
        (self.0 & QNAN) != QNAN
    }

    pub fn is_string(&self) -> bool {
        (self.0 & (QNAN | 0x000F)) == (QNAN | TAG_STRING)
    }

    pub fn is_table(&self) -> bool {
        (self.0 & (QNAN | 0x000F)) == (QNAN | TAG_TABLE)
    }

    pub fn is_function(&self) -> bool {
        (self.0 & (QNAN | 0x000F)) == (QNAN | TAG_FUNCTION)
    }

    // Value extractors
    pub fn as_boolean(&self) -> Option<bool> {
        if self.0 == (QNAN | TAG_TRUE) {
            Some(true)
        } else if self.0 == (QNAN | TAG_FALSE) {
            Some(false)
        } else {
            None
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        if self.is_number() {
            Some(f64::from_bits(self.0))
        } else {
            None
        }
    }

    pub fn as_string(&self) -> Option<Rc<LuaString>> {
        if self.is_string() {
            let ptr = (self.0 & POINTER_MASK) as *const LuaString;
            unsafe {
                // Clone the Rc without consuming it
                let rc = std::mem::ManuallyDrop::new(Rc::from_raw(ptr));
                Some(Rc::clone(&rc))
            }
        } else {
            None
        }
    }

    pub fn as_table(&self) -> Option<Rc<RefCell<LuaTable>>> {
        if self.is_table() {
            let ptr = (self.0 & POINTER_MASK) as *const RefCell<LuaTable>;
            unsafe {
                let rc = std::mem::ManuallyDrop::new(Rc::from_raw(ptr));
                Some(Rc::clone(&rc))
            }
        } else {
            None
        }
    }

    pub fn as_function(&self) -> Option<Rc<LuaFunction>> {
        if self.is_function() {
            let ptr = (self.0 & POINTER_MASK) as *const LuaFunction;
            unsafe {
                let rc = std::mem::ManuallyDrop::new(Rc::from_raw(ptr));
                Some(Rc::clone(&rc))
            }
        } else {
            None
        }
    }

    // Lua truthiness: only nil and false are falsy
    pub fn is_truthy(&self) -> bool {
        !self.is_nil() && !(self.is_boolean() && !self.as_boolean().unwrap())
    }
}

impl Drop for LuaValue {
    fn drop(&mut self) {
        // Decrement reference count for heap-allocated types
        if self.is_string() {
            let ptr = (self.0 & POINTER_MASK) as *const LuaString;
            unsafe {
                // Properly drop the Rc
                let _ = Rc::from_raw(ptr);
            }
        } else if self.is_table() {
            let ptr = (self.0 & POINTER_MASK) as *const RefCell<LuaTable>;
            unsafe {
                let _ = Rc::from_raw(ptr);
            }
        } else if self.is_function() {
            let ptr = (self.0 & POINTER_MASK) as *const LuaFunction;
            unsafe {
                let _ = Rc::from_raw(ptr);
            }
        }
    }
}

impl std::fmt::Debug for LuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_nil() {
            write!(f, "nil")
        } else if let Some(b) = self.as_boolean() {
            write!(f, "{}", b)
        } else if let Some(n) = self.as_number() {
            write!(f, "{}", n)
        } else if let Some(s) = self.as_string() {
            write!(f, "\"{}\"", s.as_str())
        } else if self.is_table() {
            write!(f, "table")
        } else if self.is_function() {
            write!(f, "function")
        } else {
            write!(f, "unknown")
        }
    }
}

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
#[derive(Debug)]
pub struct LuaTable {
    array: Vec<LuaValue>,
    hash: HashMap<TableKey, LuaValue>,
}

impl LuaTable {
    pub fn new() -> Self {
        LuaTable {
            array: Vec::new(),
            hash: HashMap::new(),
        }
    }

    pub fn get(&self, key: &LuaValue) -> Option<LuaValue> {
        if let Some(n) = key.as_number() {
            let idx = n as usize;
            if idx > 0 && idx <= self.array.len() {
                return Some(self.array[idx - 1].clone());
            }
        }
        
        TableKey::from_value(key)
            .and_then(|k| self.hash.get(&k))
            .cloned()
    }

    pub fn set(&mut self, key: LuaValue, value: LuaValue) {
        if let Some(n) = key.as_number() {
            let idx = n as usize;
            if idx > 0 && idx <= self.array.len() + 1 {
                if idx == self.array.len() + 1 {
                    self.array.push(value);
                } else {
                    self.array[idx - 1] = value;
                }
                return;
            }
        }
        
        if let Some(k) = TableKey::from_value(&key) {
            self.hash.insert(k, value);
        }
    }

    pub fn len(&self) -> usize {
        self.array.len()
    }
}

/// Table key (can be string or number)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TableKey {
    String(String),
    Integer(i64),
}

impl TableKey {
    fn from_value(value: &LuaValue) -> Option<Self> {
        if let Some(s) = value.as_string() {
            Some(TableKey::String(s.as_str().to_string()))
        } else if let Some(n) = value.as_number() {
            if n.fract() == 0.0 {
                Some(TableKey::Integer(n as i64))
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// Lua function
#[derive(Debug, Clone)]
pub struct LuaFunction {
    pub chunk: Rc<Chunk>,
    pub upvalues: Vec<LuaValue>,
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
        }
    }
}
