// Lua execution state (equivalent to lua_State in Lua C API)
// Represents a single thread/coroutine execution context
// Multiple LuaStates can share the same LuaVM (global_State)

use std::rc::Rc;

use crate::gc::UpvalueId;
use crate::lua_value::LuaValue;
use crate::lua_vm::{CallInfo, LuaError, LuaResult};
use crate::{Chunk, LuaVM};

/// Execution state for a Lua thread/coroutine
/// This is separate from LuaVM (global_State) to support multiple execution contexts
pub struct LuaState {
    vm: *mut LuaVM,
    /// Data stack - stores all values (registers, temporaries, function arguments)
    /// Layout: [frame0_values...][frame1_values...][frame2_values...]
    /// Similar to Lua's TValue stack[] in lua_State
    stack: Vec<LuaValue>,

    /// Call stack - one CallInfo per active function call
    /// Grows dynamically on demand (like Lua 5.4's linked list approach)
    /// Similar to Lua's CallInfo *ci in lua_State
    call_stack: Vec<CallInfo>,

    /// Open upvalues list - upvalues pointing to stack locations
    /// Must be kept sorted by stack index for correct closure semantics
    /// Similar to Lua's UpVal *openupval in lua_State
    open_upvalues: Vec<UpvalueId>,

    /// Error message storage (lightweight error handling)
    error_msg: String,

    /// Yield values storage (for coroutine yield)
    yield_values: Vec<LuaValue>,

    /// Hook mask and count (for debug hooks)
    hook_mask: u8,
    hook_count: i32,
}

impl LuaState {
    /// Initial stack size (similar to BASIC_STACK_SIZE in Lua)
    const INITIAL_STACK_SIZE: usize = 256;

    /// Maximum call depth (similar to LUAI_MAXCCALLS)
    pub const MAX_CALL_DEPTH: usize = 200;

    /// Create a new execution state
    /// 按需分配，而不是预分配 200 个 CallInfo（像 Lua 5.4）
    pub fn new(call_stack_size: usize, vm: *mut LuaVM) -> Self {
        Self {
            vm,
            stack: Vec::with_capacity(Self::INITIAL_STACK_SIZE),
            // 初始只分配很小的容量，按需增长（Lua 5.4 初始只有 1 个）
            call_stack: Vec::with_capacity(call_stack_size),
            open_upvalues: Vec::new(),
            error_msg: String::new(),
            yield_values: Vec::new(),
            hook_mask: 0,
            hook_count: 0,
        }
    }

    pub(crate) fn set_vm(&mut self, vm: *mut LuaVM) {
        self.vm = vm;
    }

    /// Get current call frame (equivalent to Lua's L->ci)
    #[inline(always)]
    pub fn current_frame(&self) -> Option<&CallInfo> {
        self.call_stack.last()
    }

    /// Get mutable current call frame
    #[inline(always)]
    pub fn current_frame_mut(&mut self) -> Option<&mut CallInfo> {
        self.call_stack.last_mut()
    }

    /// Get call stack depth
    #[inline(always)]
    pub fn call_depth(&self) -> usize {
        self.call_stack.len()
    }

    /// Push a new call frame (equivalent to Lua's luaD_precall)
    /// 按需动态分配 - Lua 5.4 风格
    pub fn push_frame(&mut self, func: LuaValue, base: usize, nparams: usize) -> LuaResult<()> {
        // 检查栈深度限制
        if self.call_stack.len() >= Self::MAX_CALL_DEPTH {
            return Err(LuaError::StackOverflow);
        }

        // 动态分配新的 CallInfo（Lua 5.4 也是这样做的）
        let frame = CallInfo {
            func,
            base,
            top: base + nparams,
            pc: 0,
            nresults: -1, // Variable results by default
            call_status: 0,
            nextraargs: 0,
        };

        self.call_stack.push(frame);
        Ok(())
    }

    /// Pop call frame (equivalent to Lua's luaD_poscall)
    pub fn pop_frame(&mut self) -> Option<CallInfo> {
        self.call_stack.pop()
    }

    /// Get stack value at absolute index
    #[inline(always)]
    pub fn stack_get(&self, index: usize) -> Option<LuaValue> {
        self.stack.get(index).copied()
    }

    /// Set stack value at absolute index
    #[inline(always)]
    pub fn stack_set(&mut self, index: usize, value: LuaValue) {
        if index >= self.stack.len() {
            self.stack.resize(index + 1, LuaValue::nil());
        }
        self.stack[index] = value;
    }

    /// Get register relative to current frame base
    #[inline(always)]
    pub fn reg_get(&self, reg: u8) -> Option<LuaValue> {
        if let Some(frame) = self.current_frame() {
            self.stack_get(frame.base + reg as usize)
        } else {
            None
        }
    }

    /// Set register relative to current frame base
    #[inline(always)]
    pub fn reg_set(&mut self, reg: u8, value: LuaValue) {
        if let Some(frame) = self.current_frame() {
            let index = frame.base + reg as usize;
            self.stack_set(index, value);
        }
    }

    /// Get mutable reference to stack (for bulk operations)
    #[inline(always)]
    pub fn stack_mut(&mut self) -> &mut Vec<LuaValue> {
        &mut self.stack
    }

    /// Get open upvalues list
    #[inline(always)]
    pub fn open_upvalues(&self) -> &[UpvalueId] {
        &self.open_upvalues
    }

    /// Get mutable open upvalues list
    #[inline(always)]
    pub fn open_upvalues_mut(&mut self) -> &mut Vec<UpvalueId> {
        &mut self.open_upvalues
    }

    /// Set error message
    #[inline(always)]
    pub fn error(&mut self, msg: String) -> LuaError {
        self.error_msg = msg;
        LuaError::RuntimeError
    }

    /// Get error message
    #[inline(always)]
    pub fn error_msg(&self) -> &str {
        &self.error_msg
    }

    /// Clear error state
    #[inline(always)]
    pub fn clear_error(&mut self) {
        self.error_msg.clear();
        self.yield_values.clear();
    }

    /// Set yield values
    #[inline(always)]
    pub fn set_yield(&mut self, values: Vec<LuaValue>) {
        self.yield_values = values;
    }

    /// Take yield values
    #[inline(always)]
    pub fn take_yield(&mut self) -> Vec<LuaValue> {
        std::mem::take(&mut self.yield_values)
    }

    /// Close upvalues from a given stack index upwards
    /// This is called when exiting a function or block scope
    pub fn close_upvalues(&mut self, level: usize, object_pool: &mut crate::ObjectPool) {
        // Find all open upvalues pointing to indices >= level
        let mut i = 0;
        while i < self.open_upvalues.len() {
            let upval_id = self.open_upvalues[i];
            if let Some(upval) = object_pool.get_upvalue_mut(upval_id) {
                if let Some(stack_idx) = upval.get_stack_index() {
                    if stack_idx >= level {
                        // Close this upvalue - copy stack value to closed storage
                        if let Some(value) = self.stack_get(stack_idx) {
                            upval.close(value);
                        }
                        self.open_upvalues.remove(i);
                        continue;
                    }
                }
            }
            i += 1;
        }
    }

    /// Get stack reference (for GC tracing)
    pub fn stack(&self) -> &[LuaValue] {
        &self.stack
    }

    /// Get mutable pointer to stack for VM execution
    /// 
    /// # Safety
    /// Caller must ensure stack is not reallocated during pointer usage
    #[inline(always)]
    pub fn stack_ptr_mut(&mut self) -> *mut LuaValue {
        self.stack.as_mut_ptr()
    }

    /// Get stack length
    #[inline(always)]
    pub fn stack_len(&self) -> usize {
        self.stack.len()
    }

    /// Grow stack to accommodate more values
    /// Must be called BEFORE execution if frame might exceed current capacity
    pub fn grow_stack(&mut self, needed: usize) {
        if self.stack.len() < needed {
            self.stack.resize(needed, LuaValue::nil());
        }
    }

    /// Get frame base by index
    #[inline(always)]
    pub fn get_frame_base(&self, frame_idx: usize) -> usize {
        self.call_stack.get(frame_idx).map(|f| f.base).unwrap_or(0)
    }

    /// Get frame PC by index
    #[inline(always)]
    pub fn get_frame_pc(&self, frame_idx: usize) -> u32 {
        self.call_stack.get(frame_idx).map(|f| f.pc).unwrap_or(0)
    }

    /// Get frame function by index
    #[inline(always)]
    pub fn get_frame_func(&self, frame_idx: usize) -> Option<LuaValue> {
        self.call_stack.get(frame_idx).map(|f| f.func)
    }

    /// Set frame PC by index
    #[inline(always)]
    pub fn set_frame_pc(&mut self, frame_idx: usize, pc: u32) {
        if let Some(frame) = self.call_stack.get_mut(frame_idx) {
            frame.pc = pc;
        }
    }
    
    /// Set frame function by index (for tail calls)
    #[inline(always)]
    pub fn set_frame_func(&mut self, frame_idx: usize, func: LuaValue) {
        if let Some(frame) = self.call_stack.get_mut(frame_idx) {
            frame.func = func;
        }
    }

    pub(crate) fn vm_mut(&mut self) -> &mut LuaVM {
        unsafe { &mut *self.vm }
    }

    // ===== Call Frame Management =====

    /// Get current CallInfo by index
    #[inline(always)]
    pub fn get_call_info(&self, idx: usize) -> &CallInfo {
        &self.call_stack[idx]
    }

    /// Get mutable CallInfo by index
    #[inline(always)]
    pub fn get_call_info_mut(&mut self, idx: usize) -> &mut CallInfo {
        &mut self.call_stack[idx]
    }

    /// Set stack top to new position
    #[inline(always)]
    pub fn set_top(&mut self, new_top: usize) {
        if new_top > self.stack.len() {
            self.stack.resize(new_top, LuaValue::nil());
        }
        // Note: We don't shrink the stack here for performance
    }

    /// Pop the current call frame
    #[inline]
    pub fn pop_call_frame(&mut self) {
        if !self.call_stack.is_empty() {
            self.call_stack.pop();
        }
    }

    /// Get return values from stack
    /// Returns values from stack_base to stack_base + count
    pub fn get_return_values(&self, stack_base: usize, count: usize) -> Vec<LuaValue> {
        let mut results = Vec::with_capacity(count);
        for i in 0..count {
            if let Some(val) = self.stack_get(stack_base + i) {
                results.push(val);
            } else {
                results.push(LuaValue::nil());
            }
        }
        results
    }

    /// Get all return values from stack starting at stack_base
    pub fn get_all_return_values(&self, stack_base: usize) -> Vec<LuaValue> {
        let count = if self.stack.len() > stack_base {
            self.stack.len() - stack_base
        } else {
            0
        };
        self.get_return_values(stack_base, count)
    }

    // ===== Object Creation =====

    /// Create table
    pub fn create_table(&mut self, narr: usize, nrec: usize) -> LuaValue {
        self.vm_mut().create_table(narr, nrec)
    }

    /// Create function closure
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalues: Vec<UpvalueId>) -> LuaValue {
        self.vm_mut().create_function(chunk, upvalues)
    }

    /// Create/intern string (automatically handles short string interning)
    pub fn create_string(&mut self, s: &str) -> LuaValue {
        self.vm_mut().create_string(s)
    }

    // ===== Global Access =====

    /// Get global variable
    pub fn get_global(&mut self, name: &str) -> Option<LuaValue> {
        self.vm_mut().get_global(name)
    }

    /// Set global variable
    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        self.vm_mut().set_global(name, value);
    }

    // ===== Table Operations =====

    /// Get value from table
    pub fn table_get(&mut self, table: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        self.vm_mut().table_get(table, key)
    }

    /// Set value in table
    pub fn table_set(&mut self, table: &LuaValue, key: LuaValue, value: LuaValue) -> bool {
        self.vm_mut().table_set(table, key, value)
    }
}

impl Default for LuaState {
    fn default() -> Self {
        Self::new(1, std::ptr::null_mut())
    }
}
