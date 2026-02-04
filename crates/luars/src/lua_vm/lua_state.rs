// Lua execution state (equivalent to lua_State in Lua C API)
// Represents a single thread/coroutine execution context
// Multiple LuaStates can share the same LuaVM (global_State)

use std::collections::HashMap;
use std::rc::Rc;

use crate::branch::unlikely;
use crate::lua_value::{LuaUpvalue, LuaUserdata, LuaValue, LuaValueKind, LuaValuePtr};
use crate::lua_vm::call_info::call_status::{CIST_C, CIST_LUA};
use crate::lua_vm::execute::call::{call_c_function, resolve_call_chain};
use crate::lua_vm::execute::{self, lua_execute};
use crate::lua_vm::safe_option::SafeOption;
use crate::lua_vm::{CallInfo, LuaError, LuaResult, TmKind, get_metamethod_event};
use crate::{Chunk, CreateResult, GcObjectPtr, LuaVM, StringPtr, ThreadPtr, UpvaluePtr};

/// Execution state for a Lua thread/coroutine
/// This is separate from LuaVM (global_State) to support multiple execution contexts
pub struct LuaState {
    vm: *mut LuaVM,

    thread: ThreadPtr,
    /// Data stack - stores all values (registers, temporaries, function arguments)
    /// Layout: [frame0_values...][frame1_values...][frame2_values...]
    /// Similar to Lua's TValue stack[] in lua_State
    /// IMPORTANT: This is the PHYSICAL stack, only grows, never shrinks
    pub(crate) stack: Vec<LuaValue>,

    /// Logical stack top - index of first free slot (Lua's L->top.p)
    /// This is the actual "top" that controls which stack slots are active
    /// Values above stack_top are considered "garbage" and can be reused
    pub(crate) stack_top: usize,

    /// Call stack - one CallInfo per active function call
    /// Grows dynamically on demand (like Lua 5.4's linked list approach)
    /// Similar to Lua's CallInfo *ci in lua_State
    pub(crate) call_stack: Vec<CallInfo>,

    /// Current call depth (index into call_stack)
    /// This is the actual depth, NOT call_stack.len()
    /// Implements Lua's optimization: never shrink call_stack, only move this index
    call_depth: usize,

    /// C call depth - tracks C function call nesting (like Lua's nCcalls)
    /// This counter is INDEPENDENT of tail call optimization:
    /// - Incremented on every C function entry (pcall, xpcall, C closures, etc.)
    /// - Decremented only on C function return
    /// - NOT affected by Lua tail call optimization
    /// This allows detection of C-stack overflow even when Lua stack is optimized
    c_call_depth: usize,

    /// Open upvalues - upvalues pointing to stack locations
    /// Uses HashMap for O(1) lookup by stack index (unlike Lua's sorted linked list)
    /// Also maintains a sorted Vec(higher indices first) for efficient traversal during close operations
    open_upvalues_map: HashMap<usize, UpvaluePtr>,
    open_upvalues_list: Vec<UpvaluePtr>,

    /// Error message storage (lightweight error handling)
    error_msg: String,

    /// Yield values storage (for coroutine yield)
    yield_values: Vec<LuaValue>,

    /// Hook mask and count (for debug hooks)
    _hook_mask: u8,
    _hook_count: i32,

    safe_option: SafeOption,

    is_main: bool,
}

impl LuaState {
    /// Basic stack size (similar to BASIC_STACK_SIZE in Lua = 2*LUA_MINSTACK = 40)
    const BASIC_STACK_SIZE: usize = 40;

    /// Create a new execution state
    pub fn new(
        call_stack_size: usize,
        vm: *mut LuaVM,
        is_main: bool,
        safe_option: SafeOption,
    ) -> Self {
        Self {
            vm,
            stack: Vec::with_capacity(Self::BASIC_STACK_SIZE),
            thread: ThreadPtr::null(),
            stack_top: 0, // Start with empty stack (Lua's L->top.p = L->stack)
            call_stack: Vec::with_capacity(call_stack_size),
            call_depth: 0,
            c_call_depth: 0, // Start with no calls
            open_upvalues_map: HashMap::new(),
            open_upvalues_list: Vec::new(),
            error_msg: String::new(),
            yield_values: Vec::new(),
            _hook_mask: 0,
            _hook_count: 0,
            safe_option,
            is_main,
        }
    }

    // please donot use this function directly unless you are very sure of what you are doing
    pub(crate) unsafe fn thread_ptr(&self) -> ThreadPtr {
        self.thread
    }

    pub(crate) unsafe fn set_thread_ptr(&mut self, thread: ThreadPtr) {
        self.thread = thread;
    }

    /// Remove a dead string from the intern map (called by GC during sweep)
    pub(crate) fn remove_dead_string(&mut self, str_ptr: StringPtr) {
        unsafe {
            (*self.vm).object_allocator.remove_str(str_ptr);
        }
    }

    /// Get current call frame (equivalent to Lua's L->ci)
    #[inline(always)]
    pub fn current_frame(&self) -> Option<&CallInfo> {
        if self.call_depth > 0 {
            self.call_stack.get(self.call_depth - 1)
        } else {
            None
        }
    }

    /// Get mutable current call frame
    #[inline(always)]
    pub fn current_frame_mut(&mut self) -> Option<&mut CallInfo> {
        if self.call_depth > 0 {
            self.call_stack.get_mut(self.call_depth - 1)
        } else {
            None
        }
    }

    /// Get call stack depth
    #[inline(always)]
    pub fn call_depth(&self) -> usize {
        self.call_depth
    }

    /// Get C call depth
    #[inline(always)]
    pub fn c_call_depth(&self) -> usize {
        self.c_call_depth
    }

    /// Push a new call frame (equivalent to Lua's luaD_precall)
    /// OPTIMIZED: Reuses CallInfo slots, only allocates when needed
    ///
    /// PERFORMANCE CRITICAL: This function is called on every function invocation
    /// Optimizations:
    /// - Assumes func is callable (checked by caller)
    /// - Caches as_lua_function() result to avoid repeated enum matching
    /// - Uses batch nil filling instead of loops
    /// - Minimizes branches in hot path
    #[inline]
    pub(crate) fn push_frame(
        &mut self,
        func: LuaValue,
        base: usize,
        nparams: usize,
        nresults: i32,
    ) -> LuaResult<()> {
        // Fast path: check stack depth (branch predictor friendly - usually succeeds)
        if unlikely(self.call_depth >= self.safe_option.max_call_depth) {
            return Err(self.error(format!(
                "stack overflow (Lua stack depth: {})",
                self.call_depth
            )));
        }

        // Cache lua_function extraction (avoid repeated enum matching)
        // This single call replaces multiple is_c_function/as_lua_function checks
        let lua_func = func.as_lua_function();

        // Determine function type and extract metadata in one pass
        let (is_c_function, maxstacksize, numparams, nextraargs) = if let Some(func_obj) = lua_func
        {
            if func_obj.is_c_function() {
                // C function path
                (true, nparams, nparams, 0)
            } else if let Some(chunk) = func_obj.chunk() {
                // Lua function with chunk
                let numparams = chunk.param_count;
                let nextraargs = if nparams > numparams {
                    (nparams - numparams) as i32
                } else {
                    0
                };
                (false, chunk.max_stack_size as usize, numparams, nextraargs)
            } else {
                // Lua function without chunk (shouldn't happen, but handle gracefully)
                (false, nparams, nparams, 0)
            }
        } else if func.is_cfunction() {
            // Light C function
            (true, nparams, nparams, 0)
        } else {
            // Not callable - this should be prevented by caller
            debug_assert!(false, "push_frame called with non-callable value");
            return Err(self.error(format!("attempt to call a {} value", func.type_name())));
        };

        // Check C call depth if needed
        if is_c_function {
            if unlikely(self.c_call_depth >= self.safe_option.max_call_depth) {
                return Err(self.error(format!(
                    "C stack overflow (C call depth: {})",
                    self.c_call_depth
                )));
            }
            self.c_call_depth += 1;
        }

        // Fill missing parameters with nil (optimized batch operation)
        if nparams < numparams {
            let start = base + nparams;
            let end = base + numparams;

            // Ensure stack capacity
            if self.stack.len() < end {
                self.stack.resize(end, LuaValue::nil());
            } else {
                // Batch fill with nil (faster than loop)
                self.stack[start..end].fill(LuaValue::nil());
            }

            // Update stack_top if necessary
            if self.stack_top < end {
                self.stack_top = end;
            }
        }

        let call_status = if is_c_function { CIST_C } else { CIST_LUA };
        let frame_top = base + maxstacksize;

        // Fast path: reuse existing CallInfo slot (most common case)
        if self.call_depth < self.call_stack.len() {
            let frame = unsafe { self.call_stack.get_unchecked_mut(self.call_depth) };
            frame.func = func;
            frame.base = base;
            frame.func_offset = 1;
            frame.top = frame_top;
            frame.pc = 0;
            frame.nresults = nresults;
            frame.call_status = call_status;
            frame.nextraargs = nextraargs;
        } else {
            // Slow path: allocate new CallInfo (first time reaching this depth)
            self.call_stack.push(CallInfo {
                func,
                base,
                func_offset: 1,
                top: frame_top,
                pc: 0,
                nresults,
                call_status,
                nextraargs,
            });
        }

        self.call_depth += 1;
        Ok(())
    }

    /// Pop call frame (equivalent to Lua's luaD_poscall)
    /// OPTIMIZED: Only decrements depth, never releases memory (Lua-style)
    #[inline]
    pub(crate) fn pop_frame(&mut self) -> Option<CallInfo> {
        if self.call_depth > 0 {
            self.call_depth -= 1;
            let frame = self.call_stack.get(self.call_depth).cloned();

            // 如果是C函数帧，减少C调用深度
            if let Some(ref f) = frame {
                if f.is_c() {
                    if self.c_call_depth > 0 {
                        self.c_call_depth -= 1;
                    }
                }
            }

            frame
        } else {
            None
        }
    }

    /// Get logical stack top (L->top.p in Lua source)
    /// This is the first free slot in the stack, NOT the length of physical stack
    #[inline(always)]
    pub fn get_top(&self) -> usize {
        self.stack_top
    }

    /// Set logical stack top (L->top.p = L->stack + new_top in Lua)
    /// This only updates the logical pointer, does NOT truncate the physical stack
    /// Old values remain in stack array but are considered "garbage"
    #[inline(always)]
    pub fn set_top(&mut self, new_top: usize) -> LuaResult<()> {
        // Ensure physical stack is large enough
        if new_top > self.stack.len() {
            self.resize(new_top)?;
        }
        self.stack_top = new_top;

        Ok(())
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
            self.resize(index + 1)?;
        }
        self.stack[index] = value;
        Ok(())
    }

    fn resize(&mut self, new_size: usize) -> LuaResult<()> {
        if new_size > self.safe_option.max_stack_size {
            self.error(format!(
                "stack overflow: attempted to resize to {} exceeding maximum {}",
                new_size, self.safe_option.max_stack_size
            ));
            return Err(LuaError::StackOverflow);
        }
        self.stack.resize(new_size, LuaValue::nil());
        for upval_ptr in &self.open_upvalues_list {
            if let LuaUpvalue::Open {
                stack_index,
                stack_ptr,
            } = &mut upval_ptr.as_mut_ref().data
            {
                // Update cached pointer to new stack location
                if *stack_index < self.stack.len() {
                    stack_ptr.ptr = (&self.stack[*stack_index]) as *const LuaValue as *mut LuaValue;
                }
            }
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

    /// Get open upvalues list
    #[inline(always)]
    pub fn open_upvalues(&self) -> &[UpvaluePtr] {
        &self.open_upvalues_list
    }

    /// Get mutable open upvalues list
    #[inline(always)]
    pub fn open_upvalues_mut(&mut self) -> &mut Vec<UpvaluePtr> {
        &mut self.open_upvalues_list
    }

    /// Set error message (without traceback - will be added later by top-level handler)
    #[inline(always)]
    pub fn error(&mut self, msg: String) -> LuaError {
        // Try to get current source location for the error
        let mut location = String::new();
        if let Some(ci) = self.current_frame() {
            if ci.is_lua() {
                if let Some(func_obj) = ci.func.as_lua_function() {
                    if let Some(chunk) = func_obj.chunk() {
                        let source = chunk.source_name.as_deref().unwrap_or("[string]");
                        let line = if ci.pc > 0 && (ci.pc as usize - 1) < chunk.line_info.len() {
                            chunk.line_info[ci.pc as usize - 1] as usize
                        } else if !chunk.line_info.is_empty() {
                            chunk.line_info[0] as usize
                        } else {
                            0
                        };
                        location = if line > 0 {
                            format!("{}:{}: ", source, line)
                        } else {
                            format!("{}: ", source)
                        };
                    }
                }
            }
        };

        self.error_msg = format!("{}{}", location, msg);
        LuaError::RuntimeError
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

        // Only iterate through valid frames (up to call_depth)
        // call_stack Vec may contain residual data beyond call_depth
        let valid_frames = &self.call_stack[..self.call_depth];

        // Iterate through call stack from newest to oldest
        // Start from level 0 (most recent frame, not counting the error frame itself)
        for (level, ci) in valid_frames.iter().rev().enumerate() {
            if level >= 20 {
                result.push_str("\t...\n");
                break;
            }

            // Get function info
            if ci.is_lua() {
                // Lua function - get source and line info

                if let Some(func_obj) = ci.func.as_lua_function() {
                    if let Some(chunk) = func_obj.chunk() {
                        let source = chunk.source_name.as_deref().unwrap_or("[string]");

                        // Format source name (strip @ prefix if present)
                        let source_display = if source.starts_with('@') {
                            &source[1..]
                        } else {
                            source
                        };

                        // Get current line number from PC
                        let line = if ci.pc > 0 && (ci.pc as usize - 1) < chunk.line_info.len() {
                            chunk.line_info[ci.pc as usize - 1] as usize
                        } else if !chunk.line_info.is_empty() {
                            chunk.line_info[0] as usize
                        } else {
                            0
                        };

                        // Determine if this is the main chunk
                        // Main chunk has linedefined == 0
                        // Also check if this is at the bottom of the valid call stack
                        let is_main = chunk.linedefined == 0 || level == valid_frames.len() - 1;
                        let what = if is_main { "main chunk" } else { "function" };

                        if line > 0 {
                            result
                                .push_str(&format!("\t{}:{}: in {}\n", source_display, line, what));
                        } else {
                            result.push_str(&format!("\t{}: in {}\n", source_display, what));
                        }
                        continue;
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
    /// Close upvalues from a given stack index upwards
    /// This is called when exiting a function or block scope
    pub fn close_upvalues(&mut self, level: usize) {
        // Optimization: The list is sorted by stack index descending (higher indices first).
        // Upvalues to close (index >= level) are at the beginning of the list.
        // We scan to find the cutoff point.
        let mut count = 0;
        let len = self.open_upvalues_list.len();

        while count < len {
            let upval_ptr = self.open_upvalues_list[count];
            // Check if this upvalue points to a stack index >= level
            let should_close = match upval_ptr.as_ref().data.get_stack_index() {
                Some(stack_idx) => stack_idx >= level,
                None => true, // Already closed (stale state), remove it
            };

            if !should_close {
                // Since list is sorted descending, if this one is < level, the rest are too.
                break;
            }
            count += 1;
        }

        if count > 0 {
            // Batch remove all closed upvalues from the list (efficient O(M) shift via drain)
            let to_close: Vec<UpvaluePtr> = self.open_upvalues_list.drain(0..count).collect();

            // Perform the close operation for each
            for upval_ptr in to_close {
                // 1. Identify stack index (must check before closing as closing removes it)
                let stack_idx_opt = upval_ptr.as_ref().data.get_stack_index();

                if let Some(stack_idx) = stack_idx_opt {
                    // 2. Remove from map (maintain consistency)
                    self.open_upvalues_map.remove(&stack_idx);

                    // 3. Capture value from stack
                    let value = self
                        .stack
                        .get(stack_idx)
                        .copied()
                        .unwrap_or(LuaValue::nil());

                    // 4. Close the upvalue (move value to heap)
                    upval_ptr.as_mut_ref().data.close(value);
                    let gc_ptr = GcObjectPtr::Upvalue(upval_ptr);

                    if let Some(header) = gc_ptr.header_mut() {
                        if !header.is_white() {
                            // nw2black(uv);  /* closed upvalues cannot be gray */
                            // luaC_barrier(L, uv, slot);
                            header.make_black();
                            if let Some(value_gc_ptr) = value.as_gc_ptr() {
                                self.gc_barrier(upval_ptr, value_gc_ptr);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Find or create an open upvalue for the given stack index
    /// Uses HashMap for O(1) lookup instead of O(n) linear search
    /// This is a major optimization over the naive linked list approach
    pub fn find_or_create_upvalue(&mut self, stack_index: usize) -> LuaResult<UpvaluePtr> {
        // O(1) lookup in HashMap
        if let Some(&upval_ptr) = self.open_upvalues_map.get(&stack_index) {
            return Ok(upval_ptr);
        }

        // Not found, create a new one
        let upval_ptr = {
            let ptr = LuaValuePtr {
                ptr: (&self.stack[stack_index]) as *const LuaValue as *mut LuaValue,
            };
            let vm = self.vm_mut();
            vm.create_upvalue_open(stack_index, ptr)?
        };

        // Add to HashMap for O(1) future lookups
        self.open_upvalues_map.insert(stack_index, upval_ptr);

        // Also add to sorted list for traversal (insert in sorted position, higher indices first)
        // Collect existing upvalue IDs and their stack indices
        let insert_pos = {
            self.open_upvalues_list
                .iter()
                .filter_map(|&ptr| ptr.as_ref().data.get_stack_index())
                .position(|idx| idx < stack_index)
                .unwrap_or(self.open_upvalues_list.len())
        };

        self.open_upvalues_list.insert(insert_pos, upval_ptr);

        Ok(upval_ptr)
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
    pub(crate) fn stack_mut(&mut self) -> &mut [LuaValue] {
        &mut self.stack
    }

    /// Get stack length
    #[inline(always)]
    pub fn stack_len(&self) -> usize {
        self.stack.len()
    }

    /// Truncate stack to specified length
    /// Used after function calls to remove temporary values
    pub fn stack_truncate(&mut self) {
        let new_len = 0;
        if new_len < self.stack.len() {
            for upval_ptr in &self.open_upvalues_list {
                let upval = &mut upval_ptr.as_mut_ref().data;
                if upval.is_open() {
                    if let Some(stack_idx) = upval.get_stack_index() {
                        if stack_idx >= new_len {
                            // Invalidate upvalue pointing to truncated stack
                            upval.close(self.stack[stack_idx]);
                        }
                    }
                }
            }

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
            self.resize(needed)?;
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
        // Only return PC if frame is within current call_depth (valid frames)
        if frame_idx < self.call_depth {
            self.call_stack.get(frame_idx).map(|f| f.pc).unwrap_or(0)
        } else {
            0
        }
    }

    /// Get frame function by index
    #[inline(always)]
    pub fn get_frame_func(&self, frame_idx: usize) -> Option<LuaValue> {
        // Only return frame if it's within current call_depth (valid frames)
        if frame_idx < self.call_depth {
            self.call_stack.get(frame_idx).map(|f| f.func)
        } else {
            None
        }
    }

    /// Get frame by index (for GC root collection)
    #[inline(always)]
    pub fn get_frame(&self, frame_idx: usize) -> Option<&CallInfo> {
        // Only return frame if it's within current call_depth (valid frames)
        if frame_idx < self.call_depth {
            self.call_stack.get(frame_idx)
        } else {
            None
        }
    }

    /// Get all open upvalues (for GC root collection)
    #[inline(always)]
    pub fn get_open_upvalues(&self) -> &[UpvaluePtr] {
        &self.open_upvalues_list
    }

    /// Set frame PC by index
    #[inline(always)]
    pub fn set_frame_pc(&mut self, frame_idx: usize, pc: u32) {
        if let Some(frame) = self.call_stack.get_mut(frame_idx) {
            frame.pc = pc;
        }
    }

    /// Set frame top by index (for tail calls)
    #[inline(always)]
    pub fn set_frame_top(&mut self, frame_idx: usize, top: usize) {
        if let Some(frame) = self.call_stack.get_mut(frame_idx) {
            frame.top = top;
        }
    }

    /// Set frame function by index (for tail calls)
    #[inline(always)]
    pub fn set_frame_func(&mut self, frame_idx: usize, func: LuaValue) {
        // Validate that func is callable before setting it
        let is_callable =
            func.is_function() || func.is_cfunction() || func.as_lua_function().is_some();

        if !is_callable {
            // This should not happen in correct code, but防御性编程
            eprintln!(
                "WARNING: Attempting to set non-callable value as frame func: {:?}",
                func
            );
            return;
        }

        if let Some(frame) = self.call_stack.get_mut(frame_idx) {
            frame.func = func;
        }
    }

    /// Set frame nextraargs by index (for tail calls)
    #[inline(always)]
    pub fn set_frame_nextraargs(&mut self, frame_idx: usize, nextraargs: i32) {
        if let Some(frame) = self.call_stack.get_mut(frame_idx) {
            frame.nextraargs = nextraargs;
        }
    }

    pub fn set_frame_call_status(&mut self, frame_idx: usize, call_status: u32) {
        if let Some(frame) = self.call_stack.get_mut(frame_idx) {
            frame.call_status = call_status;
        }
    }

    pub(crate) fn vm_mut(&mut self) -> &mut LuaVM {
        unsafe { &mut *self.vm }
    }

    pub(crate) fn vm_ptr(&self) -> *mut LuaVM {
        self.vm
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

    /// Pop the current call frame
    #[inline]
    pub fn pop_call_frame(&mut self) {
        if self.call_depth > 0 {
            self.call_depth -= 1;
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
        let top = self.get_top();
        let count = if top > stack_base {
            top - stack_base
        } else {
            0
        };
        self.get_return_values(stack_base, count)
    }

    // ===== Function Argument Access =====

    /// Get all arguments for the current C function call
    /// Returns arguments starting from index 1 (index 0 is the function itself)
    pub fn get_args(&self) -> Vec<LuaValue> {
        if self.call_depth == 0 {
            return Vec::new();
        }

        let frame = &self.call_stack[self.call_depth - 1];
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
        if index == 0 || self.call_depth == 0 {
            return None;
        }

        let frame = &self.call_stack[self.call_depth - 1];
        let base = frame.base;
        let top = frame.top;

        // Arguments are 1-based: arg 1 is at base, arg 2 is at base+1, etc.
        // (NOT base+1, because C function frame.base already points to first arg)
        let stack_index = base + index - 1;

        // Check if argument position is within the frame's range and the stack
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
        if self.call_depth == 0 {
            return 0;
        }

        let frame = &self.call_stack[self.call_depth - 1];
        let base = frame.base;
        let top = frame.top;

        // Arguments are from base to top-1
        if top > base { top - base } else { 0 }
    }

    pub fn push_value(&mut self, value: LuaValue) -> LuaResult<()> {
        // Check stack limit (Lua's luaD_checkstack equivalent)
        if self.stack_top >= self.safe_option.max_stack_size {
            self.error(format!(
                "stack overflow: attempted to push value exceeding maximum {}",
                self.safe_option.max_stack_size
            ));
            return Err(LuaError::StackOverflow);
        }

        // Save current top before any borrows
        let current_top = self.stack_top;

        // Ensure physical stack is large enough (Lua's luaD_reallocstack equivalent)
        if current_top >= self.stack.len() {
            // 1.5 x growth strategy
            let mut new_size = current_top + current_top / 2;
            if new_size < current_top + 1 {
                new_size = current_top + 1;
            }
            if new_size > self.safe_option.max_stack_size {
                new_size = self.safe_option.max_stack_size;
            }
            self.resize(new_size)?;
        }

        // Write at logical top position (L->top.p->value = value)
        self.stack[current_top] = value;

        // Increment logical top (L->top.p++)
        let new_top = current_top + 1;
        self.stack_top = new_top;

        // Update current frame's top limit (CallInfo.top)
        if let Some(frame) = self.current_frame_mut() {
            frame.top = new_top;
        }

        Ok(())
    }

    // ===== Object Creation =====

    /// Create table
    pub fn create_table(&mut self, narr: usize, nrec: usize) -> CreateResult {
        self.vm_mut().create_table(narr, nrec)
    }

    /// Create function closure
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalues: Vec<UpvaluePtr>) -> CreateResult {
        self.vm_mut().create_function(chunk, upvalues)
    }

    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> LuaResult<UpvaluePtr> {
        self.vm_mut().create_upvalue_closed(value)
    }

    pub fn create_upvalue_open(
        &mut self,
        stack_index: usize,
        stack_ptr: LuaValuePtr,
    ) -> LuaResult<UpvaluePtr> {
        self.vm_mut().create_upvalue_open(stack_index, stack_ptr)
    }

    /// Create/intern string (automatically handles short string interning)
    pub fn create_string(&mut self, s: &str) -> CreateResult {
        self.vm_mut().create_string(s)
    }

    pub fn create_string_owned(&mut self, s: String) -> CreateResult {
        self.vm_mut().create_string_owned(s)
    }

    pub fn create_binary(&mut self, data: Vec<u8>) -> CreateResult {
        self.vm_mut().create_binary(data)
    }

    /// Create userdata
    pub fn create_userdata(&mut self, data: LuaUserdata) -> CreateResult {
        self.vm_mut().create_userdata(data)
    }

    // ===== Global Access =====

    /// Get global variable
    pub fn get_global(&mut self, name: &str) -> LuaResult<Option<LuaValue>> {
        self.vm_mut().get_global(name)
    }

    /// Set global variable
    pub fn set_global(&mut self, name: &str, value: LuaValue) -> LuaResult<()> {
        self.vm_mut().set_global(name, value)
    }

    // ===== Table Operations =====

    /// Get value from table
    pub fn raw_get(&mut self, table: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        self.vm_mut().raw_get(table, key)
    }

    /// Set value in table
    pub fn raw_set(&mut self, table: &LuaValue, key: LuaValue, value: LuaValue) -> bool {
        self.vm_mut().raw_set(table, key, value)
    }

    pub fn raw_geti(&mut self, table: &LuaValue, index: i64) -> Option<LuaValue> {
        self.vm_mut().raw_geti(table, index)
    }

    pub fn raw_seti(&mut self, table: &LuaValue, index: i64, value: LuaValue) -> bool {
        self.vm_mut().raw_seti(table, index, value)
    }

    pub fn get_error_msg(&mut self, e: LuaError) -> String {
        match e {
            LuaError::OutOfMemory => {
                format!("out of memory: {}", self.vm_mut().gc.get_error_message())
            }
            _ => {
                // Return just the error message without "Runtime Error: " prefix
                // to match Lua 5.5 behavior (pcall returns the raw error message)
                std::mem::take(&mut self.error_msg)
            }
        }
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

        // Push function and args to stack
        self.stack.push(func);
        for arg in args {
            self.stack.push(arg);
        }
        let arg_count = self.stack.len() - func_idx - 1;

        // Resolve __call chain if needed

        let (actual_arg_count, ccmt_depth) = match resolve_call_chain(self, func_idx, arg_count) {
            Ok((count, depth)) => (count, depth),
            Err(e) => {
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                self.stack.truncate(func_idx);
                return Ok((false, vec![err_str]));
            }
        };

        // Get resolved function
        let func = self
            .stack_get(func_idx)
            .ok_or_else(|| self.error("pcall: function not found".to_string()))?;

        // Check if it's a C function
        let is_c_function = if func.is_cfunction() {
            true
        } else if let Some(func_body) = func.as_lua_function() {
            func_body.is_c_function()
        } else {
            false
        };

        if is_c_function {
            // Get the C function pointer
            let cfunc = if func.is_cfunction() {
                func.as_cfunction()
            } else if let Some(func_body) = func.as_lua_function() {
                func_body.c_function()
            } else {
                None
            };

            if let Some(cfunc) = cfunc {
                // Create frame for C function
                let base = func_idx + 1;
                if let Err(e) = self.push_frame(func, base, actual_arg_count, -1) {
                    self.stack.truncate(func_idx);
                    let error_msg = self.get_error_msg(e);
                    let err_str = self.create_string(&error_msg)?;
                    return Ok((false, vec![err_str]));
                }

                // Set ccmt count in call_status
                if ccmt_depth > 0 {
                    use crate::lua_vm::call_info::call_status;
                    let frame_idx = self.call_depth - 1;
                    if let Some(frame) = self.call_stack.get_mut(frame_idx) {
                        frame.call_status =
                            call_status::set_ccmt_count(frame.call_status, ccmt_depth);
                    }
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
                    Err(e) => {
                        let error_msg = self.get_error_msg(e);
                        let err_str = self.create_string(&error_msg)?;
                        self.stack.truncate(func_idx);
                        Ok((false, vec![err_str]))
                    }
                }
            } else {
                let err_str = self.create_string("not a function")?;
                Ok((false, vec![err_str]))
            }
        } else {
            // Lua function - use lua_execute
            let base = func_idx + 1;
            // pcall expects all return values
            if let Err(e) = self.push_frame(func, base, actual_arg_count, -1) {
                self.stack.truncate(func_idx);
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                return Ok((false, vec![err_str]));
            }

            // Execute via lua_execute_until - only execute the new frame
            let result = execute::lua_execute(self, initial_depth);

            match result {
                Ok(()) => {
                    // Success - collect return values from stack
                    let mut results = Vec::new();
                    for i in func_idx..self.stack.len() {
                        if let Some(val) = self.stack_get(i) {
                            results.push(val);
                        }
                    }

                    // Ensure call_depth is back to initial_depth
                    // (normally RETURN should have handled this, but double-check)
                    while self.call_depth() > initial_depth {
                        self.pop_frame();
                    }

                    // Clean up stack
                    self.stack.truncate(func_idx);

                    Ok((true, results))
                }
                Err(LuaError::Yield) => Err(LuaError::Yield),
                Err(e) => {
                    // Error occurred - clean up

                    // Close upvalues
                    if self.call_depth() > initial_depth {
                        if let Some(frame_base) = self.call_stack.get(initial_depth).map(|f| f.base)
                        {
                            self.close_upvalues(frame_base);
                        }
                    }

                    // Pop frames
                    while self.call_depth() > initial_depth {
                        self.pop_frame();
                    }

                    // Get error message
                    let error_msg = self.get_error_msg(e);
                    let err_str = self.create_string(&error_msg)?;

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

        // Resolve __call metamethod chain if needed

        let (actual_arg_count, ccmt_depth) = match resolve_call_chain(self, func_idx, arg_count) {
            Ok((count, depth)) => (count, depth),
            Err(e) => {
                // __call resolution failed - return error
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                self.stack_set(func_idx, err_str)?;
                self.set_top(func_idx + 1)?;
                return Ok((false, 1));
            }
        };

        // Now func_idx contains a real callable (after __call resolution)
        let func = self
            .stack_get(func_idx)
            .ok_or_else(|| self.error("pcall: function not found after resolution".to_string()))?;

        // Call the function using the internal call machinery
        let result = if func.is_cfunction()
            || func
                .as_lua_function()
                .and_then(|f| f.c_function())
                .is_some()
        {
            // C function - call directly
            call_c_function(
                self,
                func_idx,
                actual_arg_count,
                -1, // MULTRET - want all results
            )
            .map(|_| ())
        } else {
            // Lua function - push frame and execute, expecting all return values
            let base = func_idx + 1;
            self.push_frame(func, base, actual_arg_count, -1)?;

            // Set ccmt count in call_status for Lua functions too
            if ccmt_depth > 0 {
                use crate::lua_vm::call_info::call_status;
                let frame_idx = self.call_depth - 1;
                if let Some(frame) = self.call_stack.get_mut(frame_idx) {
                    frame.call_status = call_status::set_ccmt_count(frame.call_status, ccmt_depth);
                }
            }

            lua_execute(self, initial_depth)
        };

        match result {
            Ok(()) => {
                // Success - count results from func_idx to logical stack top
                let stack_top = self.get_top();
                let result_count = if stack_top > func_idx {
                    stack_top - func_idx
                } else {
                    0
                };
                Ok((true, result_count))
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(e) => {
                // Error - clean up and return error message

                // Close upvalues
                if self.call_depth() > initial_depth {
                    if let Some(frame_base) = self.call_stack.get(initial_depth).map(|f| f.base) {
                        self.close_upvalues(frame_base);
                    }
                }

                // Pop frames
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                // Get error and push to stack
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;

                // Set error at func_idx and update stack top
                self.stack_set(func_idx, err_str)?;
                self.set_top(func_idx + 1)?;

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
        let handler_idx = self.stack_top;
        self.push_value(err_handler)?;

        let initial_depth = self.call_depth();
        let func_idx = self.stack_top;
        self.push_value(func)?;

        for arg in args {
            self.push_value(arg)?;
        }
        let arg_count = self.stack_top - func_idx - 1;

        // Resolve __call chain if needed

        let (actual_arg_count, ccmt_depth) = match resolve_call_chain(self, func_idx, arg_count) {
            Ok((count, depth)) => (count, depth),
            Err(e) => {
                self.set_top(handler_idx)?;
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                return Ok((false, vec![err_str]));
            }
        };

        // Get resolved function
        let func = self
            .stack_get(func_idx)
            .ok_or_else(|| self.error("xpcall: function not found".to_string()))?;

        // Create call frame, expecting all return values
        let base = func_idx + 1;
        if let Err(e) = self.push_frame(func, base, actual_arg_count, -1) {
            // Error during setup
            self.set_top(handler_idx)?;
            let error_msg = self.get_error_msg(e);
            let err_str = self.create_string(&error_msg)?;
            return Ok((false, vec![err_str]));
        }

        // Set ccmt count in call_status
        if ccmt_depth > 0 {
            use crate::lua_vm::call_info::call_status;
            let frame_idx = self.call_depth - 1;
            if let Some(frame) = self.call_stack.get_mut(frame_idx) {
                frame.call_status = call_status::set_ccmt_count(frame.call_status, ccmt_depth);
            }
        }

        // Execute
        let result = execute::lua_execute(self, initial_depth);

        match result {
            Ok(()) => {
                // Success - collect results
                // Execution (via RETURN) sets stack_top to end of results
                // Results start at func_idx (replacing func and args)
                let mut results = Vec::new();
                let top = self.stack_top;

                if top > func_idx {
                    for i in func_idx..top {
                        if let Some(val) = self.stack_get(i) {
                            results.push(val);
                        }
                    }
                }

                // Ensure call_depth is back to initial_depth
                // (normally RETURN should have handled this, but double-check)
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                self.set_top(handler_idx)?;
                Ok((true, results))
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(e) => {
                // Error occurred - call error handler
                let error_msg = self.get_error_msg(e);

                // Clean up failed frames
                if self.call_depth() > initial_depth {
                    if let Some(frame_base) = self.call_stack.get(initial_depth).map(|f| f.base) {
                        self.close_upvalues(frame_base);
                    }
                }

                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                // Set up error handler call
                // Reset stack to [handler]
                self.set_top(handler_idx + 1)?;

                // Push error message as argument
                let err_value = self.create_string(&error_msg)?;
                self.push_value(err_value)?;

                // Get handler and create frame
                let handler = self.stack_get(handler_idx).unwrap_or(LuaValue::nil());
                let handler_base = handler_idx + 1;

                if let Err(_) = self.push_frame(handler, handler_base, 1, -1) {
                    // Error handler setup failed
                    self.set_top(handler_idx)?;
                    let final_err =
                        self.create_string(&format!("error in error handling: {}", error_msg))?;
                    return Ok((false, vec![final_err]));
                }

                // Execute error handler
                let handler_result = execute::lua_execute(self, initial_depth);

                match handler_result {
                    Ok(()) => {
                        // Error handler succeeded
                        // Results start at handler_idx (replacing handler)
                        // Stack top is at end of results
                        let mut results = Vec::new();
                        let top = self.stack_top;

                        if top > handler_idx {
                            for i in handler_idx..top {
                                if let Some(val) = self.stack_get(i) {
                                    results.push(val);
                                }
                            }
                        }

                        if results.is_empty() {
                            results.push(self.create_string(&error_msg)?);
                        }

                        self.set_top(handler_idx)?;
                        Ok((false, results))
                    }
                    Err(_) => {
                        // Error handler failed - clean up its frame
                        while self.call_depth() > initial_depth {
                            self.pop_frame();
                        }

                        self.set_top(handler_idx)?;
                        let final_err =
                            self.create_string(&format!("error in error handling: {}", error_msg))?;
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

            // Check if function is C or Lua
            let is_c_function = func.is_cfunction()
                || (func.is_function()
                    && func.as_lua_function().map_or(false, |f| f.is_c_function()));

            // Execute until yield or completion
            let result = if is_c_function {
                // Call C function directly
                execute::call::call_c_function(self, 0, nargs, -1)
            } else {
                // Execute Lua bytecode
                execute::lua_execute(self, 0)
            };

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
            // The yield function's frame is still on the stack, we need to:
            // 1. Close upvalues for the yield frame
            // 2. Pop the yield frame
            // 3. Place resume arguments as yield's return values
            // 4. Continue execution from the caller's frame

            // Get the yield frame info before popping
            let (func_idx, frame_base, _nresults) = if let Some(frame) = self.current_frame() {
                // func_idx is base - 1 (where the yield function was called)
                let func_idx = if frame.base > 0 { frame.base - 1 } else { 0 };
                let frame_base = frame.base;
                let nresults = frame.nresults;
                (func_idx, frame_base, nresults)
            } else {
                return Err(self.error("cannot resume: no frame".to_string()));
            };

            //  Close upvalues before popping the frame
            // This ensures open upvalues don't point to invalid stack indices
            self.close_upvalues(frame_base);

            // Pop the yield frame
            self.pop_frame();

            // Place resume arguments at func_idx as yield's return values
            // This simulates the yield function returning normally
            let actual_nresults = args.len();
            for (i, arg) in args.into_iter().enumerate() {
                self.stack_set(func_idx + i, arg)?;
            }

            // Update stack top and current frame's top
            let new_top = func_idx + actual_nresults;
            self.set_top(new_top)?;

            if let Some(frame) = self.current_frame_mut() {
                frame.top = new_top;
            }

            // Execute until yield or completion
            let result = execute::lua_execute(self, 0);

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

    // ============ GC Barriers ============

    /// Forward GC barrier (luaC_barrier in Lua 5.5)
    /// Called when modifying an object to point to another object
    pub fn gc_barrier(&mut self, upvalue_ptr: UpvaluePtr, value_gc_ptr: GcObjectPtr) {
        let vm = unsafe { &mut *self.vm };
        let owner_ptr = GcObjectPtr::Upvalue(upvalue_ptr);
        vm.gc.barrier(self, owner_ptr, value_gc_ptr);
    }

    /// Backward GC barrier (luaC_barrierback in Lua 5.5)
    /// Called when modifying a BLACK object (typically table) with new values
    /// Instead of marking the value, re-gray the object for re-traversal
    pub fn gc_barrier_back(&mut self, gc_ptr: GcObjectPtr) {
        let vm = unsafe { &mut *self.vm };
        vm.gc.barrier_back(gc_ptr);
    }

    #[inline(always)]
    pub fn check_gc(&mut self) -> LuaResult<bool> {
        let vm = unsafe { &mut *self.vm };
        let work = vm.check_gc(self);
        Ok(work)
    }

    pub fn collect_garbage(&mut self) -> LuaResult<()> {
        let vm = unsafe { &mut *self.vm };
        vm.full_gc(self, false);
        Ok(())
    }

    pub fn to_string(&mut self, value: &LuaValue) -> LuaResult<String> {
        // Fast path: simple types without metamethods
        match value.kind() {
            LuaValueKind::Binary => {
                if let Some(s) = value.as_binary() {
                    return Ok(format!("<binary: {} bytes>", s.len()));
                }
            }
            LuaValueKind::Nil => return Ok("nil".to_string()),
            LuaValueKind::Boolean => {
                if let Some(b) = value.as_boolean() {
                    return Ok(if b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    });
                }
            }
            LuaValueKind::Integer => {
                if let Some(n) = value.as_integer() {
                    return Ok(n.to_string());
                }
            }
            LuaValueKind::Float => {
                if let Some(n) = value.as_number() {
                    return Ok(n.to_string());
                }
            }
            LuaValueKind::String => {
                if let Some(s) = value.as_str() {
                    return Ok(s.to_string());
                }
            }
            LuaValueKind::Function | LuaValueKind::CFunction => {
                // Functions: use default representation (no metatable support for functions)
                return Ok(format!("{}", value));
            }
            _ => {
                // Check for __tostring metamethod
                if let Some(mm) = get_metamethod_event(self, value, TmKind::ToString) {
                    // Call __tostring metamethod
                    let (succ, results) = self.pcall(mm, vec![value.clone()])?;
                    if !succ {
                        return Err(self.error("error in __tostring metamethod".to_string()));
                    }
                    if let Some(result) = results.get(0) {
                        if let Some(s) = result.as_str() {
                            return Ok(s.to_string());
                        }
                        // __tostring must return a string
                        return Err(self.error("'__tostring' must return a string".to_string()));
                    } else {
                        return Err(
                            self.error("error in __tostring metamethod: no result".to_string())
                        );
                    }
                }
            }
        }

        // Fallback: generic representation
        Ok(format!("{}", value))
    }

    pub fn is_main_thread(&self) -> bool {
        self.is_main
    }
}

impl Default for LuaState {
    fn default() -> Self {
        Self::new(1, std::ptr::null_mut(), false, SafeOption::default())
    }
}
