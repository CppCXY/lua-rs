//! Lua execution context - combines LuaState and LuaVM access
//! 
//! This is the Rust-safe equivalent of Lua C's lua_State pointer.
//! C functions access both execution state and global resources through this context.

use crate::gc::{TableId, UpvalueId};
use crate::lua_value::LuaValue;
use crate::lua_vm::{CallInfo, LuaError, LuaResult, LuaState, LuaVM};
use std::rc::Rc;

/// Execution context for Lua functions
/// 
/// Combines mutable access to both:
/// - LuaState: Execution stack, call frames, upvalues
/// - LuaVM: Object pool, GC, global tables
/// 
/// This is the primary interface for Rust functions called from Lua.
/// Equivalent to `lua_State*` in Lua C API.
pub struct LuaContext<'a> {
    /// Execution state (stack, frames, upvalues)
    pub(crate) state: &'a mut LuaState,
    
    /// Global VM (object pool, GC, globals)
    pub(crate) vm: &'a mut LuaVM,
}

impl<'a> LuaContext<'a> {
    /// Create a new context
    #[inline]
    pub fn new(state: &'a mut LuaState, vm: &'a mut LuaVM) -> Self {
        Self { state, vm }
    }

    // ===== Object Creation (delegated to VM) =====

    /// Create a new table
    #[inline]
    pub fn create_table(&mut self, narr: usize, nrec: usize) -> LuaValue {
        self.vm.create_table(narr, nrec)
    }

    /// Create a new function closure
    #[inline]
    pub fn create_function(&mut self, chunk: Rc<crate::Chunk>, upvalues: Vec<UpvalueId>) -> LuaValue {
        self.vm.create_function(chunk, upvalues)
    }

    /// Create a string
    #[inline]
    pub fn create_string(&mut self, s: &str) -> LuaValue {
        self.vm.create_string(s)
    }

    // ===== Table Operations =====

    /// Get table value
    #[inline]
    pub fn table_get(&mut self, table: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        self.vm.table_get(table, key)
    }

    // Note: table_set 和 table_len 需要通过 object_pool 直接访问
    // 或者在 VM 中添加这些方法

    // ===== Global Variables =====

    /// Get global variable
    #[inline]
    pub fn get_global(&mut self, name: &str) -> Option<LuaValue> {
        self.vm.get_global(name)
    }

    /// Set global variable
    #[inline]
    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        self.vm.set_global(name, value);
    }

    // ===== Stack Operations (delegated to State) =====

    /// Get register relative to current frame
    #[inline]
    pub fn reg_get(&self, reg: u8) -> LuaValue {
        self.state.reg_get(reg)
    }

    /// Set register relative to current frame
    #[inline]
    pub fn reg_set(&mut self, reg: u8, value: LuaValue) {
        self.state.reg_set(reg, value);
    }

    /// Get stack value at absolute index
    #[inline]
    pub fn stack_get(&self, index: usize) -> LuaValue {
        self.state.stack_get(index)
    }

    /// Set stack value at absolute index
    #[inline]
    pub fn stack_set(&mut self, index: usize, value: LuaValue) {
        self.state.stack_set(index, value);
    }

    /// Push value to stack
    #[inline]
    pub fn push(&mut self, value: LuaValue) {
        self.state.stack_mut().push(value);
    }

    /// Pop value from stack
    #[inline]
    pub fn pop(&mut self) -> Option<LuaValue> {
        self.state.stack_mut().pop()
    }

    /// Get stack top index
    #[inline]
    pub fn stack_top(&self) -> usize {
        self.state.stack().len()
    }

    // ===== Call Frame Operations =====

    /// Get current call frame
    #[inline]
    pub fn current_frame(&self) -> Option<&CallInfo> {
        self.state.current_frame()
    }

    /// Get mutable current call frame
    #[inline]
    pub fn current_frame_mut(&mut self) -> Option<&mut CallInfo> {
        self.state.current_frame_mut()
    }

    /// Push new call frame
    #[inline]
    pub fn push_frame(&mut self, func: LuaValue, base: usize, nparams: usize) -> LuaResult<()> {
        self.state.push_frame(func, base, nparams)
    }

    /// Pop call frame
    #[inline]
    pub fn pop_frame(&mut self) -> Option<CallInfo> {
        self.state.pop_frame()
    }

    /// Get call stack depth
    #[inline]
    pub fn call_depth(&self) -> usize {
        self.state.call_depth()
    }

    // ===== Error Handling =====

    /// Create a runtime error
    #[inline]
    pub fn error(&mut self, message: impl Into<String>) -> LuaError {
        let msg = message.into();
        self.state.set_error(msg);
        LuaError::RuntimeError
    }

    /// Get error message
    #[inline]
    pub fn error_msg(&self) -> &str {
        self.state.error_msg()
    }

    // ===== Type Conversion Helpers =====

    /// Convert value to string
    #[inline]
    pub fn to_string(&mut self, value: &LuaValue) -> String {
        self.vm.value_to_string_raw(value)
    }

    /// Check if value is a specific type
    #[inline]
    pub fn check_type(&self, _value: &LuaValue, _expected_type: &str) -> Result<(), String> {
        // Type checking logic here
        Ok(())
    }
}

// ===== Direct field access for advanced use =====

impl<'a> LuaContext<'a> {
    /// Get immutable reference to execution state
    /// Use this for advanced operations that need direct state access
    #[inline]
    pub fn state(&self) -> &LuaState {
        self.state
    }

    /// Get mutable reference to execution state
    /// Use this for advanced operations that need direct state access
    #[inline]
    pub fn state_mut(&mut self) -> &mut LuaState {
        self.state
    }

    /// Get immutable reference to VM
    /// Use this for advanced operations that need direct VM access
    #[inline]
    pub fn vm(&self) -> &LuaVM {
        self.vm
    }

    /// Get mutable reference to VM
    /// Use this for advanced operations that need direct VM access
    #[inline]
    pub fn vm_mut(&mut self) -> &mut LuaVM {
        self.vm
    }
}
