// Lua 5.4 compatible value representation
// 16 bytes, no pointer caching, all GC objects accessed via ID
pub mod chunk_serializer;
mod lua_table;
mod lua_value;

use std::any::Any;
use std::fmt;
use std::rc::Rc;

// Re-export the optimized LuaValue and type enum for pattern matching
pub use lua_table::LuaTable;
pub use lua_value::{LuaValue, LuaValueKind};

// Re-export type tag constants for VM execution
pub use lua_value::{
    LUA_TBOOLEAN, LUA_TNIL, LUA_TNUMBER, LUA_TSTRING, LUA_VFALSE, LUA_VNIL, LUA_VNUMFLT,
    LUA_VNUMINT, LUA_VTRUE,
};

use crate::lua_vm::CFunction;
use crate::{Instruction, StringInterner, TablePtr, UpvaluePtr};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LuaValuePtr {
    pub ptr: *mut LuaValue,
}

/// Lua function
/// Runtime upvalue - can be open (pointing to stack) or closed (owns value)
/// This matches Lua's UpVal implementation
pub enum LuaUpvalue {
    Open {
        stack_index: usize,     // Absolute index in register_stack
        stack_ptr: LuaValuePtr, // Cached pointer for fast access
    },
    Closed(LuaValue), // Value moved to heap after frame exits
}

impl LuaUpvalue {
    /// Create an open upvalue pointing to a stack location (absolute index)
    pub fn new_open(stack_index: usize, stack_ptr: LuaValuePtr) -> Self {
        LuaUpvalue::Open {
            stack_index,
            stack_ptr,
        }
    }

    /// Create a closed upvalue with an owned value
    pub fn new_closed(value: LuaValue) -> Self {
        LuaUpvalue::Closed(value)
    }

    /// Check if this upvalue is open
    pub fn is_open(&self) -> bool {
        matches!(self, LuaUpvalue::Open { .. })
    }

    /// Get the stack index if open (for comparison during close)
    pub fn get_stack_index(&self) -> Option<usize> {
        match self {
            LuaUpvalue::Open { stack_index, .. } => Some(*stack_index),
            _ => None,
        }
    }

    /// Close this upvalue (move value from stack to heap)
    pub fn close(&mut self, stack_value: LuaValue) {
        match self {
            LuaUpvalue::Open { .. } => {
                // Replace with closed variant
                *self = LuaUpvalue::Closed(stack_value);
            }
            LuaUpvalue::Closed(_) => {
                *self = LuaUpvalue::Closed(stack_value);
            }
        }
    }

    /// Get the value (requires register_stack if open)
    pub fn get_value(&self) -> LuaValue {
        match self {
            LuaUpvalue::Open { stack_ptr, .. } => unsafe { *stack_ptr.ptr },
            LuaUpvalue::Closed(val) => val.clone(),
        }
    }

    pub fn get_closed_value(&self) -> Option<&LuaValue> {
        match self {
            LuaUpvalue::Closed(val) => Some(val),
            _ => None,
        }
    }
}

/// Userdata - arbitrary Rust data with optional metatable
pub struct LuaUserdata {
    data: Box<dyn Any>,
    metatable: TablePtr,
}

impl LuaUserdata {
    pub fn new<T: Any>(data: T) -> Self {
        LuaUserdata {
            data: Box::new(data),
            metatable: TablePtr::null(),
        }
    }

    pub fn with_metatable<T: Any>(data: T, metatable: TablePtr) -> Self {
        LuaUserdata {
            data: Box::new(data),
            metatable,
        }
    }

    pub fn get_data(&self) -> &Box<dyn Any> {
        &self.data
    }

    pub fn get_data_mut(&mut self) -> &mut Box<dyn Any> {
        &mut self.data
    }

    pub fn get_metatable(&self) -> Option<LuaValue> {
        if self.metatable.is_null() {
            None
        } else {
            Some(LuaValue::table(self.metatable))
        }
    }

    pub(crate) fn set_metatable(&mut self, metatable: LuaValue) {
        if let Some(table_ptr) = metatable.as_table_ptr() {
            self.metatable = table_ptr;
        } else if metatable.is_nil() {
            self.metatable = TablePtr::null();
        } else {
            debug_assert!(
                false,
                "Attempted to set userdata metatable to non-table, non-nil value"
            );
        }
    }
}

impl fmt::Debug for LuaUserdata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Userdata({:p})", self.data.as_ref() as *const dyn Any)
    }
}

/// Upvalue descriptor
#[derive(Debug, Clone)]
pub struct UpvalueDesc {
    pub name: String,   // upvalue name
    pub is_local: bool, // true if captures parent local, false if captures parent upvalue
    pub index: u32,     // index in parent's register or upvalue array
}

/// Compiled chunk (bytecode + metadata)
#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<Instruction>,
    pub constants: Vec<LuaValue>,
    pub locals: Vec<String>,
    pub upvalue_count: usize,
    pub param_count: usize,
    pub is_vararg: bool,          // Whether function uses ... (varargs)
    pub needs_vararg_table: bool, // Whether function needs vararg table (PF_VATAB in Lua 5.5)
    pub use_hidden_vararg: bool,  // Whether function uses hidden vararg args (PF_VAHID in Lua 5.5)
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
            needs_vararg_table: false,
            use_hidden_vararg: false,
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

#[derive(Debug, Clone)]
pub struct LuaString {
    pub str: String,
    pub hash: u64,
}

impl LuaString {
    pub fn new(s: String, hash: u64) -> Self {
        Self { str: s, hash }
    }

    pub fn as_str(&self) -> &str {
        &self.str
    }

    pub fn is_short(&self) -> bool {
        self.str.len() <= StringInterner::SHORT_STRING_LIMIT
    }

    pub fn is_long(&self) -> bool {
        self.str.len() > StringInterner::SHORT_STRING_LIMIT
    }
}

impl Eq for LuaString {}

impl PartialEq for LuaString {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.str == other.str
    }
}

pub struct LuaFunction {
    chunk: Rc<Chunk>,
    upvalue_ptrs: Vec<UpvaluePtr>,
}

impl LuaFunction {
    pub fn new(chunk: Rc<Chunk>, upvalue_ptrs: Vec<UpvaluePtr>) -> Self {
        LuaFunction {
            chunk,
            upvalue_ptrs,
        }
    }

    /// Get the chunk if this is a Lua function
    #[inline(always)]
    pub fn chunk(&self) -> &Chunk {
        &self.chunk
    }

    /// Get cached upvalues (direct pointers for fast access)
    #[inline(always)]
    pub fn upvalues(&self) -> &Vec<UpvaluePtr> {
        &self.upvalue_ptrs
    }

    /// Get mutable access to cached upvalues for updating pointers
    #[inline(always)]
    pub fn upvalues_mut(&mut self) -> &mut Vec<UpvaluePtr> {
        &mut self.upvalue_ptrs
    }
}

pub struct CClosureFunction {
    func: CFunction,
    upvalues: Vec<LuaValue>,
}

impl CClosureFunction {
    pub fn new(func: CFunction, upvalues: Vec<LuaValue>) -> Self {
        CClosureFunction { func, upvalues }
    }

    /// Get the C function pointer
    #[inline(always)]
    pub fn func(&self) -> CFunction {
        self.func
    }

    /// Get upvalues
    #[inline(always)]
    pub fn upvalues(&self) -> &Vec<LuaValue> {
        &self.upvalues
    }

    /// Get mutable access to upvalues
    #[inline(always)]
    pub fn upvalues_mut(&mut self) -> &mut Vec<LuaValue> {
        &mut self.upvalues
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
}
