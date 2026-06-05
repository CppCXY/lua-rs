// Lua 5.5 compatible value representation
// 16 bytes, no pointer caching, all GC objects accessed via ID
pub mod alive_ref;
pub mod chunk_serializer;
pub mod lua_convert;
mod lua_string;
mod lua_table;
#[allow(clippy::module_inception)]
mod lua_value;
mod userdata;
pub mod userdata_builder;
pub mod userdata_trait;

use self::lua_value::Value;
use std::sync::Arc;

pub use lua_string::*;
pub use userdata::LuaUserdata;
pub use userdata_builder::UserDataBuilder;
pub use userdata_trait::{
    lua_value_to_udvalue, udvalue_to_lua_value, udvalue_to_lua_value_with_token,
};

// Re-export the optimized LuaValue and type enum for pattern matching
pub use lua_table::LuaRawTable;
pub use lua_value::{BIT_ISCOLLECTABLE, LUA_VFALSE, LUA_VNIL, LUA_VNUMFLT, LUA_VNUMINT, LUA_VTRUE};
pub use lua_value::{LuaValue, LuaValueKind};

pub(crate) type LuaInnerValue = Value; // For internal use within LuaValue implementation

use crate::Instruction;
use crate::gc::{ProtoPtr, UpvaluePtr};
use crate::lua_vm::{CFunction, StkId};

/// Runtime upvalue — pointer-based design matching C Lua's UpVal.
///
/// Like C Lua, `v` always points to the current value:
/// - **Open**: `v` points to the stack slot in `register_stack`
/// - **Closed**: `v` points to `self.closed_value`
///
/// This eliminates the match branch on every `get_value()`/`set_value()` call,
/// replacing it with a single pointer dereference (zero branching).
pub struct LuaUpvalue {
    /// Always-valid pointer to the upvalue's current value.
    /// Open → stack slot, Closed → &self.closed_value
    v: StkId,
    /// Storage for the closed value. When closed, `v` points here.
    closed_value: LuaValue,
    /// Stack index (only meaningful when open)
    stack_index: usize,
}

impl LuaUpvalue {
    /// Create an open upvalue pointing to a stack location (absolute index).
    /// `stack_ptr` must remain valid until the upvalue is closed or the pointer is updated.
    #[inline(always)]
    pub fn new_open(stack_index: usize, stk_id: StkId) -> Self {
        LuaUpvalue {
            v: stk_id,
            closed_value: LuaValue::nil(),
            stack_index,
        }
    }

    /// Create a closed upvalue with an owned value.
    /// **IMPORTANT**: `v` is initially null. You MUST call `fix_closed_ptr()` after
    /// the struct is placed at its final heap location (Box/Gc allocation).
    #[inline(always)]
    pub fn new_closed(value: LuaValue) -> Self {
        LuaUpvalue {
            v: StkId::null(),
            closed_value: value,
            stack_index: 0,
        }
    }

    /// Fix up the `v` pointer for a newly-created closed upvalue.
    /// Must be called once after the struct is heap-allocated (won't move again).
    /// No-op for open upvalues (where v is already a valid stack pointer).
    #[inline(always)]
    pub fn fix_closed_ptr(&mut self) {
        if !self.v.is_valid() {
            self.v = StkId::from_mut_ptr(&mut self.closed_value as *mut LuaValue);
        }
    }

    /// Check if this upvalue is open (like C Lua's `upisopen` macro).
    /// Open ⟺ `v` does NOT point to our own `closed_value` field.
    #[inline(always)]
    pub fn is_open(&self) -> bool {
        !std::ptr::eq(self.v.as_ptr(), &self.closed_value)
    }

    /// Get the stack index (only meaningful when open).
    #[inline(always)]
    pub fn get_stack_index(&self) -> usize {
        self.stack_index
    }

    /// Close this upvalue — copy value from stack into owned storage,
    /// then redirect `v` to point to `self.closed_value`.
    #[inline(always)]
    pub fn close(&mut self, stack_value: LuaValue) {
        self.closed_value = stack_value;
        self.v = StkId::from_mut_ptr(&mut self.closed_value as *mut LuaValue);
    }

    /// Update the cached stack pointer (called after stack reallocation).
    #[inline(always)]
    pub fn update_stack_ptr(&mut self, stk_id: StkId) {
        self.v = stk_id;
    }

    /// Get the raw v pointer (for caching in the execute loop).
    #[inline(always)]
    pub fn get_v_stk_id(&self) -> StkId {
        self.v
    }

    /// Get the value with **zero branching** — single pointer dereference.
    #[inline(always)]
    pub fn get_value(&self) -> LuaValue {
        self.v.get()
    }

    /// Get reference to the value with **zero branching**.
    #[inline(always)]
    pub fn get_value_ref(&self) -> &LuaValue {
        self.v.get_ref()
    }

    /// Set the value with **zero branching** — single pointer write.
    #[inline(always)]
    pub fn set_value(&mut self, val: LuaValue) {
        self.v.write(&val);
    }

    /// Set the value by raw parts to avoid constructing a temporary LuaValue.
    #[inline(always)]
    pub fn set_value_parts(&mut self, value: Value, tt: u8) {
        self.v.write_parts(tt, value);
    }

    pub fn get_closed_value(&self) -> Option<&LuaValue> {
        if !self.is_open() {
            Some(&self.closed_value)
        } else {
            None
        }
    }
}

/// Upvalue descriptor
#[derive(Debug, Clone)]
pub struct UpvalueDesc {
    pub name: String,   // upvalue name
    pub is_local: bool, // true if captures parent local, false if captures parent upvalue
    pub index: u32,     // index in parent's register or upvalue array
}

/// Local variable debug info (mirrors Lua 5.5's LocVar)
#[derive(Debug, Clone)]
pub struct LocVar {
    pub name: String, // variable name
    pub startpc: u32, // first point where variable is active
    pub endpc: u32,   // first point where variable is dead
}

/// Compiled chunk (bytecode + metadata)
#[derive(Debug, Clone)]
pub struct LuaProto {
    pub code: Vec<Instruction>,
    pub constants: Vec<LuaValue>,
    pub locals: Vec<LocVar>,
    pub upvalue_count: usize,
    pub param_count: usize,
    pub is_vararg: bool,          // Whether function uses ... (varargs)
    pub needs_vararg_table: bool, // Whether function needs vararg table (PF_VATAB in Lua 5.5)
    pub use_hidden_vararg: bool,  // Whether function uses hidden vararg args (PF_VAHID in Lua 5.5)
    pub max_stack_size: usize,
    pub child_protos: Vec<ProtoPtr>,     // Nested function prototypes
    pub upvalue_descs: Vec<UpvalueDesc>, // Upvalue descriptors
    pub source_name: Option<Arc<str>>,   // Source file/chunk name for debugging
    pub line_info: Vec<u32>,             // Line number for each instruction (for debug)
    pub linedefined: usize,              // Line where function starts (0 for main)
    pub lastlinedefined: usize,          // Line where function ends (0 for main)
    pub proto_data_size: u32,            // Cached size for GC (code+constants+children+lines)
}

impl Default for LuaProto {
    fn default() -> Self {
        Self::new()
    }
}

impl LuaProto {
    pub fn new() -> Self {
        LuaProto {
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
            proto_data_size: 0,
        }
    }

    /// Compute and cache proto_data_size. Call once after compilation is complete.
    pub fn compute_proto_data_size(&mut self) {
        use std::mem::size_of;
        let instr_size = self.code.len() * size_of::<crate::lua_vm::Instruction>();
        let const_size = self.constants.len() * size_of::<LuaValue>();
        let child_size = self.child_protos.len() * size_of::<ProtoPtr>();
        let line_size = self.line_info.len() * size_of::<u32>();
        self.proto_data_size = (instr_size + const_size + child_size + line_size) as u32;
    }

    #[cfg(feature = "shared-proto")]
    pub fn share_constant_strings(&mut self) -> usize {
        let mut shared_count = 0;

        for constant in &mut self.constants {
            shared_count += usize::from(crate::gc::share_lua_value(constant));
        }

        shared_count
    }

    #[cfg(feature = "shared-proto")]
    pub fn share_proto_strings(&mut self) -> usize {
        let mut shared_count = self.share_constant_strings();

        for child in &mut self.child_protos {
            shared_count += child.as_mut_ref().data.share_proto_strings();
        }

        shared_count
    }
}

/// Inline storage for upvalue pointers — avoids heap allocation for 0-1 upvalues.
/// Most closures in Lua have 1 upvalue (_ENV), so this eliminates one allocation
/// per closure creation on the most common path.
pub enum UpvalueStore {
    Empty,
    One(UpvaluePtr),
    Many(Box<[UpvaluePtr]>),
}

impl UpvalueStore {
    #[inline(always)]
    pub fn from_single(ptr: UpvaluePtr) -> Self {
        UpvalueStore::One(ptr)
    }

    #[inline(always)]
    pub fn from_vec(v: Vec<UpvaluePtr>) -> Self {
        match v.len() {
            0 => UpvalueStore::Empty,
            1 => UpvalueStore::One(v[0]),
            _ => UpvalueStore::Many(v.into_boxed_slice()),
        }
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[UpvaluePtr] {
        match self {
            UpvalueStore::Empty => &[],
            UpvalueStore::One(p) => std::slice::from_ref(p),
            UpvalueStore::Many(b) => b,
        }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [UpvaluePtr] {
        match self {
            UpvalueStore::Empty => &mut [],
            UpvalueStore::One(p) => std::slice::from_mut(p),
            UpvalueStore::Many(b) => b,
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        match self {
            UpvalueStore::Empty => 0,
            UpvalueStore::One(_) => 1,
            UpvalueStore::Many(b) => b.len(),
        }
    }
}

pub struct LuaRawFunction {
    chunk: ProtoPtr,
    upvalue_store: UpvalueStore,
}

impl LuaRawFunction {
    pub fn new(chunk: ProtoPtr, upvalue_store: UpvalueStore) -> Self {
        LuaRawFunction {
            chunk,
            upvalue_store,
        }
    }

    /// Get the chunk if this is a Lua function
    #[inline(always)]
    pub fn chunk(&self) -> &LuaProto {
        &self.chunk.as_ref().data
    }

    #[inline(always)]
    pub fn proto(&self) -> ProtoPtr {
        self.chunk
    }

    /// Get upvalue pointers as a slice.
    #[inline(always)]
    pub fn upvalues(&self) -> &[UpvaluePtr] {
        self.upvalue_store.as_slice()
    }

    /// Get mutable access to upvalue pointers (used by debug.upvaluejoin)
    #[inline(always)]
    pub fn upvalues_mut(&mut self) -> &mut [UpvaluePtr] {
        self.upvalue_store.as_mut_slice()
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

/// Rust closure callback — can capture arbitrary Rust state via `Box<dyn Fn>`
pub type RustCallback = Box<dyn Fn(&mut crate::lua_vm::LuaState) -> crate::LuaResult<usize>>;

/// RClosure: Rust closure function with optional LuaValue upvalues.
/// Unlike CClosureFunction (which stores a bare fn pointer), this stores
/// a heap-allocated trait object that can capture arbitrary Rust state.
pub struct RClosureFunction {
    func: RustCallback,
    upvalues: Vec<LuaValue>,
}

impl RClosureFunction {
    pub fn new(func: RustCallback, upvalues: Vec<LuaValue>) -> Self {
        RClosureFunction { func, upvalues }
    }

    /// Call the Rust closure
    #[inline(always)]
    pub fn call(&self, state: &mut crate::lua_vm::LuaState) -> crate::LuaResult<usize> {
        (self.func)(state)
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
        let float_val = LuaValue::number(3.15);

        // as_number works for both
        assert_eq!(int_val.as_number(), Some(42.0));
        assert_eq!(float_val.as_number(), Some(3.15));
    }
}
