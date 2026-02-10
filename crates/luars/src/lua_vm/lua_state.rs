// Lua execution state (equivalent to lua_State in Lua C API)
// Represents a single thread/coroutine execution context
// Multiple LuaStates can share the same LuaVM (global_State)

use std::collections::HashMap;
use std::rc::Rc;

use crate::lua_value::{LuaUserdata, LuaValue, LuaValueKind, LuaValuePtr};
use crate::lua_vm::call_info::call_status::{self, CIST_C, CIST_LUA, CIST_RECST, CIST_YPCALL};
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
    pub(crate) error_msg: String,

    /// Error object storage (preserves actual error value for pcall)
    pub(crate) error_object: LuaValue,

    /// Yield values storage (for coroutine yield)
    yield_values: Vec<LuaValue>,

    /// Hook mask and count (for debug hooks)
    _hook_mask: u8,
    _hook_count: i32,

    safe_option: SafeOption,

    is_main: bool,

    /// To-be-closed variable list - stack indices of variables marked with <close>
    /// Maintained in order: most recently added TBC variable is last
    /// When leaving a block (OpCode::Close), we iterate from the end and call __close
    /// on each TBC variable whose stack index >= the close level
    pub(crate) tbc_list: Vec<usize>,
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
            error_object: LuaValue::nil(),
            yield_values: Vec::new(),
            _hook_mask: 0,
            _hook_count: 0,
            safe_option,
            is_main,
            tbc_list: Vec::new(),
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
        func: &LuaValue,
        base: usize,
        nparams: usize,
        nresults: i32,
    ) -> LuaResult<()> {
        // Fast path: check stack depth (branch predictor friendly - usually succeeds)
        if self.call_depth >= self.safe_option.max_call_depth {
            return Err(self.error(format!(
                "stack overflow (Lua stack depth: {})",
                self.call_depth
            )));
        }

        // Cache lua_function extraction (avoid repeated enum matching)
        // This single call replaces multiple is_c_function/as_lua_function checks

        // Determine function type and extract metadata in one pass
        let (call_status, maxstacksize, numparams, nextraargs) =
            if let Some(func_obj) = func.as_lua_function() {
                let chunk = func_obj.chunk();
                // Lua function with chunk
                let numparams = chunk.param_count;
                let nextraargs = if nparams > numparams {
                    (nparams - numparams) as i32
                } else {
                    0
                };
                (
                    CIST_LUA,
                    chunk.max_stack_size as usize,
                    numparams,
                    nextraargs,
                )
            } else if func.is_c_callable() {
                if self.c_call_depth >= self.safe_option.max_call_depth {
                    return Err(self.error(format!(
                        "C stack overflow (C call depth: {})",
                        self.c_call_depth
                    )));
                }
                self.c_call_depth += 1;
                // Light C function
                (CIST_C, nparams, nparams, 0)
            } else {
                // Not callable - this should be prevented by caller
                debug_assert!(false, "push_frame called with non-callable value");
                return Err(self.error(format!("attempt to call a {} value", func.type_name())));
            };

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

        let frame_top = base + maxstacksize;

        // Ensure physical stack has EXTRA_STACK (5) slots above frame_top
        // for metamethod arguments (matching Lua 5.5's EXTRA_STACK guarantee)
        let needed_physical = frame_top + 5;
        if needed_physical > self.stack.len() {
            self.resize(needed_physical)?;
        }

        // Fast path: reuse existing CallInfo slot (most common case)
        if self.call_depth < self.call_stack.len() {
            let ci = unsafe { self.call_stack.get_unchecked_mut(self.call_depth) };
            *ci = CallInfo {
                func: *func,
                base,
                func_offset: 1,
                top: frame_top,
                pc: 0,
                nresults,
                call_status,
                nextraargs,
                saved_nres: 0,
            };
        } else {
            // Slow path: allocate new CallInfo (first time reaching this depth)
            let ci = CallInfo {
                func: *func,
                base,
                func_offset: 1,
                top: frame_top,
                pc: 0,
                nresults,
                call_status,
                nextraargs,
                saved_nres: 0,
            };
            self.call_stack.push(ci);
        }

        self.call_depth += 1;

        // Match Lua 5.5's luaD_precall: L->top.p = ci->top.p
        // For Lua functions, set stack_top to frame_top so the GC's
        // traverse_thread scans the full frame extent. Without this,
        // stack_top stays at the CALL instruction's ra+b, which can be
        // BELOW caller-frame locals that are still live — causing the GC
        // to miss marking those objects.
        if call_status & CIST_LUA != 0 {
            if frame_top > self.stack_top {
                self.stack_top = frame_top;
            }
        }

        Ok(())
    }

    /// Push a Lua function call frame (specialized fast path).
    /// Caller MUST already know `func` is a Lua function and provide the chunk metadata.
    /// Skips the function-type dispatch entirely.
    #[inline]
    pub(crate) fn push_lua_frame(
        &mut self,
        func: &LuaValue,
        base: usize,
        nparams: usize,
        nresults: i32,
        param_count: usize,
        max_stack_size: usize,
    ) -> LuaResult<()> {
        // Check stack depth
        if self.call_depth >= self.safe_option.max_call_depth {
            return Err(self.error(format!(
                "stack overflow (Lua stack depth: {})",
                self.call_depth
            )));
        }

        let nextraargs = if nparams > param_count {
            (nparams - param_count) as i32
        } else {
            0
        };

        // Fill missing parameters with nil
        if nparams < param_count {
            let start = base + nparams;
            let end = base + param_count;
            if self.stack.len() < end {
                self.stack.resize(end, LuaValue::nil());
            } else {
                self.stack[start..end].fill(LuaValue::nil());
            }
            if self.stack_top < end {
                self.stack_top = end;
            }
        }

        let frame_top = base + max_stack_size;
        let needed_physical = frame_top + 5;
        if needed_physical > self.stack.len() {
            self.resize(needed_physical)?;
        }

        // Reuse existing CallInfo slot or allocate
        if self.call_depth < self.call_stack.len() {
            let ci = unsafe { self.call_stack.get_unchecked_mut(self.call_depth) };
            *ci = CallInfo {
                func: *func,
                base,
                func_offset: 1,
                top: frame_top,
                pc: 0,
                nresults,
                call_status: CIST_LUA,
                nextraargs,
                saved_nres: 0,
            };
        } else {
            self.call_stack.push(CallInfo {
                func: *func,
                base,
                func_offset: 1,
                top: frame_top,
                pc: 0,
                nresults,
                call_status: CIST_LUA,
                nextraargs,
                saved_nres: 0,
            });
        }

        self.call_depth += 1;

        // Set stack_top to frame_top for GC safety
        if frame_top > self.stack_top {
            self.stack_top = frame_top;
        }

        Ok(())
    }

    /// Push a C function call frame (specialized fast path).
    /// Caller MUST already know `func` is a C function / cclosure.
    /// Skips the function-type dispatch entirely; mirrors `push_lua_frame`.
    #[inline(always)]
    pub(crate) fn push_c_frame(
        &mut self,
        func: &LuaValue,
        base: usize,
        nargs: usize,
        nresults: i32,
    ) -> LuaResult<()> {
        // Check stack depth
        if self.call_depth >= self.safe_option.max_call_depth {
            return Err(self.error(format!(
                "stack overflow (Lua stack depth: {})",
                self.call_depth
            )));
        }
        if self.c_call_depth >= self.safe_option.max_call_depth {
            return Err(self.error(format!(
                "C stack overflow (C call depth: {})",
                self.c_call_depth
            )));
        }
        self.c_call_depth += 1;

        // For C functions: maxstacksize = nargs, numparams = nargs (no nil filling needed)
        let frame_top = base + nargs;

        // Ensure physical stack has EXTRA_STACK (5) slots above frame_top
        let needed_physical = frame_top + 5;
        if needed_physical > self.stack.len() {
            self.resize(needed_physical)?;
        }

        // Reuse existing CallInfo slot or allocate
        if self.call_depth < self.call_stack.len() {
            let ci = unsafe { self.call_stack.get_unchecked_mut(self.call_depth) };
            *ci = CallInfo {
                func: *func,
                base,
                func_offset: 1,
                top: frame_top,
                pc: 0,
                nresults,
                call_status: CIST_C,
                nextraargs: 0,
                saved_nres: 0,
            };
        } else {
            self.call_stack.push(CallInfo {
                func: *func,
                base,
                func_offset: 1,
                top: frame_top,
                pc: 0,
                nresults,
                call_status: CIST_C,
                nextraargs: 0,
                saved_nres: 0,
            });
        }

        self.call_depth += 1;
        Ok(())
    }

    /// Pop call frame (equivalent to Lua's luaD_poscall)
    #[inline(always)]
    pub(crate) fn pop_frame(&mut self) {
        if self.call_depth > 0 {
            self.call_depth -= 1;
            // Use call_status bit check instead of Option chain
            let ci = unsafe { self.call_stack.get_unchecked(self.call_depth) };
            if ci.call_status & CIST_C != 0 && self.c_call_depth > 0 {
                self.c_call_depth -= 1;
            }
        }
    }

    /// Pop a C call frame (specialized fast path, skips call_status bit check).
    /// Caller MUST know the current frame is a C frame.
    #[inline(always)]
    pub(crate) fn pop_c_frame(&mut self) {
        debug_assert!(self.call_depth > 0);
        self.call_depth -= 1;
        if self.c_call_depth > 0 {
            self.c_call_depth -= 1;
        }
    }

    /// Get logical stack top (L->top.p in Lua source)
    /// This is the first free slot in the stack, NOT the length of physical stack
    #[inline(always)]
    pub fn get_top(&self) -> usize {
        self.stack_top
    }

    /// Set logical stack top (L->top.p = L->stack + new_top in Lua)
    /// This is an internal VM operation — just moves the pointer.
    /// GC safety for stale slots above top is handled by the GC atomic phase
    /// (traverse_thread clears dead stack slices), matching Lua 5.5's design.
    #[inline(always)]
    pub fn set_top(&mut self, new_top: usize) -> LuaResult<()> {
        // Ensure physical stack is large enough
        if new_top > self.stack.len() {
            self.resize(new_top)?;
        }
        self.stack_top = new_top;

        Ok(())
    }

    /// Set logical stack top without any checks (fastest path).
    /// Caller must ensure physical stack is already large enough.
    /// Equivalent to Lua 5.5's `L->top.p = L->stack + new_top`.
    #[inline(always)]
    pub fn set_top_raw(&mut self, new_top: usize) {
        self.stack_top = new_top;
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
        let capacity = self.stack.capacity();
        self.stack.resize(new_size, LuaValue::nil());
        if self.stack.capacity() > capacity {
            // If the vector had to reallocate, we need to update all open upvalue pointers
            self.fix_open_upvalue_pointers();
        }

        Ok(())
    }

    /// Fix all open upvalue cached pointers after a Vec reallocation.
    /// Must be called whenever the stack Vec's internal buffer moves
    /// (e.g., after Vec::push triggers a reallocation).
    pub fn fix_open_upvalue_pointers(&mut self) {
        for upval_ptr in &self.open_upvalues_list {
            let data = &mut upval_ptr.as_mut_ref().data;
            // All entries in open_upvalues_list must be open
            debug_assert!(data.is_open());
            let stack_index = data.get_stack_index();
            if stack_index < self.stack.len() {
                data.update_stack_ptr(
                    (&self.stack[stack_index]) as *const LuaValue as *mut LuaValue,
                );
            }
        }
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
    #[cold]
    #[inline(never)]
    pub fn error(&mut self, msg: String) -> LuaError {
        // Try to get current source location for the error
        let mut location = String::new();
        if let Some(ci) = self.current_frame() {
            if ci.is_lua() {
                if let Some(func_obj) = ci.func.as_lua_function() {
                    let chunk = func_obj.chunk();
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
        };

        self.error_msg = format!("{}{}", location, msg);
        LuaError::RuntimeError
    }

    /// Set error message with preserved error object (for pcall to return)
    #[cold]
    #[inline(never)]
    pub fn error_with_object(&mut self, msg: String, obj: LuaValue) -> LuaError {
        self.error_object = obj;
        self.error(msg)
    }

    /// Clear error state
    #[inline(always)]
    pub fn clear_error(&mut self) {
        self.error_msg.clear();
        self.error_object = LuaValue::nil();
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
                    let chunk = func_obj.chunk();
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
                        result.push_str(&format!("\t{}:{}: in {}\n", source_display, line, what));
                    } else {
                        result.push_str(&format!("\t{}: in {}\n", source_display, what));
                    }
                    continue;
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
            let data = &upval_ptr.as_ref().data;
            // All entries in open_upvalues_list should be open
            if !data.is_open() || data.get_stack_index() < level {
                break;
            }
            count += 1;
        }

        if count > 0 {
            // Batch remove all closed upvalues from the list (efficient O(M) shift via drain)
            let to_close: Vec<UpvaluePtr> = self.open_upvalues_list.drain(0..count).collect();

            // Perform the close operation for each
            for upval_ptr in to_close {
                let data = &upval_ptr.as_ref().data;
                if data.is_open() {
                    let stack_idx = data.get_stack_index();

                    // Remove from map (maintain consistency)
                    self.open_upvalues_map.remove(&stack_idx);

                    // Capture value from stack
                    let value = self
                        .stack
                        .get(stack_idx)
                        .copied()
                        .unwrap_or(LuaValue::nil());

                    // Close the upvalue (move value to heap)
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

    /// Get the name of a local variable at the given stack index
    /// by looking at the current frame's locvars debug info
    fn get_local_var_name(&self, stack_index: usize) -> Option<String> {
        let ci = self.current_frame()?;
        if !ci.is_lua() {
            return None;
        }
        let func_obj = ci.func.as_lua_function()?;
        let chunk = func_obj.chunk();
        let reg = stack_index.checked_sub(ci.base)?;
        // Use ci.pc (next instruction) as the PC for lookup,
        // because at TBC instruction, the variable's startpc equals the TBC PC
        // and ci.pc has already been incremented past it
        let pc = ci.pc as usize;
        // Walk locvars to find which variable occupies register 'reg' at 'pc'
        let mut n = 0usize;
        for locvar in &chunk.locals {
            if (locvar.startpc as usize) > pc {
                break;
            }
            if pc < locvar.endpc as usize {
                if n == reg {
                    return Some(locvar.name.clone());
                }
                n += 1;
            }
        }
        None
    }

    /// Mark a stack slot as to-be-closed (TBC)
    /// Called by OpCode::Tbc
    /// If the value is nil or false, it doesn't need to be closed
    /// Otherwise, it must have a __close metamethod
    pub fn mark_tbc(&mut self, stack_index: usize) -> LuaResult<()> {
        let value = self
            .stack
            .get(stack_index)
            .copied()
            .unwrap_or(LuaValue::nil());

        // nil and false don't need to be closed
        if value.is_falsy() {
            return Ok(());
        }

        // Check that the value has a __close metamethod
        use crate::lua_vm::execute::TmKind;
        use crate::lua_vm::execute::get_metamethod_event;
        let has_close = get_metamethod_event(self, &value, TmKind::Close).is_some();

        if !has_close {
            // Try to get the variable name from locvars
            let var_name = self.get_local_var_name(stack_index);
            let msg = if let Some(name) = var_name {
                format!("variable '{}' got a non-closable value", name)
            } else {
                "variable got a non-closable value".to_string()
            };
            return Err(self.error(msg));
        }

        self.tbc_list.push(stack_index);
        Ok(())
    }

    /// Close all to-be-closed variables down to (and including) the given level
    /// This calls __close(obj) on each TBC variable in reverse order
    /// For normal block exit (LUA_OK status), only 1 argument is passed
    /// If a __close method throws, subsequent closes get the error as 2nd arg
    /// (cascading error behavior from Lua 5.5)
    /// If a __close method yields, we propagate the yield immediately.
    /// The current TBC was already popped from tbc_list, so on resume
    /// close_tbc can be called again to continue with remaining entries.
    pub fn close_tbc(&mut self, level: usize) -> LuaResult<()> {
        let mut current_error: Option<LuaValue> = None;

        while let Some(&tbc_idx) = self.tbc_list.last() {
            if tbc_idx < level {
                break;
            }
            self.tbc_list.pop();

            let value = self.stack.get(tbc_idx).copied().unwrap_or(LuaValue::nil());

            // Skip nil/false (shouldn't be in the list, but be safe)
            if value.is_falsy() {
                continue;
            }

            let close_method = get_metamethod_event(self, &value, TmKind::Close);

            if let Some(close_fn) = close_method {
                let result = if let Some(ref err) = current_error {
                    // Previous __close threw — pass error as 2nd arg
                    self.call_close_method_with_error(&close_fn, &value, err.clone())
                } else {
                    // Normal close — 1 argument only
                    self.call_close_method_normal(&close_fn, &value)
                };

                match result {
                    Ok(()) => {}
                    Err(LuaError::Yield) => {
                        // Close method yielded — propagate yield immediately.
                        // The TBC entry was already popped, so on resume
                        // close_tbc can continue with remaining entries.
                        return Err(LuaError::Yield);
                    }
                    Err(_) => {
                        // This __close threw an error — capture it as current error
                        let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                        if !err_obj.is_nil() {
                            current_error = Some(err_obj);
                        } else {
                            let msg = self.error_msg.clone();
                            if let Ok(s) = self.create_string(&msg) {
                                current_error = Some(s.into());
                            }
                        }
                        self.clear_error();
                    }
                }
            } else {
                // No __close metamethod on a non-nil/non-false TBC value
                // This is an error (metamethod was removed after marking)
                let var_name = self.get_local_var_name(tbc_idx);
                let msg = if let Some(name) = var_name {
                    format!(
                        "attempt to close non-closable variable '{}' (no metamethod 'close')",
                        name
                    )
                } else {
                    "attempt to close variable (no metamethod 'close')".to_string()
                };
                if let Ok(s) = self.create_string(&msg) {
                    current_error = Some(s.into());
                }
            }
        }

        // If any __close threw, propagate the last error
        if let Some(err) = current_error {
            self.error_object = err.clone();
            let msg = if let Some(s) = err.as_str() {
                s.to_string()
            } else {
                format!("{:?}", err)
            };
            return Err(self.error(msg));
        }

        Ok(())
    }

    /// Close all upvalues AND to-be-closed variables down to the given level
    /// This is the main "close" operation used by OpCode::Close and return handlers
    pub fn close_all(&mut self, level: usize) -> LuaResult<()> {
        // First close upvalues (captures values from stack)
        self.close_upvalues(level);
        // Then call __close on TBC variables (in reverse order)
        self.close_tbc(level)
    }

    /// Close all to-be-closed variables with error status
    /// Used when unwinding due to errors  
    /// Calls __close(obj, err) — 2 arguments, with cascading error handling
    /// Yield propagation works the same as close_tbc.
    pub fn close_tbc_with_error(&mut self, level: usize, err: LuaValue) -> LuaResult<()> {
        use crate::lua_vm::execute::TmKind;
        use crate::lua_vm::execute::get_metamethod_event;

        let mut current_error = err;
        let mut had_close_error = false;

        while let Some(&tbc_idx) = self.tbc_list.last() {
            if tbc_idx < level {
                break;
            }
            self.tbc_list.pop();

            let value = self.stack.get(tbc_idx).copied().unwrap_or(LuaValue::nil());

            if value.is_falsy() {
                continue;
            }

            let close_method = get_metamethod_event(self, &value, TmKind::Close);

            if let Some(close_fn) = close_method {
                // Call __close(obj, err) with 2 arguments
                let result =
                    self.call_close_method_with_error(&close_fn, &value, current_error.clone());

                match result {
                    Ok(()) => {}
                    Err(LuaError::Yield) => {
                        // Close method yielded — propagate yield
                        return Err(LuaError::Yield);
                    }
                    Err(_) => {
                        // This __close threw — capture as new current error
                        had_close_error = true;
                        let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                        if !err_obj.is_nil() {
                            current_error = err_obj;
                        } else {
                            let msg = self.error_msg.clone();
                            if let Ok(s) = self.create_string(&msg) {
                                current_error = s.into();
                            }
                        }
                        self.clear_error();
                    }
                }
            } else {
                // No __close metamethod — treat as error
                had_close_error = true;
                let var_name = self.get_local_var_name(tbc_idx);
                let msg = if let Some(name) = var_name {
                    format!(
                        "attempt to close non-closable variable '{}' (no metamethod 'close')",
                        name
                    )
                } else {
                    "attempt to close variable (no metamethod 'close')".to_string()
                };
                if let Ok(s) = self.create_string(&msg) {
                    current_error = s.into();
                }
            }
        }

        // Store the final cascaded error in error_object so pcall/xpcall can retrieve it
        if had_close_error {
            self.error_object = current_error;
        }

        Ok(())
    }

    /// Call __close(obj) for normal block exit — 1 argument only
    /// Lua 5.5: normal close passes errobj=NULL, so callclosemethod only pushes self
    fn call_close_method_normal(&mut self, close_fn: &LuaValue, obj: &LuaValue) -> LuaResult<()> {
        use crate::lua_vm::execute::{call, lua_execute};

        let caller_depth = self.call_depth();

        // Use current top directly (like Lua 5.5's callclosemethod)
        // Do NOT restore ci->top here — during pcall cleanup, frames may already
        // be popped and ci->top would be wrong
        let func_pos = self.get_top();

        // Ensure stack has space
        if func_pos + 2 >= self.stack.len() {
            self.grow_stack(func_pos + 3)?;
        }

        {
            let stack = self.stack_mut();
            stack[func_pos] = *close_fn; // function
            stack[func_pos + 1] = *obj; // self (1st argument)
        }
        self.set_top_raw(func_pos + 2); // 2 values: function + 1 arg

        let result = if close_fn.is_c_callable() {
            call::call_c_function(self, func_pos, 1, 0)
        } else if close_fn.is_lua_function() {
            let new_base = func_pos + 1;
            self.push_frame(close_fn, new_base, 1, 0)?;
            lua_execute(self, caller_depth)
        } else {
            // Non-callable close method (e.g., a number)
            let type_name = close_fn.type_name();
            Err(self.error(format!(
                "attempt to call a {} value (metamethod 'close')",
                type_name
            )))
        };

        match &result {
            Err(LuaError::Yield) => {
                // Yield: do NOT pop frames — they stay for resume
            }
            Err(_) => {
                // Error: pop any frames pushed by the close method
                while self.call_depth() > caller_depth {
                    self.pop_frame();
                }
            }
            Ok(()) => {}
        }

        result
    }

    /// Call __close(obj, err) for error unwinding — 2 arguments
    fn call_close_method_with_error(
        &mut self,
        close_fn: &LuaValue,
        obj: &LuaValue,
        err: LuaValue,
    ) -> LuaResult<()> {
        use crate::lua_vm::execute::{call, lua_execute};

        let caller_depth = self.call_depth();

        // Like Lua 5.5's callclosemethod: use current top directly, don't restore ci->top
        // (after frame pops, ci->top may be lower than TBC variables on the stack)
        let func_pos = self.get_top();
        // Ensure stack has room for function + 2 args
        if func_pos + 3 > self.stack().len() {
            self.grow_stack(3)?;
        }
        {
            let stack = self.stack_mut();
            stack[func_pos] = *close_fn; // function
            stack[func_pos + 1] = *obj; // self (1st argument)
            stack[func_pos + 2] = err; // error (2nd argument)
        }
        self.set_top_raw(func_pos + 3); // 3 values: function + 2 args

        let result = if close_fn.is_c_callable() {
            call::call_c_function(self, func_pos, 2, 0)
        } else if close_fn.is_lua_function() {
            let new_base = func_pos + 1;
            self.push_frame(close_fn, new_base, 2, 0)?;
            lua_execute(self, caller_depth)
        } else {
            // Non-callable close method (e.g., a number)
            let type_name = close_fn.type_name();
            Err(self.error(format!(
                "attempt to call a {} value (metamethod 'close')",
                type_name
            )))
        };

        match &result {
            Err(LuaError::Yield) => {
                // Yield: do NOT pop frames — they stay for resume
            }
            Err(_) => {
                // Error: pop any frames pushed by the close method
                while self.call_depth() > caller_depth {
                    self.pop_frame();
                }
            }
            Ok(()) => {}
        }

        result
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
                .position(|&ptr| ptr.as_ref().data.get_stack_index() < stack_index)
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
                    let stack_idx = upval.get_stack_index();
                    if stack_idx >= new_len {
                        // Invalidate upvalue pointing to truncated stack
                        upval.close(self.stack[stack_idx]);
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
        unsafe { self.call_stack.get_unchecked(frame_idx).base }
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
        unsafe { self.call_stack.get_unchecked_mut(frame_idx).pc = pc };
    }

    /// Set frame top by index (for tail calls)
    #[inline(always)]
    pub fn set_frame_top(&mut self, frame_idx: usize, top: usize) {
        if let Some(frame) = self.call_stack.get_mut(frame_idx) {
            frame.top = top;
        }
    }

    /// Set frame function by index (for tail calls)
    #[inline]
    pub fn set_frame_func(&mut self, frame_idx: usize, func: LuaValue) {
        debug_assert!(func.is_function(), "Frame func must be callable");

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

    /// Get current CallInfo by index (unchecked — caller must ensure idx < call_depth)
    #[inline(always)]
    pub fn get_call_info(&self, idx: usize) -> &CallInfo {
        debug_assert!(idx < self.call_stack.len());
        unsafe { self.call_stack.get_unchecked(idx) }
    }

    /// Get mutable CallInfo by index (unchecked — caller must ensure idx < call_depth)
    #[inline(always)]
    pub fn get_call_info_mut(&mut self, idx: usize) -> &mut CallInfo {
        debug_assert!(idx < self.call_stack.len());
        unsafe { self.call_stack.get_unchecked_mut(idx) }
    }

    /// Pop the current call frame (Lua callers only — does NOT adjust c_call_depth)
    #[inline(always)]
    pub fn pop_call_frame(&mut self) {
        debug_assert!(self.call_depth > 0);
        self.call_depth -= 1;
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

    /// Get a specific argument without bounds checking (1-based index).
    /// SAFETY: Caller MUST ensure index >= 1, call_depth > 0,
    /// and `base + index - 1 < stack.len()`.
    #[inline(always)]
    pub unsafe fn get_arg_unchecked(&self, index: usize) -> LuaValue {
        unsafe {
            let frame = self.call_stack.get_unchecked(self.call_depth - 1);
            *self.stack.get_unchecked(frame.base + index - 1)
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
        // Do NOT modify frame.top (ci->top) — it's immutable after push_frame.
        self.stack_top = current_top + 1;

        Ok(())
    }

    /// Push a value to the stack without capacity/overflow checking.
    /// SAFETY: Caller MUST ensure physical stack has room (guaranteed by EXTRA_STACK
    /// after push_c_frame) and stack_top < max_stack_size.
    #[inline(always)]
    pub unsafe fn push_value_unchecked(&mut self, value: LuaValue) {
        unsafe {
            let top = self.stack_top;
            *self.stack.get_unchecked_mut(top) = value;
            self.stack_top = top + 1;
        }
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

    /// Get value from table (raw, no metamethods)
    pub fn raw_get(&mut self, table: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        self.vm_mut().raw_get(table, key)
    }

    /// Get value from table with __index metamethod support
    pub fn table_get(&mut self, table: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        // First try raw access
        if let Some(val) = self.vm_mut().raw_get(table, key) {
            return Some(val);
        }
        // If not found, try __index metamethod
        execute::helper::lookup_from_metatable(self, table, key)
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
        let saved_stack_top = self.stack_top;
        let func_idx = self.stack.len();

        // Push function and args to stack
        // Track capacity to detect Vec reallocation.
        let old_capacity = self.stack.capacity();
        self.stack.push(func);
        for arg in args {
            self.stack.push(arg);
        }
        // If Vec reallocated, fix all open upvalue cached pointers.
        // This is critical: open upvalues cache raw pointers into the Vec buffer.
        // Vec::push can reallocate the buffer, leaving those pointers dangling.
        if self.stack.capacity() != old_capacity {
            self.fix_open_upvalue_pointers();
        }
        let arg_count = self.stack.len() - func_idx - 1;

        // Sync logical stack top with physical stack after Vec::push
        // This is critical: push_value writes to stack_top, so it must
        // be consistent with the actual stack contents.
        self.stack_top = self.stack.len();

        // Resolve __call chain if needed

        let (actual_arg_count, ccmt_depth) = match resolve_call_chain(self, func_idx, arg_count) {
            Ok((count, depth)) => (count, depth),
            Err(e) => {
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                self.stack.truncate(func_idx);
                self.stack_top = saved_stack_top;
                return Ok((false, vec![err_str]));
            }
        };

        // Get resolved function
        let func = self
            .stack_get(func_idx)
            .ok_or_else(|| self.error("pcall: function not found".to_string()))?;

        // Check if it's a C function
        let is_c_callable = func.is_c_callable();
        if is_c_callable {
            // Create frame for C function
            let base = func_idx + 1;
            if let Err(e) = self.push_frame(&func, base, actual_arg_count, -1) {
                self.stack.truncate(func_idx);
                self.stack_top = saved_stack_top;
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                return Ok((false, vec![err_str]));
            }

            // Set ccmt count in call_status
            if ccmt_depth > 0 {
                let frame_idx = self.call_depth - 1;
                if let Some(frame) = self.call_stack.get_mut(frame_idx) {
                    frame.call_status = call_status::set_ccmt_count(frame.call_status, ccmt_depth);
                }
            }

            // Get the C function pointer
            let cfunc = if let Some(c_func) = func.as_cfunction() {
                c_func
            } else if let Some(closure) = func.as_cclosure() {
                closure.func()
            } else {
                unreachable!()
            };

            // Call C function
            let result = cfunc(self);

            // Pop frame
            self.pop_frame();

            match result {
                Ok(nresults) => {
                    // Success - collect results from stack_top (where push_value writes)
                    let mut results = Vec::new();
                    let result_start = if self.stack_top >= nresults {
                        self.stack_top - nresults
                    } else {
                        0
                    };

                    for i in result_start..self.stack_top {
                        if let Some(val) = self.stack_get(i) {
                            results.push(val);
                        }
                    }

                    // Clean up stack
                    self.stack.truncate(func_idx);
                    self.stack_top = saved_stack_top;

                    Ok((true, results))
                }
                Err(LuaError::Yield) => Err(LuaError::Yield),
                Err(e) => {
                    let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                    let result_err = if !err_obj.is_nil() {
                        err_obj
                    } else {
                        let error_msg = self.get_error_msg(e);
                        self.create_string(&error_msg)?.into()
                    };
                    self.stack.truncate(func_idx);
                    self.stack_top = saved_stack_top;
                    Ok((false, vec![result_err]))
                }
            }
        } else {
            // Lua function - use lua_execute
            let base = func_idx + 1;
            // pcall expects all return values
            if let Err(e) = self.push_frame(&func, base, actual_arg_count, -1) {
                self.stack.truncate(func_idx);
                self.stack_top = saved_stack_top;
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                return Ok((false, vec![err_str]));
            }

            // Execute via lua_execute_until - only execute the new frame
            let result = execute::lua_execute(self, initial_depth);

            match result {
                Ok(()) => {
                    // Success - collect return values from stack
                    // Use stack_top (not stack.len()) because RETURN opcodes
                    // set stack_top to reflect actual return values
                    let mut results = Vec::new();
                    for i in func_idx..self.stack_top {
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
                    self.stack_top = saved_stack_top;

                    Ok((true, results))
                }
                Err(LuaError::Yield) => Err(LuaError::Yield),
                Err(e) => {
                    // Error occurred - clean up
                    // Lua 5.5 order: L->ci = old_ci first, then closeprotected
                    // This ensures debug.getinfo(2) inside __close sees pcall's caller

                    // Get error object BEFORE closing TBC (close may modify it)
                    let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                    let error_msg_str = self.get_error_msg(e);

                    // Get frame_base before popping frames
                    let frame_base = if self.call_depth() > initial_depth {
                        self.call_stack.get(initial_depth).map(|f| f.base)
                    } else {
                        None
                    };

                    // Pop frames FIRST (like Lua 5.5: L->ci = old_ci)
                    while self.call_depth() > initial_depth {
                        self.pop_frame();
                    }

                    // Then close upvalues and TBC variables
                    if let Some(base) = frame_base {
                        self.close_upvalues(base);
                        // Pass error to TBC close methods
                        // close_tbc_with_error may update error_object if __close cascades
                        let _ = self.close_tbc_with_error(base, err_obj);
                    }

                    // Check if close_tbc_with_error updated error_object (from cascading __close errors)
                    let cascaded_err = std::mem::replace(&mut self.error_object, LuaValue::nil());
                    let result_err = if !cascaded_err.is_nil() {
                        cascaded_err
                    } else if !err_obj.is_nil() {
                        err_obj
                    } else {
                        self.create_string(&error_msg_str)?.into()
                    };

                    // Clean up stack
                    self.stack.truncate(func_idx);
                    self.stack_top = saved_stack_top;

                    Ok((false, vec![result_err]))
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
        let result = if func.is_c_callable() {
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
            self.push_frame(&func, base, actual_arg_count, -1)?;

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
            Err(LuaError::Yield) => {
                // Mark pcall's own C frame with CIST_YPCALL so that
                // finish_c_frame knows to wrap results with true on resume.
                // pcall's C frame is at initial_depth - 1.
                if initial_depth > 0 {
                    use crate::lua_vm::call_info::call_status::CIST_YPCALL;
                    let pcall_frame_idx = initial_depth - 1;
                    if pcall_frame_idx < self.call_depth {
                        let ci = self.get_call_info_mut(pcall_frame_idx);
                        ci.call_status |= CIST_YPCALL;
                    }
                }
                Err(LuaError::Yield)
            }
            Err(e) => {
                // Error - clean up and return error
                // Lua 5.5 order: pop frames first, then close TBC

                // Get error object BEFORE closing TBC
                let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                let error_msg_str = self.get_error_msg(e);

                // Get frame_base before popping frames
                let frame_base = if self.call_depth() > initial_depth {
                    self.call_stack.get(initial_depth).map(|f| f.base)
                } else {
                    None
                };

                // Pop frames FIRST (like Lua 5.5: L->ci = old_ci)
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                // Then close upvalues and TBC variables
                if let Some(base) = frame_base {
                    self.close_upvalues(base);
                    let close_result = self.close_tbc_with_error(base, err_obj.clone());
                    match close_result {
                        Ok(()) => {} // continue to set up error result below
                        Err(LuaError::Yield) => {
                            // TBC close yielded during error recovery.
                            // Save state: mark pcall's C frame with CIST_YPCALL + CIST_RECST
                            // so finish_c_frame will handle error result on resume.
                            let pcall_ci_idx = initial_depth - 1;
                            if pcall_ci_idx < self.call_depth {
                                let ci = self.get_call_info_mut(pcall_ci_idx);
                                ci.call_status |= CIST_YPCALL | CIST_RECST;
                            }
                            // Save error value (may have cascaded) for finish_c_frame
                            let cascaded =
                                std::mem::replace(&mut self.error_object, LuaValue::nil());
                            self.error_object = if !cascaded.is_nil() {
                                cascaded
                            } else {
                                err_obj
                            };
                            return Err(LuaError::Yield);
                        }
                        Err(_e2) => {
                            // TBC close threw — use the new error
                            // Fall through to set up error result below
                        }
                    }
                }

                // Check if close_tbc_with_error updated error_object (cascading)
                let cascaded_err = std::mem::replace(&mut self.error_object, LuaValue::nil());
                let result_err = if !cascaded_err.is_nil() {
                    cascaded_err
                } else if !err_obj.is_nil() {
                    err_obj
                } else {
                    self.create_string(&error_msg_str)?.into()
                };

                // Set error at func_idx and update stack top
                self.stack_set(func_idx, result_err)?;
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
        if let Err(e) = self.push_frame(&func, base, actual_arg_count, -1) {
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

                // Get error object BEFORE closing TBC
                let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                let error_msg_str = self.get_error_msg(e);

                // Get frame_base before popping frames
                let frame_base = if self.call_depth() > initial_depth {
                    self.call_stack.get(initial_depth).map(|f| f.base)
                } else {
                    None
                };

                // Pop frames FIRST (like Lua 5.5: L->ci = old_ci)
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                // Then close upvalues and TBC
                if let Some(base) = frame_base {
                    self.close_upvalues(base);
                    let _ = self.close_tbc_with_error(base, err_obj.clone());
                }

                // Check if close_tbc_with_error updated error_object (cascading)
                let cascaded_err = std::mem::replace(&mut self.error_object, LuaValue::nil());

                // Set up error handler call
                // Reset stack to [handler]
                self.set_top(handler_idx + 1)?;

                // Push error value as argument — use cascaded error if available
                let err_value = if !cascaded_err.is_nil() {
                    cascaded_err
                } else if !err_obj.is_nil() {
                    err_obj
                } else {
                    self.create_string(&error_msg_str)?.into()
                };
                self.push_value(err_value.clone())?;

                // Get handler and create frame
                let handler = self.stack_get(handler_idx).unwrap_or(LuaValue::nil());
                let handler_base = handler_idx + 1;

                if let Err(_) = self.push_frame(&handler, handler_base, 1, -1) {
                    // Error handler setup failed
                    self.set_top(handler_idx)?;
                    let err_desc = if let Some(s) = err_value.as_str() {
                        s.to_string()
                    } else {
                        format!("{:?}", err_value)
                    };
                    let final_err =
                        self.create_string(&format!("error in error handling: {}", err_desc))?;
                    return Ok((false, vec![final_err]));
                }

                // Execute error handler - distinguish C vs Lua handler
                let handler_result = if handler.is_cfunction() || handler.as_cclosure().is_some() {
                    // C function handler (e.g., debug.traceback)
                    // pop the Lua frame we just pushed (push_frame pushes Lua frame)
                    // and use call_c_function instead
                    while self.call_depth() > initial_depth {
                        self.pop_frame();
                    }
                    // Set up stack: [handler, err_value]
                    self.set_top(handler_idx + 1)?;
                    self.push_value(err_value.clone())?;
                    execute::call::call_c_function(self, handler_idx, 1, -1)
                } else {
                    execute::lua_execute(self, initial_depth)
                };

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
                            results.push(self.create_string(&error_msg_str)?);
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
                        let final_err = self.create_string(&format!(
                            "error in error handling: {}",
                            error_msg_str
                        ))?;
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
            let old_capacity = self.stack.capacity();
            for arg in args {
                self.stack.push(arg);
            }
            // Fix open upvalue pointers if Vec was reallocated
            if self.stack.capacity() != old_capacity {
                self.fix_open_upvalue_pointers();
            }

            // Create initial frame, expecting all return values
            let nargs = self.stack.len() - 1; // -1 for function itself
            let base = 1; // Arguments start at index 1 (function is at 0)
            self.push_frame(&func, base, nargs, -1)?;

            // Execute until yield or completion
            let result = if func.is_c_callable() {
                // Call C function directly
                execute::call::call_c_function(self, 0, nargs, -1)
            } else {
                // Execute Lua bytecode
                execute::lua_execute(self, 0)
            };

            self.handle_resume_result(result)
        } else {
            // Resuming after yield
            // The yield function's frame is still on the stack, we need to:
            // 1. Pop the yield frame
            // 2. Place resume arguments as yield's return values
            // 3. Continue execution from the caller's frame
            // NOTE: Do NOT close upvalues or TBC variables on yield resume!
            // Yield is not an exit — variables are still alive.

            // Get the yield frame info before popping
            let func_idx = if let Some(frame) = self.current_frame() {
                // func_idx is base - func_offset (where the yield function was called)
                let func_idx = frame.base - frame.func_offset;
                func_idx
            } else {
                return Err(self.error("cannot resume: no frame".to_string()));
            };

            // Pop the yield frame
            self.pop_frame();

            // Place resume arguments at func_idx as yield's return values
            // This simulates the yield function returning normally
            let actual_nresults = args.len();
            for (i, arg) in args.into_iter().enumerate() {
                self.stack_set(func_idx + i, arg)?;
            }

            // Update stack top only (do NOT modify caller frame's top)
            let new_top = func_idx + actual_nresults;
            self.set_top_raw(new_top);

            // Execute until yield or completion
            let result = execute::lua_execute(self, 0);

            // Handle result with pcall error recovery (precover)
            self.handle_resume_result(result)
        }
    }

    /// Handle the result of lua_execute during resume.
    /// Implements Lua 5.5's precover: when an error occurs, search the
    /// call stack for a pcall frame (CIST_YPCALL), recover there, and
    /// continue execution.
    fn handle_resume_result(
        &mut self,
        initial_result: LuaResult<()>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        let mut result = initial_result;

        loop {
            match result {
                Ok(()) => {
                    // Coroutine completed
                    let results = self.get_all_return_values(0);
                    self.stack.clear();
                    return Ok((true, results));
                }
                Err(LuaError::Yield) => {
                    // Coroutine yielded
                    let yield_vals = self.take_yield();
                    return Ok((false, yield_vals));
                }
                Err(_e) => {
                    // Error — try to find a pcall frame to recover
                    let pcall_idx = self.find_pcall_recovery_frame();
                    if pcall_idx.is_none() {
                        // No recovery point — coroutine dies.
                        // Close all TBC variables before dying
                        // (equivalent to Lua 5.5's luaF_close in lua_resume).
                        let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                        let error_val = if !err_obj.is_nil() {
                            err_obj
                        } else {
                            LuaValue::nil()
                        };

                        // Pop all frames
                        while self.call_depth() > 0 {
                            self.pop_frame();
                        }

                        // Close all upvalues and TBC variables from level 0
                        self.close_upvalues(0);
                        let _ = self.close_tbc_with_error(0, error_val.clone());

                        // Restore error state: if close cascaded, error_object
                        // is already set by close_tbc_with_error. If not, restore
                        // the original error value and msg.
                        if self.error_object.is_nil() {
                            self.error_object = error_val;
                        } else {
                            // Cascaded error — update error_msg to match
                            self.error_msg = format!("{}", self.error_object);
                        }

                        return Err(_e);
                    }
                    let pcall_frame_idx = pcall_idx.unwrap();

                    // Get pcall's info before cleanup
                    let pcall_ci = self.get_call_info(pcall_frame_idx);
                    let pcall_func_pos = pcall_ci.base - pcall_ci.func_offset;
                    let pcall_nresults = pcall_ci.nresults;
                    let close_level = pcall_ci.base; // close from body position

                    // Get error object
                    let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                    let error_val = if !err_obj.is_nil() {
                        err_obj
                    } else {
                        let msg = self.error_msg.clone();
                        self.create_string(&msg)
                            .map(|s| s.into())
                            .unwrap_or(LuaValue::nil())
                    };
                    self.clear_error();

                    // Pop frames down to pcall (exclusive — keep pcall's frame temporarily)
                    while self.call_depth() > pcall_frame_idx + 1 {
                        self.pop_frame();
                    }

                    // Close upvalues
                    self.close_upvalues(close_level);

                    // Close TBC with error (may yield or throw again)
                    let close_result = self.close_tbc_with_error(close_level, error_val.clone());

                    match close_result {
                        Ok(()) => {
                            // Get the final error (might be cascaded from TBC closes)
                            let final_err =
                                std::mem::replace(&mut self.error_object, LuaValue::nil());
                            let result_err = if !final_err.is_nil() {
                                final_err
                            } else {
                                error_val
                            };
                            self.clear_error();

                            // Set up pcall error result: (false, error)
                            self.stack_set(pcall_func_pos, LuaValue::boolean(false))
                                .ok();
                            self.stack_set(pcall_func_pos + 1, result_err).ok();
                            let n = 2;

                            // Pop pcall frame
                            self.pop_frame();

                            // Handle nresults like call_c_function post-processing
                            let final_n = if pcall_nresults == -1 {
                                n
                            } else {
                                pcall_nresults as usize
                            };
                            let new_top = pcall_func_pos + final_n;
                            if pcall_nresults >= 0 {
                                let wanted = pcall_nresults as usize;
                                for i in n..wanted {
                                    self.stack_set(pcall_func_pos + i, LuaValue::nil()).ok();
                                }
                            }
                            self.set_top_raw(new_top);

                            // Restore caller frame top
                            if self.call_depth() > 0 {
                                let ci_idx = self.call_depth() - 1;
                                if pcall_nresults == -1 {
                                    let ci_top = self.get_call_info(ci_idx).top;
                                    if ci_top < new_top {
                                        self.get_call_info_mut(ci_idx).top = new_top;
                                    }
                                } else {
                                    let frame_top = self.get_call_info(ci_idx).top;
                                    self.set_top_raw(frame_top);
                                }
                            }

                            // Continue execution
                            result = execute::lua_execute(self, 0);
                            // Loop again to check for more errors/yields
                        }
                        Err(LuaError::Yield) => {
                            // TBC close yielded during error recovery.
                            // Save recovery state: mark pcall frame with CIST_RECST
                            // and store the error value in error_object.
                            // When the close method finishes, finish_c_frame will
                            // detect CIST_RECST and set up (false, error) result.
                            use crate::lua_vm::call_info::call_status::CIST_RECST;
                            if pcall_frame_idx < self.call_depth() {
                                let ci = self.get_call_info_mut(pcall_frame_idx);
                                ci.call_status |= CIST_RECST;
                            }
                            // Store the error value for finish_c_frame to retrieve later
                            self.error_object = error_val;

                            // Return yield values normally
                            let yield_vals = self.take_yield();
                            return Ok((false, yield_vals));
                        }
                        Err(_e2) => {
                            // TBC close threw again — try to recover with updated error
                            result = Err(_e2);
                            // Loop continues to find next pcall frame
                        }
                    }
                }
            }
        }
    }

    /// Find a pcall C frame (CIST_YPCALL) on the call stack for error recovery.
    /// Returns the frame index if found.
    fn find_pcall_recovery_frame(&self) -> Option<usize> {
        for i in (0..self.call_depth()).rev() {
            let ci = self.get_call_info(i);
            if ci.call_status & CIST_YPCALL != 0 {
                return Some(i);
            }
        }
        None
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
    #[inline(always)]
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

    #[inline]
    pub fn check_gc(&mut self) -> LuaResult<bool> {
        let vm = unsafe { &mut *self.vm };
        if vm.gc.gc_debt > 0 {
            return Ok(false);
        }

        // Run GC step with the current top. Do NOT raise top to ci_top.
        //
        // In C Lua 5.5, the checkGC macro works with whatever top is set by
        // the caller. Opcodes like OP_NEWTABLE temporarily LOWER top to ra+1
        // before checkGC, which intentionally excludes stale registers from
        // the GC's stack scan. Raising top to ci_top would scan stale
        // registers that may hold dead values, keeping them alive and
        // breaking weak table clearing.
        //
        // traverse_thread in the atomic phase clears slots [top..stack_last]
        // to nil, which handles any stale references above top.
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
