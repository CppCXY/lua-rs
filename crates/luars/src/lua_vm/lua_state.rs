// Lua execution state (equivalent to lua_State in Lua C API)
// Represents a single thread/coroutine execution context
// Multiple LuaStates can share the same LuaVM (global_State)

use std::rc::Rc;

use crate::gc::UpvalueId;
use crate::lua_value::{LuaString, LuaUserdata, LuaValue};
use crate::lua_vm::safe_option::SafeOption;
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

    safe_option: SafeOption,
}

impl LuaState {
    /// Basic stack size (similar to BASIC_STACK_SIZE in Lua = 2*LUA_MINSTACK = 40)
    const BASIC_STACK_SIZE: usize = 40;

    /// Create a new execution state
    /// 按需分配，而不是预分配 200 个 CallInfo（像 Lua 5.4）
    pub fn new(call_stack_size: usize, vm: *mut LuaVM, safe_option: SafeOption) -> Self {
        // Start with BASIC_STACK_SIZE, will grow dynamically up to MAX_STACK_SIZE
        let stack = Vec::with_capacity(Self::BASIC_STACK_SIZE);
        Self {
            vm,
            stack,
            // 初始只分配很小的容量，按需增长（Lua 5.4 初始只有 1 个）
            call_stack: Vec::with_capacity(call_stack_size),
            open_upvalues: Vec::new(),
            error_msg: String::new(),
            yield_values: Vec::new(),
            hook_mask: 0,
            hook_count: 0,
            safe_option,
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
    pub fn push_frame(&mut self, func: LuaValue, base: usize, nparams: usize, nresults: i32) -> LuaResult<()> {
        // 检查栈深度限制
        if self.call_stack.len() >= self.safe_option.max_call_depth {
            self.error(format!(
                "call stack overflow: exceeded maximum depth of {}",
                self.safe_option.max_call_depth
            ));
            return Err(LuaError::StackOverflow);
        }

        // Determine call status based on function type
        use crate::lua_vm::call_info::call_status::{CIST_C, CIST_LUA};
        let call_status = if func.is_cfunction()
            || func
                .as_function_id()
                .and_then(|id| {
                    let vm = unsafe { &*self.vm };
                    vm.object_pool.get_function(id).and_then(|f| f.c_function())
                })
                .is_some()
        {
            CIST_C
        } else {
            CIST_LUA
        };

        // 动态分配新的 CallInfo（Lua 5.4 也是这样做的）
        let frame = CallInfo {
            func,
            base,
            top: base + nparams,
            pc: 0,
            nresults, // Use the nresults from caller
            call_status,
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
    pub fn stack_set(&mut self, index: usize, value: LuaValue) -> LuaResult<()> {
        if index >= self.safe_option.max_stack_size {
            self.error(format!(
                "stack overflow: attempted to set index {} exceeding maximum {}",
                index, self.safe_option.max_stack_size
            ));
            return Err(LuaError::StackOverflow);
        }
        if index >= self.stack.len() {
            self.stack.resize(index + 1, LuaValue::nil());
        }
        self.stack[index] = value;
        Ok(())
    }

    /// Insert a value at a specific stack position, shifting everything after it
    pub fn stack_insert(&mut self, index: usize, value: LuaValue) -> LuaResult<()> {
        if self.stack.len() + 1 >= self.safe_option.max_stack_size {
            self.error(format!(
                "stack overflow: attempted to insert at index {} exceeding maximum {}",
                index, self.safe_option.max_stack_size
            ));
            return Err(LuaError::StackOverflow);
        }
        if index >= self.stack.len() {
            self.stack.resize(index, LuaValue::nil());
            self.stack.push(value);
        } else {
            self.stack.insert(index, value);
        }
        Ok(())
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
    pub fn reg_set(&mut self, reg: u8, value: LuaValue) -> LuaResult<()> {
        if let Some(frame) = self.current_frame() {
            let index = frame.base + reg as usize;
            self.stack_set(index, value)?;
        }

        Ok(())
    }

    /// Get mutable reference to stack (for bulk operations)
    #[inline(always)]
    pub(crate) fn stack_mut(&mut self) -> &mut Vec<LuaValue> {
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

    /// Set error message (without traceback - will be added later by top-level handler)
    #[inline(always)]
    pub fn error(&mut self, msg: String) -> LuaError {
        // Try to get current source location for the error
        let location = if let Some(ci) = self.call_stack.last() {
            if ci.is_lua() {
                if let Some(func_id) = ci.func.as_function_id() {
                    let vm = unsafe { &*self.vm };
                    if let Some(func_obj) = vm.object_pool.get_function(func_id) {
                        if let Some(chunk) = func_obj.chunk() {
                            let source = chunk.source_name.as_deref().unwrap_or("[string]");
                            let line = if ci.pc > 0 && (ci.pc as usize - 1) < chunk.line_info.len()
                            {
                                chunk.line_info[ci.pc as usize - 1] as usize
                            } else if !chunk.line_info.is_empty() {
                                chunk.line_info[0] as usize
                            } else {
                                0
                            };
                            if line > 0 {
                                format!("{}:{}: ", source, line)
                            } else {
                                format!("{}: ", source)
                            }
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        self.error_msg = format!("{}{}", location, msg);
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

    /// Generate a Lua-style stack traceback
    /// Similar to luaL_traceback in lauxlib.c
    pub fn generate_traceback(&self) -> String {
        let mut result = String::new();
        let vm = unsafe { &*self.vm };

        // Iterate through call stack from newest to oldest
        for (level, ci) in self.call_stack.iter().rev().enumerate() {
            if level >= 20 {
                result.push_str("\t...\n");
                break;
            }

            // Get function info
            if ci.is_lua() {
                // Lua function - get source and line info
                if let Some(func_id) = ci.func.as_function_id() {
                    if let Some(func_obj) = vm.object_pool.get_function(func_id) {
                        if let Some(chunk) = func_obj.chunk() {
                            let source = chunk.source_name.as_deref().unwrap_or("[string]");

                            // Get current line number from PC
                            let line = if ci.pc > 0 && (ci.pc as usize - 1) < chunk.line_info.len()
                            {
                                chunk.line_info[ci.pc as usize - 1] as usize
                            } else if !chunk.line_info.is_empty() {
                                chunk.line_info[0] as usize
                            } else {
                                0
                            };

                            if level == 0 {
                                // Current function (where error occurred)
                                if line > 0 {
                                    result.push_str(&format!(
                                        "\t{}:{}: in main chunk\n",
                                        source, line
                                    ));
                                } else {
                                    result.push_str(&format!("\t{}: in main chunk\n", source));
                                }
                            } else {
                                // Called functions
                                if line > 0 {
                                    result
                                        .push_str(&format!("\t{}:{}: in function\n", source, line));
                                } else {
                                    result.push_str(&format!("\t{}: in function\n", source));
                                }
                            }
                            continue;
                        }
                    }
                }
                result.push_str("\t[?]: in function\n");
            } else if ci.is_c() {
                // C function
                result.push_str("\t[C]: in function\n");
            }
        }

        result
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
    pub(crate) fn stack_ptr_mut(&mut self) -> *mut LuaValue {
        self.stack.as_mut_ptr()
    }

    /// Get stack length
    #[inline(always)]
    pub fn stack_len(&self) -> usize {
        self.stack.len()
    }

    /// Truncate stack to specified length
    /// Used after function calls to remove temporary values
    pub fn stack_truncate(&mut self, new_len: usize) {
        if new_len < self.stack.len() {
            self.stack.truncate(new_len);
        }
    }

    /// Grow stack to accommodate more values
    /// Grow stack to accommodate needed size (similar to luaD_growstack in Lua)
    /// Stack can grow dynamically up to MAX_STACK_SIZE
    /// C functions can call this, which means Vec may reallocate
    pub fn grow_stack(&mut self, needed: usize) -> LuaResult<()> {
        if needed > self.safe_option.max_stack_size {
            self.error(format!(
                "stack overflow: attempted to grow stack to {} exceeding maximum {}",
                needed, self.safe_option.max_stack_size
            ));
            return Err(LuaError::StackOverflow);
        }
        if self.stack.len() < needed {
            self.stack.resize(needed, LuaValue::nil());
        }

        Ok(())
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

    // ===== Function Argument Access =====

    /// Get all arguments for the current C function call
    /// Returns arguments starting from index 1 (index 0 is the function itself)
    pub fn get_args(&self) -> Vec<LuaValue> {
        if self.call_stack.is_empty() {
            return Vec::new();
        }

        let frame = &self.call_stack[self.call_stack.len() - 1];
        let base = frame.base;
        let top = frame.top;

        // Arguments are from base to top-1 (NOT base+1!)
        // In Lua, the function itself is NOT part of the frame's stack
        // The frame starts at the first argument
        let arg_count = if top > base { top - base } else { 0 };

        let mut args = Vec::with_capacity(arg_count);
        for i in 0..arg_count {
            if let Some(val) = self.stack_get(base + i) {
                args.push(val);
            } else {
                args.push(LuaValue::nil());
            }
        }
        args
    }

    /// Get a specific argument (1-based index, Lua convention)
    /// Returns None if index is out of bounds
    pub fn get_arg(&self, index: usize) -> Option<LuaValue> {
        if index == 0 || self.call_stack.is_empty() {
            return None;
        }

        let frame = &self.call_stack[self.call_stack.len() - 1];
        let base = frame.base;
        let top = frame.top;

        // Arguments are 1-based: arg 1 is at base, arg 2 is at base+1, etc.
        // (NOT base+1, because C function frame.base already points to first arg)
        let stack_index = base + index - 1;

        // Check if argument position is within the valid range
        if stack_index < top && stack_index < self.stack.len() {
            // Return the value (including nil values)
            Some(self.stack[stack_index])
        } else {
            // Argument doesn't exist
            None
        }
    }

    /// Get the number of arguments for the current function call
    pub fn arg_count(&self) -> usize {
        if self.call_stack.is_empty() {
            return 0;
        }

        let frame = &self.call_stack[self.call_stack.len() - 1];
        let base = frame.base;
        let top = frame.top;

        // Arguments are from base to top-1
        if top > base { top - base } else { 0 }
    }

    pub fn push_value(&mut self, value: LuaValue) -> LuaResult<()> {
        if self.stack.len() >= self.safe_option.max_stack_size {
            self.error(format!(
                "stack overflow: attempted to push value exceeding maximum {}",
                self.safe_option.max_stack_size
            ));
            return Err(LuaError::StackOverflow);
        }
        self.stack.push(value);
        
        // Update current frame's top to reflect the new stack top
        // This is crucial for C functions that push results
        let new_top = self.stack.len();
        if let Some(frame) = self.current_frame_mut() {
            frame.top = new_top;
        }
        
        Ok(())
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

    pub fn create_string_owned(&mut self, s: String) -> LuaValue {
        self.vm_mut().create_string_owned(s)
    }

    /// Create userdata
    pub fn create_userdata(&mut self, data: LuaUserdata) -> LuaValue {
        self.vm_mut().create_userdata(data)
    }

    /// Get userdata reference
    pub fn get_userdata(&self, value: &LuaValue) -> Option<&LuaUserdata> {
        let vm = unsafe { &*self.vm };
        vm.get_userdata(value)
    }

    /// Get mutable userdata reference
    pub fn get_userdata_mut(&mut self, value: &LuaValue) -> Option<&mut LuaUserdata> {
        self.vm_mut().get_userdata_mut(value)
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

    /// Set value in table with metatable support
    pub fn table_set_with_meta(
        &mut self,
        table: LuaValue,
        key: LuaValue,
        value: LuaValue,
    ) -> LuaResult<()> {
        self.vm_mut().table_set_with_meta(table, key, value)
    }

    /// Get string from value
    pub fn get_string(&self, value: &LuaValue) -> Option<&LuaString> {
        let vm = unsafe { &*self.vm };
        vm.get_string(value)
    }

    // ===== Protected Call (pcall/xpcall) =====

    /// Protected call - execute function with error handling (pcall semantics)
    /// Returns (success, results) where:
    /// - success=true, results=return values
    /// - success=false, results=[error_message]
    /// Note: Yields are NOT caught by pcall - they propagate through
    pub fn pcall(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Save state for cleanup
        let initial_depth = self.call_depth();
        let func_idx = self.stack.len();

        // Check if it's a C function - handle differently
        let is_c_function = if func.is_cfunction() {
            true
        } else if let Some(func_id) = func.as_function_id() {
            let vm = unsafe { &*self.vm };
            vm.object_pool
                .get_function(func_id)
                .map(|f| f.is_c_function())
                .unwrap_or(false)
        } else {
            false
        };

        if is_c_function {
            // C function - call directly
            self.stack.push(func);
            let nargs = args.len();
            for arg in args {
                self.stack.push(arg);
            }

            // Get the C function pointer
            let cfunc = if func.is_cfunction() {
                func.as_cfunction()
            } else if let Some(func_id) = func.as_function_id() {
                let vm = unsafe { &*self.vm };
                vm.object_pool
                    .get_function(func_id)
                    .and_then(|f| f.c_function())
            } else {
                None
            };

            if let Some(cfunc) = cfunc {
                // Create frame for C function
                let base = func_idx + 1;
                if let Err(_) = self.push_frame(func, base, nargs, -1) {
                    self.stack.truncate(func_idx);
                    let error_msg = std::mem::take(&mut self.error_msg);
                    let err_str = self.create_string(&error_msg);
                    return Ok((false, vec![err_str]));
                }

                // Call C function
                let result = cfunc(self);

                // Pop frame
                self.pop_frame();

                match result {
                    Ok(nresults) => {
                        // Success - collect results
                        let mut results = Vec::new();
                        let result_start = if self.stack.len() >= nresults {
                            self.stack.len() - nresults
                        } else {
                            0
                        };

                        for i in result_start..self.stack.len() {
                            if let Some(val) = self.stack_get(i) {
                                results.push(val);
                            }
                        }

                        // Clean up stack
                        self.stack.truncate(func_idx);

                        Ok((true, results))
                    }
                    Err(LuaError::Yield) => Err(LuaError::Yield),
                    Err(_) => {
                        let error_msg = std::mem::take(&mut self.error_msg);
                        let err_str = self.create_string(&error_msg);
                        self.stack.truncate(func_idx);
                        Ok((false, vec![err_str]))
                    }
                }
            } else {
                let err_str = self.create_string("not a function");
                Ok((false, vec![err_str]))
            }
        } else {
            // Lua function - use lua_execute
            self.stack.push(func);
            let nargs = args.len();
            for arg in args {
                self.stack.push(arg);
            }

            // Create call frame
            let base = func_idx + 1;
            // pcall expects all return values
            if let Err(_) = self.push_frame(func, base, nargs, -1) {
                self.stack.truncate(func_idx);
                let error_msg = std::mem::take(&mut self.error_msg);
                let err_str = self.create_string(&error_msg);
                return Ok((false, vec![err_str]));
            }

            // Execute via lua_execute_until - only execute the new frame
            let result = crate::lua_vm::execute::lua_execute_until(self, initial_depth);

            match result {
                Ok(()) => {
                    // Success - collect return values from stack
                    let mut results = Vec::new();
                    for i in func_idx..self.stack.len() {
                        if let Some(val) = self.stack_get(i) {
                            results.push(val);
                        }
                    }

                    // Clean up stack
                    self.stack.truncate(func_idx);

                    Ok((true, results))
                }
                Err(LuaError::Yield) => Err(LuaError::Yield),
                Err(_) => {
                    // Error occurred - clean up

                    // Close upvalues
                    if self.call_depth() > initial_depth {
                        if let Some(frame_base) = self.call_stack.get(initial_depth).map(|f| f.base)
                        {
                            let vm = unsafe { &mut *self.vm };
                            self.close_upvalues(frame_base, &mut vm.object_pool);
                        }
                    }

                    // Pop frames
                    while self.call_depth() > initial_depth {
                        self.pop_frame();
                    }

                    // Get error message
                    let error_msg = std::mem::take(&mut self.error_msg);
                    let err_str = self.create_string(&error_msg);

                    // Clean up stack
                    self.stack.truncate(func_idx);

                    Ok((false, vec![err_str]))
                }
            }
        }
    }

    /// Protected call with stack-based arguments (zero-allocation fast path)
    /// Args are already on stack at [arg_base, arg_base+arg_count)
    /// Returns (success, result_count) where results are left on stack
    pub fn pcall_stack_based(
        &mut self,
        func_idx: usize,
        arg_count: usize,
    ) -> LuaResult<(bool, usize)> {
        // Save current call stack depth
        let initial_depth = self.call_depth();

        // Get function from stack
        let func = match self.stack_get(func_idx) {
            Some(f) => f,
            None => {
                self.error("pcall: invalid function index".to_string());
                let err_str = self.create_string("pcall: invalid function index");
                self.stack.truncate(func_idx);
                self.stack.push(err_str);
                return Ok((false, 1));
            }
        };

        // Call the function using the internal call machinery
        // This handles both C and Lua functions correctly
        let result = if func.is_cfunction() || func.as_function_id().and_then(|id| {
            unsafe { &*self.vm }.object_pool.get_function(id).and_then(|f| f.c_function())
        }).is_some() {
            // C function - call directly
            crate::lua_vm::execute::call::call_c_function(
                self,
                func_idx,
                arg_count,
                0, // MULTRET - want all results
            ).map(|_| ())
        } else {
            // Lua function - push frame and execute, expecting all return values
            let base = func_idx + 1;
            self.push_frame(func, base, arg_count, -1)?;
            crate::lua_vm::execute::lua_execute_until(self, initial_depth)
        };

        match result {
            Ok(()) => {
                // Success - count results from func_idx to stack top
                let result_count = if self.stack.len() > func_idx {
                    self.stack.len() - func_idx
                } else {
                    0
                };
                Ok((true, result_count))
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(_) => {
                // Error - clean up and return error message

                // Close upvalues
                if self.call_depth() > initial_depth {
                    if let Some(frame_base) = self.call_stack.get(initial_depth).map(|f| f.base) {
                        let vm = unsafe { &mut *self.vm };
                        self.close_upvalues(frame_base, &mut vm.object_pool);
                    }
                }

                // Pop frames
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                // Get error and push to stack
                let error_msg = std::mem::take(&mut self.error_msg);
                let err_str = self.create_string(&error_msg);

                self.stack.truncate(func_idx);
                self.stack.push(err_str);

                Ok((false, 1))
            }
        }
    }

    /// Protected call with error handler (xpcall semantics)
    /// The error handler is called if an error occurs
    /// Returns (success, results)
    pub fn xpcall(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
        err_handler: LuaValue,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Save error handler and function on stack
        let handler_idx = self.stack.len();
        self.stack.push(err_handler);

        let initial_depth = self.call_depth();
        let func_idx = self.stack.len();
        self.stack.push(func);

        let nargs = args.len();
        for arg in args {
            self.stack.push(arg);
        }

        // Create call frame, expecting all return values
        let base = func_idx + 1;
        if let Err(_) = self.push_frame(func, base, nargs, -1) {
            // Error during setup
            self.stack.truncate(handler_idx);
            let error_msg = std::mem::take(&mut self.error_msg);
            let err_str = self.create_string(&error_msg);
            return Ok((false, vec![err_str]));
        }

        // Execute
        let result = crate::lua_vm::execute::lua_execute_until(self, initial_depth);

        match result {
            Ok(()) => {
                // Success - collect results from func_idx to stack top
                let mut results = Vec::new();
                for i in func_idx..self.stack.len() {
                    if let Some(val) = self.stack_get(i) {
                        results.push(val);
                    }
                }

                self.stack.truncate(handler_idx);
                Ok((true, results))
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(_) => {
                // Error occurred - call error handler
                let error_msg = self.error_msg.clone();

                // Clean up failed frames
                if self.call_depth() > initial_depth {
                    if let Some(frame_base) = self.call_stack.get(initial_depth).map(|f| f.base) {
                        let vm = unsafe { &mut *self.vm };
                        self.close_upvalues(frame_base, &mut vm.object_pool);
                    }
                }

                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                // Set up error handler call
                self.stack.truncate(handler_idx + 1); // Keep error handler

                // Push error message as argument
                let err_value = self.create_string(&error_msg);
                self.stack.push(err_value);

                // Get handler and create frame, expecting all return values
                let handler = self.stack_get(handler_idx).unwrap_or(LuaValue::nil());
                let handler_base = handler_idx + 1;

                if let Err(_) = self.push_frame(handler, handler_base, 1, -1) {
                    // Error handler setup failed
                    self.stack.truncate(handler_idx);
                    let final_err =
                        self.create_string(&format!("error in error handling: {}", error_msg));
                    return Ok((false, vec![final_err]));
                }

                // Execute error handler
                let handler_result = crate::lua_vm::execute::lua_execute_until(self, initial_depth);

                match handler_result {
                    Ok(()) => {
                        // Error handler succeeded - collect results from handler_idx
                        let mut results = Vec::new();
                        for i in handler_idx..self.stack.len() {
                            if let Some(val) = self.stack_get(i) {
                                results.push(val);
                            }
                        }

                        if results.is_empty() {
                            results.push(self.create_string(&error_msg));
                        }

                        self.stack.truncate(handler_idx);
                        Ok((false, results))
                    }
                    Err(_) => {
                        // Error handler failed
                        self.stack.truncate(handler_idx);
                        let final_err =
                            self.create_string(&format!("error in error handling: {}", error_msg));
                        Ok((false, vec![final_err]))
                    }
                }
            }
        }
    }

    // ===== Coroutine Support (resume/yield) =====

    /// Resume a coroutine (should be called on the thread's LuaState)
    /// Returns (finished, results) where:
    /// - finished=true: coroutine completed normally
    /// - finished=false: coroutine yielded
    pub fn resume(&mut self, args: Vec<LuaValue>) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Check if this is the first resume (no frames yet)
        if self.call_stack.is_empty() {
            // Initial resume - need to set up the function
            // The function should be at stack[0] (set by create_thread)
            if self.stack.is_empty() {
                return Err(self.error("cannot resume dead coroutine".to_string()));
            }

            let func = self.stack[0];

            // Push arguments
            for arg in args {
                self.stack.push(arg);
            }

            // Create initial frame, expecting all return values
            let nargs = self.stack.len() - 1; // -1 for function itself
            let base = 1; // Arguments start at index 1 (function is at 0)
            self.push_frame(func, base, nargs, -1)?;

            // Execute until yield or completion
            let result = crate::lua_vm::execute::lua_execute(self);

            match result {
                Ok(()) => {
                    // Coroutine completed - collect return values from stack[0..]
                    let results = self.get_all_return_values(0);
                    self.stack.clear();
                    Ok((true, results))
                }
                Err(LuaError::Yield) => {
                    // Coroutine yielded
                    let yield_vals = self.take_yield();
                    Ok((false, yield_vals))
                }
                Err(e) => Err(e),
            }
        } else {
            // Resuming after yield
            let has_yield_values = !self.yield_values.is_empty();

            if has_yield_values {
                // Restore yield values to stack
                let yield_vals = self.take_yield();

                if let Some(frame) = self.current_frame() {
                    let return_base = frame.base;

                    if let Err(_) = self.grow_stack(return_base + yield_vals.len()) {
                        return Err(self.error("stack overflow during resume".to_string()));
                    }

                    for (i, val) in yield_vals.into_iter().enumerate() {
                        let _ = self.stack_set(return_base + i, val);
                    }
                }
            } else {
                // Push new arguments onto stack at the current frame's base
                // These will become the return values of the yield call
                if let Some(frame) = self.current_frame() {
                    let base = frame.base;

                    // Ensure stack has enough space
                    if let Err(_) = self.grow_stack(base + args.len()) {
                        return Err(self.error("stack overflow during resume".to_string()));
                    }

                    // Place args at base positions (they become yield's return values)
                    for (i, arg) in args.into_iter().enumerate() {
                        let _ = self.stack_set(base + i, arg);
                    }
                }
            }

            // Execute until yield or completion
            let result = crate::lua_vm::execute::lua_execute(self);

            match result {
                Ok(()) => {
                    // Coroutine completed
                    let results = self.get_all_return_values(0);
                    self.stack.clear();
                    Ok((true, results))
                }
                Err(LuaError::Yield) => {
                    // Coroutine yielded
                    let yield_vals = self.take_yield();
                    Ok((false, yield_vals))
                }
                Err(e) => Err(e),
            }
        }
    }

    /// Yield from current coroutine
    /// This should be called by Lua code via coroutine.yield
    pub fn do_yield(&mut self, values: Vec<LuaValue>) -> LuaResult<()> {
        self.set_yield(values);
        Err(LuaError::Yield)
    }
}

impl Default for LuaState {
    fn default() -> Self {
        Self::new(1, std::ptr::null_mut(), SafeOption::default())
    }
}
