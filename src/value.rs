// Lua 5.4 compatible value representation with GC support
// This implementation separates Integer and Float types as per Lua 5.4 spec

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::VM;

/// Multi-return values from Lua functions
/// The first value is the primary return value, additional values go in the Vec
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
/// Returns MultiValue for supporting multiple return values
pub type CFunction = fn(&mut VM) -> Result<MultiValue, String>;

/// Lua value types following Lua 5.4 specification
/// Size: 16 bytes (8-byte discriminant + 8-byte payload)
#[derive(Clone)]
pub enum LuaValue {
    /// Nil type
    Nil,
    /// Boolean type
    Boolean(bool),
    /// Integer type (Lua 5.4+) - 64-bit signed integer
    Integer(i64),
    /// Float type (Lua 5.4+) - 64-bit floating point
    Float(f64),
    /// String type - reference counted, potentially interned
    String(Rc<LuaString>),
    /// Table type - reference counted
    Table(Rc<RefCell<LuaTable>>),
    /// Function type - reference counted
    Function(Rc<LuaFunction>),
    /// C Function type - native Rust function
    CFunction(CFunction),
    /// Userdata type - arbitrary Rust data with optional metatable
    Userdata(Rc<LuaUserdata>),
}

impl LuaValue {
    // Constructors
    pub fn nil() -> Self {
        LuaValue::Nil
    }

    pub fn boolean(b: bool) -> Self {
        LuaValue::Boolean(b)
    }

    pub fn integer(i: i64) -> Self {
        LuaValue::Integer(i)
    }

    pub fn number(n: f64) -> Self {
        LuaValue::Float(n)
    }

    /// Internal use only - create string value from already-allocated LuaString
    /// For GC-managed strings, use VM::create_string() instead
    #[doc(hidden)]
    pub fn string(s: LuaString) -> Self {
        LuaValue::String(Rc::new(s))
    }

    /// Internal use only - create table value from already-allocated LuaTable
    /// For GC-managed tables, use VM::create_table() instead
    #[doc(hidden)]
    pub fn table(t: LuaTable) -> Self {
        LuaValue::Table(Rc::new(RefCell::new(t)))
    }

    pub fn function(f: LuaFunction) -> Self {
        LuaValue::Function(Rc::new(f))
    }

    pub fn cfunction(f: CFunction) -> Self {
        LuaValue::CFunction(f)
    }

    pub fn userdata<T: Any>(data: T) -> Self {
        LuaValue::Userdata(Rc::new(LuaUserdata::new(data)))
    }
    
    pub fn userdata_with_metatable<T: Any>(data: T, metatable: Rc<RefCell<LuaTable>>) -> Self {
        LuaValue::Userdata(Rc::new(LuaUserdata::with_metatable(data, metatable)))
    }

    // Type checks
    pub fn is_nil(&self) -> bool {
        matches!(self, LuaValue::Nil)
    }

    pub fn is_boolean(&self) -> bool {
        matches!(self, LuaValue::Boolean(_))
    }

    pub fn is_integer(&self) -> bool {
        self.as_integer().is_some()
    }

    pub fn is_float(&self) -> bool {
        matches!(self, LuaValue::Float(_))
    }

    pub fn is_number(&self) -> bool {
        matches!(self, LuaValue::Integer(_) | LuaValue::Float(_))
    }

    pub fn is_string(&self) -> bool {
        matches!(self, LuaValue::String(_))
    }

    pub fn is_table(&self) -> bool {
        matches!(self, LuaValue::Table(_))
    }

    pub fn is_function(&self) -> bool {
        matches!(self, LuaValue::Function(_))
    }

    pub fn is_cfunction(&self) -> bool {
        matches!(self, LuaValue::CFunction(_))
    }

    pub fn is_userdata(&self) -> bool {
        matches!(self, LuaValue::Userdata(_))
    }

    pub fn is_callable(&self) -> bool {
        matches!(self, LuaValue::Function(_) | LuaValue::CFunction(_))
    }
    
    /// Get metatable for tables and userdata
    pub fn get_metatable(&self) -> Option<Rc<RefCell<LuaTable>>> {
        match self {
            LuaValue::Userdata(ud) => ud.get_metatable(),
            LuaValue::Table(t) => t.borrow().get_metatable().clone(),
            _ => None,
        }
    }

    // Value extractors
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            LuaValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Get as integer, with automatic conversion from float if exact
    /// Follows Lua 5.4 conversion rules
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            LuaValue::Integer(i) => Some(*i),
            LuaValue::Float(f) => {
                // Float converts to integer if it represents an exact integer
                if f.fract() == 0.0 && f.is_finite() {
                    Some(*f as i64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Get as float, with automatic conversion from integer
    pub fn as_float(&self) -> Option<f64> {
        match self {
            LuaValue::Integer(i) => Some(*i as f64),
            LuaValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Get as number (alias for as_float for backwards compatibility)
    pub fn as_number(&self) -> Option<f64> {
        self.as_float()
    }

    pub fn as_string(&self) -> Option<Rc<LuaString>> {
        match self {
            LuaValue::String(s) => Some(Rc::clone(s)),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<Rc<RefCell<LuaTable>>> {
        match self {
            LuaValue::Table(t) => Some(Rc::clone(t)),
            _ => None,
        }
    }

    pub fn as_function(&self) -> Option<Rc<LuaFunction>> {
        match self {
            LuaValue::Function(f) => Some(Rc::clone(f)),
            _ => None,
        }
    }

    pub fn as_cfunction(&self) -> Option<CFunction> {
        match self {
            LuaValue::CFunction(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_userdata(&self) -> Option<Rc<LuaUserdata>> {
        match self {
            LuaValue::Userdata(u) => Some(Rc::clone(u)),
            _ => None,
        }
    }

    // Convert to Lua-style string representation for printing
    pub fn to_string_repr(&self) -> String {
        match self {
            LuaValue::Nil => "nil".to_string(),
            LuaValue::Boolean(b) => b.to_string(),
            LuaValue::Integer(i) => i.to_string(),
            LuaValue::Float(f) => f.to_string(),
            LuaValue::String(s) => s.as_str().to_string(),
            LuaValue::Table(t) => format!("table: {:p}", Rc::as_ptr(t)),
            LuaValue::Function(f) => format!("function: {:p}", Rc::as_ptr(f)),
            LuaValue::CFunction(_) => "function: [C]".to_string(),
            LuaValue::Userdata(u) => format!("userdata: {:p}", Rc::as_ptr(u)),
        }
    }

    // Lua truthiness: only nil and false are falsy
    pub fn is_truthy(&self) -> bool {
        !matches!(self, LuaValue::Nil | LuaValue::Boolean(false))
    }
}

impl fmt::Debug for LuaValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LuaValue::Nil => write!(f, "nil"),
            LuaValue::Boolean(b) => write!(f, "{}", b),
            LuaValue::Integer(i) => write!(f, "{}", i),
            LuaValue::Float(n) => write!(f, "{}", n),
            LuaValue::String(s) => write!(f, "\"{}\"", s.as_str()),
            LuaValue::Table(_) => write!(f, "table"),
            LuaValue::Function(_) => write!(f, "function"),
            LuaValue::CFunction(_) => write!(f, "cfunction"),
            LuaValue::Userdata(_) => write!(f, "userdata"),
        }
    }
}

impl PartialEq for LuaValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LuaValue::Nil, LuaValue::Nil) => true,
            (LuaValue::Boolean(a), LuaValue::Boolean(b)) => a == b,
            (LuaValue::Integer(a), LuaValue::Integer(b)) => a == b,
            (LuaValue::Float(a), LuaValue::Float(b)) => a == b,
            // Allow comparison between integer and float
            (LuaValue::Integer(a), LuaValue::Float(b)) => *a as f64 == *b,
            (LuaValue::Float(a), LuaValue::Integer(b)) => *a == *b as f64,
            (LuaValue::String(a), LuaValue::String(b)) => a.as_str() == b.as_str(),
            // Tables are compared by reference
            (LuaValue::Table(a), LuaValue::Table(b)) => Rc::ptr_eq(a, b),
            // Functions are compared by reference
            (LuaValue::Function(a), LuaValue::Function(b)) => Rc::ptr_eq(a, b),
            // CFunction comparison by pointer (not perfect but workable)
            (LuaValue::CFunction(a), LuaValue::CFunction(b)) => {
                std::ptr::eq(a as *const CFunction, b as *const CFunction)
            }
            // Userdata compared by reference
            (LuaValue::Userdata(a), LuaValue::Userdata(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for LuaValue {}

impl PartialOrd for LuaValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LuaValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        
        // Define type priority: numbers < strings < others
        fn type_priority(val: &LuaValue) -> u8 {
            match val {
                LuaValue::Integer(_) | LuaValue::Float(_) => 0,
                LuaValue::String(_) => 1,
                _ => 2,
            }
        }
        
        let self_priority = type_priority(self);
        let other_priority = type_priority(other);
        
        // First compare by type priority
        match self_priority.cmp(&other_priority) {
            Ordering::Equal => {
                // Same type priority, compare within type
                match (self, other) {
                    // Numbers: compare numerically
                    (LuaValue::Integer(a), LuaValue::Integer(b)) => a.cmp(b),
                    (LuaValue::Float(a), LuaValue::Float(b)) => {
                        a.partial_cmp(b).unwrap_or(Ordering::Equal)
                    }
                    (LuaValue::Integer(a), LuaValue::Float(b)) => {
                        (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
                    }
                    (LuaValue::Float(a), LuaValue::Integer(b)) => {
                        a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
                    }
                    
                    // Strings: lexicographic comparison
                    (LuaValue::String(a), LuaValue::String(b)) => {
                        a.as_str().cmp(b.as_str())
                    }
                    
                    // Other types: compare by pointer address
                    (LuaValue::Table(a), LuaValue::Table(b)) => {
                        let ptr_a = Rc::as_ptr(a) as usize;
                        let ptr_b = Rc::as_ptr(b) as usize;
                        ptr_a.cmp(&ptr_b)
                    }
                    (LuaValue::Function(a), LuaValue::Function(b)) => {
                        let ptr_a = Rc::as_ptr(a) as usize;
                        let ptr_b = Rc::as_ptr(b) as usize;
                        ptr_a.cmp(&ptr_b)
                    }
                    (LuaValue::CFunction(a), LuaValue::CFunction(b)) => {
                        let ptr_a = *a as usize;
                        let ptr_b = *b as usize;
                        ptr_a.cmp(&ptr_b)
                    }
                    (LuaValue::Userdata(a), LuaValue::Userdata(b)) => {
                        let ptr_a = Rc::as_ptr(a) as usize;
                        let ptr_b = Rc::as_ptr(b) as usize;
                        ptr_a.cmp(&ptr_b)
                    }
                    (LuaValue::Boolean(a), LuaValue::Boolean(b)) => {
                        a.cmp(b)
                    }
                    (LuaValue::Nil, LuaValue::Nil) => Ordering::Equal,
                    
                    // Mixed types within same priority (shouldn't happen based on type_priority)
                    _ => Ordering::Equal,
                }
            }
            other_ordering => other_ordering,
        }
    }
}

/// impl hash
impl std::hash::Hash for LuaValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            LuaValue::Nil => {
                0u8.hash(state);
            }
            LuaValue::Boolean(b) => {
                1u8.hash(state);
                b.hash(state);
            }
            LuaValue::Integer(i) => {
                2u8.hash(state);
                i.hash(state);
            }
            LuaValue::Float(f) => {
                3u8.hash(state);
                // Hash the bits of the float
                let bits = f.to_bits();
                bits.hash(state);
            }
            LuaValue::String(s) => {
                4u8.hash(state);
                s.as_str().hash(state);
            }
            LuaValue::Table(t) => {
                5u8.hash(state);
                Rc::as_ptr(t).hash(state);
            }
            LuaValue::Function(f) => {
                6u8.hash(state);
                Rc::as_ptr(f).hash(state);
            }
            LuaValue::CFunction(f) => {
                7u8.hash(state);
                let ptr = *f as *const CFunction;
                ptr.hash(state);
            }
            LuaValue::Userdata(u) => {
                8u8.hash(state);
                Rc::as_ptr(u).hash(state);
            }
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

    /// Get value with raw access (no metamethods)
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        // Try array part first for integer keys
        if let Some(i) = key.as_integer() {
            let idx = i as usize;
            if idx > 0 {
                if let Some(ref arr) = self.array {
                    if idx <= arr.len() {
                        return arr.get(idx - 1).cloned();
                    }
                }
            }
        }

        // Try hash part
        self.hash.as_ref().and_then(|h| h.get(key).cloned())
    }

    /// Set value with raw access (no metamethods)
    pub fn raw_set(&mut self, key: LuaValue, value: LuaValue) {
        // Try array part first for integer keys in range
        if let Some(i) = key.as_integer() {
            let idx = i as usize;
            if idx > 0 {
                // Get or create array
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
        }

        // Use hash part for all other keys
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
                LuaValue::Nil
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
