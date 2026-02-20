// Lua execution state (equivalent to lua_State in Lua C API)
// Represents a single thread/coroutine execution context
// Multiple LuaStates can share the same LuaVM (global_State)

use std::rc::Rc;

use crate::lua_value::{LuaUserdata, LuaValue, LuaValueKind, LuaValuePtr, UpvalueStore};
use crate::lua_vm::call_info::call_status::{
    self, CIST_C, CIST_LUA, CIST_RECST, CIST_XPCALL, CIST_YPCALL,
};
use crate::lua_vm::execute::call::{call_c_function, resolve_call_chain};
use crate::lua_vm::execute::{self, lua_execute};
use crate::lua_vm::safe_option::SafeOption;
use crate::lua_vm::{CallInfo, LuaError, LuaResult, TmKind, get_metamethod_event};
use crate::{
    Chunk, CreateResult, GcObjectPtr, LuaRegistrable, LuaVM, StringPtr, ThreadPtr, UpvaluePtr,
};

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

    /// Open upvalues - upvalues pointing to stack locations
    /// Sorted Vec (higher stack indices first) for efficient lookup and close traversal.
    /// Linear scan is faster than HashMap for typical 0-5 open upvalues due to
    /// no hashing overhead and better cache locality. Matches C Lua's sorted list design.
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

    /// Whether this coroutine has yielded and is waiting to be resumed.
    /// Used to distinguish "running" from "yielded" when call_stack is non-empty.
    /// Set to true when yield is captured by resume, false when execution resumes.
    yielded: bool,

    /// Whether close_tbc_with_error is currently running on this thread.
    /// Used to detect re-entrant coroutine.close() calls from __close handlers.
    pub(crate) is_closing: bool,

    /// Non-yieldable nesting depth (like C Lua's nny packed in nCcalls).
    /// Main thread starts at 1 (always non-yieldable).
    /// Coroutine threads start at 0 (yieldable).
    /// Incremented when entering a non-yieldable C call boundary (e.g., pcall method
    /// used by C stdlib functions like gsub that don't support continuations).
    /// `yieldable(L)` == `nny == 0`.
    pub(crate) nny: u32,
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
            open_upvalues_list: Vec::new(),
            error_msg: String::new(),
            error_object: LuaValue::nil(),
            yield_values: Vec::new(),
            _hook_mask: 0,
            _hook_count: 0,
            safe_option,
            is_main,
            tbc_list: Vec::new(),
            yielded: false,
            is_closing: false,
            nny: if is_main { 1 } else { 0 },
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
            if self.safe_option.max_call_depth > self.safe_option.base_call_depth {
                // In error handler extra zone - C Lua's stackerror behavior
                return Err(LuaError::ErrorInErrorHandling);
            }
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

            // Ensure stack capacity.
            // CRITICAL: Use self.resize() instead of self.stack.resize() so that
            // open upvalue raw pointers are fixed if the Vec reallocates.
            // Direct self.stack.resize() bypasses fix_open_upvalue_pointers(),
            // causing use-after-free when upvalues read/write via stale pointers.
            if self.stack.len() < end {
                self.resize(end)?;
            }
            // Batch fill missing parameter slots with nil
            self.stack[start..end].fill(LuaValue::nil());

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
                pending_finish_get: -1,
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
                pending_finish_get: -1,
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
    #[inline(always)]
    pub(crate) fn push_lua_frame(
        &mut self,
        func: &LuaValue,
        base: usize,
        nparams: usize,
        nresults: i32,
        param_count: usize,
        max_stack_size: usize,
    ) -> LuaResult<()> {
        // Check stack depth (cold — almost never triggers)
        if self.call_depth >= self.safe_option.max_call_depth {
            return self.push_lua_frame_overflow();
        }

        // Pre-compute common values
        let frame_top = base + max_stack_size;

        // Fast path for the common case: enough params (no nil filling needed),
        // stack already large enough, call_stack slot available for reuse.
        // Covers exact match AND extra args (common in metamethods like __len
        // which receives 2 args but declares 1 param).
        if nparams >= param_count
            && frame_top + 5 <= self.stack.len()
            && self.call_depth < self.call_stack.len()
        {
            let ci = unsafe { self.call_stack.get_unchecked_mut(self.call_depth) };
            ci.func = *func;
            ci.base = base;
            ci.func_offset = 1;
            ci.top = frame_top;
            ci.pc = 0;
            ci.nresults = nresults;
            ci.call_status = CIST_LUA;
            ci.nextraargs = (nparams - param_count) as i32;

            self.call_depth += 1;

            // Set stack_top to frame_top for GC safety
            if frame_top > self.stack_top {
                self.stack_top = frame_top;
            }

            return Ok(());
        }

        // Slow path: handle extra args, nil filling, stack resize, new slot allocation
        self.push_lua_frame_slow(
            func,
            base,
            nparams,
            nresults,
            param_count,
            max_stack_size,
            frame_top,
        )
    }

    /// Stack overflow error for push_lua_frame (cold path)
    #[cold]
    #[inline(never)]
    fn push_lua_frame_overflow(&mut self) -> LuaResult<()> {
        if self.safe_option.max_call_depth > self.safe_option.base_call_depth {
            // In error handler extra zone - C Lua's stackerror behavior
            return Err(LuaError::ErrorInErrorHandling);
        }
        Err(self.error(format!(
            "stack overflow (Lua stack depth: {})",
            self.call_depth
        )))
    }

    /// Slow path for push_lua_frame — handles nil filling, resize, new slot allocation
    #[cold]
    #[inline(never)]
    fn push_lua_frame_slow(
        &mut self,
        func: &LuaValue,
        base: usize,
        nparams: usize,
        nresults: i32,
        param_count: usize,
        _max_stack_size: usize,
        frame_top: usize,
    ) -> LuaResult<()> {
        let nextraargs = if nparams > param_count {
            (nparams - param_count) as i32
        } else {
            0
        };

        // Fill missing parameters with nil
        if nparams < param_count {
            let start = base + nparams;
            let end = base + param_count;
            // CRITICAL: Use self.resize() instead of self.stack.resize() so that
            // open upvalue raw pointers are fixed if the Vec reallocates.
            if self.stack.len() < end {
                self.resize(end)?;
            }
            // Batch fill missing parameter slots with nil
            self.stack[start..end].fill(LuaValue::nil());
            if self.stack_top < end {
                self.stack_top = end;
            }
        }

        let needed_physical = frame_top + 5;
        if needed_physical > self.stack.len() {
            self.resize(needed_physical)?;
        }

        // Reuse existing CallInfo slot or allocate new one
        if self.call_depth < self.call_stack.len() {
            let ci = unsafe { self.call_stack.get_unchecked_mut(self.call_depth) };
            ci.func = *func;
            ci.base = base;
            ci.func_offset = 1;
            ci.top = frame_top;
            ci.pc = 0;
            ci.nresults = nresults;
            ci.call_status = CIST_LUA;
            ci.nextraargs = nextraargs;
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
                pending_finish_get: -1,
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
            if self.safe_option.max_call_depth > self.safe_option.base_call_depth {
                // In error handler extra zone - C Lua's stackerror behavior
                return Err(LuaError::ErrorInErrorHandling);
            }
            return Err(self.error(format!(
                "stack overflow (Lua stack depth: {})",
                self.call_depth
            )));
        }

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
                pending_finish_get: -1,
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
                pending_finish_get: -1,
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
        }
    }

    /// Pop a C call frame (specialized fast path, skips call_status bit check).
    /// Caller MUST know the current frame is a C frame.
    #[inline(always)]
    pub(crate) fn pop_c_frame(&mut self) {
        debug_assert!(self.call_depth > 0);
        self.call_depth -= 1;
    }

    /// Get logical stack top (L->top.p in Lua source)
    /// This is the first free slot in the stack, NOT the length of physical stack
    #[inline(always)]
    pub fn get_top(&self) -> usize {
        self.stack_top
    }

    /// Port of lua_checkstack (lapi.c): check if the stack can grow by `n` slots.
    /// Returns true if the stack can accommodate `n` more elements.
    pub fn check_stack(&self, n: usize) -> bool {
        self.stack_top + n <= self.safe_option.max_stack_size
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
                    let source = match chunk.source_name.as_deref() {
                        Some(raw) => crate::compiler::format_source(raw),
                        None => "?".to_string(), // stripped debug info
                    };
                    let line = if ci.pc > 0 && (ci.pc as usize - 1) < chunk.line_info.len() {
                        chunk.line_info[ci.pc as usize - 1] as usize
                    } else {
                        0
                    };
                    location = if line > 0 {
                        format!("{}:{}: ", source, line)
                    } else if chunk.line_info.is_empty() {
                        // No line info at all (stripped) — use ?
                        format!("{}:?: ", source)
                    } else {
                        format!("{}: ", source)
                    };
                }
            }
        };

        self.error_msg = format!("{}{}", location, msg);
        LuaError::RuntimeError
    }

    /// Raise an error from a C function, finding the calling Lua frame for location.
    /// Mirrors C Lua's luaL_error behavior: uses luaL_where(L,1) to get
    /// location from the Lua frame that called the current C function.
    #[cold]
    #[inline(never)]
    pub fn error_from_c(&mut self, msg: String) -> LuaError {
        // Find the nearest Lua frame by traversing up from current frame
        let depth = self.call_depth;
        for level in 0..depth {
            let idx = depth - 1 - level;
            if let Some(ci) = self.get_frame(idx) {
                if ci.is_lua() {
                    if let Some(func_obj) = ci.func.as_lua_function() {
                        let chunk = func_obj.chunk();
                        let source = match chunk.source_name.as_deref() {
                            Some(raw) => crate::compiler::format_source(raw),
                            None => "?".to_string(),
                        };
                        let line = if ci.pc > 0 && (ci.pc as usize - 1) < chunk.line_info.len() {
                            chunk.line_info[ci.pc as usize - 1] as usize
                        } else {
                            0
                        };
                        let location = if line > 0 {
                            format!("{}:{}: ", source, line)
                        } else if chunk.line_info.is_empty() {
                            format!("{}:?: ", source)
                        } else {
                            format!("{}: ", source)
                        };
                        self.error_msg = format!("{}{}", location, msg);
                        return LuaError::RuntimeError;
                    }
                }
            }
        }
        // Fallback: no Lua frame found
        self.error_msg = msg;
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

    /// Get the last error message (for debugging)
    pub fn last_error_msg(&self) -> &str {
        &self.error_msg
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
            // Process upvalues to close in-place, then drain.
            for i in 0..count {
                let upval_ptr = self.open_upvalues_list[i];
                let data = &upval_ptr.as_ref().data;
                if data.is_open() {
                    let stack_idx = data.get_stack_index();

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
                            header.make_black();
                            if let Some(value_gc_ptr) = value.as_gc_ptr() {
                                self.gc_barrier(upval_ptr, value_gc_ptr);
                            }
                        }
                    }
                }
            }
            // Batch remove from front of list
            self.open_upvalues_list.drain(0..count);
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
                    // Root the error on the Lua stack before the call so that
                    // GC can see it (a Rust local is invisible to the collector).
                    // This mirrors C Lua's prepcallclosemth which places the
                    // error at uv+1 and sets L->top accordingly.
                    let err_slot = tbc_idx + 1;
                    let needed = err_slot + 1;
                    if needed > self.stack.len() {
                        self.grow_stack(needed + 3)?;
                    }
                    self.stack[err_slot] = err.clone();
                    self.set_top_raw(err_slot + 1);

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

        let was_closing = self.is_closing;
        self.is_closing = true;

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
                // Match C Lua's prepcallclosemth: set L->top to just past the
                // TBC variable + error slot.  This ensures ALL stack positions
                // below (including pending TBC vars) are within the GC scan
                // range (0..stack_top).  Place error on the stack at tbc_idx+1
                // so it is also a GC root (not just a Rust local).
                let err_slot = tbc_idx + 1;
                let needed = err_slot + 1; // need at least tbc_idx + 2
                if needed > self.stack.len() {
                    self.grow_stack(needed + 3)?;
                }
                self.stack[err_slot] = current_error.clone();
                self.set_top_raw(err_slot + 1);

                // Call __close(obj, err) with 2 arguments.
                // call_close_method_with_error will place the call starting at
                // get_top() == tbc_idx + 2, right after the error slot.
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

        self.is_closing = was_closing;
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
            self.inc_n_ccalls()?;
            let r = lua_execute(self, caller_depth);
            self.dec_n_ccalls();
            r
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
            self.grow_stack(func_pos + 3)?;
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
            self.inc_n_ccalls()?;
            let r = lua_execute(self, caller_depth);
            self.dec_n_ccalls();
            r
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

    /// Find or create an open upvalue for the given stack index.
    /// Uses linear scan on sorted Vec — faster than HashMap for typical 0-5 open upvalues.
    pub fn find_or_create_upvalue(&mut self, stack_index: usize) -> LuaResult<UpvaluePtr> {
        // Linear scan on sorted list (descending by stack index).
        // For 0-5 elements, this is faster than HashMap (no hash computation).
        for &upval_ptr in &self.open_upvalues_list {
            let idx = upval_ptr.as_ref().data.get_stack_index();
            if idx == stack_index {
                return Ok(upval_ptr);
            }
            if idx < stack_index {
                // Passed the insertion point — not found (list is sorted descending)
                break;
            }
        }

        // Not found, create a new one
        let upval_ptr = {
            let ptr = LuaValuePtr {
                ptr: (&self.stack[stack_index]) as *const LuaValue as *mut LuaValue,
            };
            let vm = self.vm_mut();
            vm.create_upvalue_open(stack_index, ptr)?
        };

        // Insert in sorted position (higher indices first)
        let insert_pos = self
            .open_upvalues_list
            .iter()
            .position(|&ptr| ptr.as_ref().data.get_stack_index() < stack_index)
            .unwrap_or(self.open_upvalues_list.len());

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

    /// Lua 5.5-style ccall depth tracking: increment shared n_ccalls before
    /// a recursive `lua_execute` call.  Returns `Err("C stack overflow")` if
    /// the limit is reached.  The limit is checked against this thread's
    /// `safe_option.max_call_depth` so that xpcall's EXTRA_STACK increase
    /// takes effect.
    #[inline(always)]
    pub(crate) fn inc_n_ccalls(&mut self) -> LuaResult<()> {
        let vm = unsafe { &mut *self.vm };
        vm.n_ccalls += 1;
        if vm.n_ccalls >= self.safe_option.max_call_depth {
            vm.n_ccalls -= 1;
            if self.safe_option.max_call_depth > self.safe_option.base_call_depth {
                // In error handler extra zone - C Lua's stackerror behavior
                return Err(LuaError::ErrorInErrorHandling);
            }
            Err(self.error("C stack overflow".to_string()))
        } else {
            Ok(())
        }
    }

    /// Decrement shared n_ccalls after returning from a recursive `lua_execute`.
    #[inline(always)]
    pub(crate) fn dec_n_ccalls(&self) {
        unsafe {
            (*self.vm).n_ccalls -= 1;
        }
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

    /// Pop the current call frame (Lua callers only — does NOT adjust VM n_ccalls)
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
    #[inline]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalues: UpvalueStore) -> CreateResult {
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

    /// Create an RClosure from any `Fn(&mut LuaState) -> LuaResult<usize> + 'static`.
    /// Unlike `CFunction` (bare fn pointer), this can capture arbitrary Rust state.
    #[inline]
    pub fn create_closure<F>(&mut self, func: F) -> CreateResult
    where
        F: Fn(&mut LuaState) -> LuaResult<usize> + 'static,
    {
        self.vm_mut().create_closure(func)
    }

    /// Create an RClosure with upvalues.
    #[inline]
    pub fn create_closure_with_upvalues<F>(
        &mut self,
        func: F,
        upvalues: Vec<LuaValue>,
    ) -> CreateResult
    where
        F: Fn(&mut LuaState) -> LuaResult<usize> + 'static,
    {
        self.vm_mut().create_closure_with_upvalues(func, upvalues)
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

    // ===== Execute =====

    /// Compile and execute a Lua source string, returning results.
    ///
    /// This is a convenience proxy for `LuaVM::execute_string`.
    ///
    /// # Example
    /// ```ignore
    /// let results = state.execute_string("return 1 + 2")?;
    /// assert_eq!(results[0].as_integer(), Some(3));
    /// ```
    pub fn execute_string(&mut self, source: &str) -> LuaResult<Vec<LuaValue>> {
        self.vm_mut().execute_string(source)
    }

    /// Execute a pre-compiled chunk, returning results.
    ///
    /// This is a convenience proxy for `LuaVM::execute`.
    pub fn execute(
        &mut self,
        chunk: std::rc::Rc<crate::lua_value::Chunk>,
    ) -> LuaResult<Vec<LuaValue>> {
        self.vm_mut().execute(chunk)
    }

    // ===== Type Registration =====

    /// Register a UserData type as a Lua global table with its static methods.
    ///
    /// Creates a table (e.g. `Point`) and populates it with all associated
    /// functions defined in the type's `#[lua_methods]` block (functions
    /// without `self`, such as constructors).
    ///
    /// After registration, Lua code can call e.g. `Point.new(3, 4)`.
    ///
    /// # Usage
    /// ```ignore
    /// // In Rust:
    /// state.register_type("Point", Point::__lua_static_methods())?;
    ///
    /// // In Lua:
    /// local p = Point.new(3, 4)
    /// print(p.x, p.y)      -- 3.0  4.0
    /// print(p:distance())   -- 5.0
    /// ```
    pub fn register_type(
        &mut self,
        name: &str,
        static_methods: &[(&str, super::CFunction)],
    ) -> LuaResult<()> {
        let class_table = self.create_table(0, static_methods.len())?;

        for &(method_name, func) in static_methods {
            let key = self.create_string(method_name)?;
            let value = LuaValue::cfunction(func);
            self.raw_set(&class_table, key, value);
        }

        self.set_global(name, class_table)
    }

    /// Register a UserData type by its generic type parameter.
    ///
    /// Equivalent to `register_type(name, T::__lua_static_methods())` but more
    /// concise and type-safe. Uses the `LuaStaticMethodProvider` trait (auto-
    /// implemented by `#[lua_methods]`) to discover static methods.
    ///
    /// # Usage
    /// ```ignore
    /// // Instead of:
    /// state.register_type("Point", Point::__lua_static_methods())?;
    ///
    /// // Write:
    /// state.register_type_of::<Point>("Point")?;
    /// ```
    pub fn register_type_of<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<()> {
        self.register_type(name, T::lua_static_methods())
    }

    // ===== Table Operations =====

    /// Get value from table (raw, no metamethods)
    pub fn raw_get(&mut self, table: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        self.vm_mut().raw_get(table, key)
    }

    /// Get value from table with __index metamethod support
    pub fn table_get(&mut self, table: &LuaValue, key: &LuaValue) -> LuaResult<Option<LuaValue>> {
        // First try raw access
        if let Some(val) = self.vm_mut().raw_get(table, key) {
            return Ok(Some(val));
        }
        // If not found, try __index metamethod
        execute::helper::lookup_from_metatable(self, table, key)
    }

    /// Set value in table with metamethod support (__newindex)
    pub fn table_set(&mut self, table: &LuaValue, key: LuaValue, value: LuaValue) -> LuaResult<()> {
        execute::helper::finishset(self, table, &key, value)?;
        Ok(())
    }

    /// Compare two values using < operator with metamethod support (__lt)
    pub fn obj_lt(&mut self, a: &LuaValue, b: &LuaValue) -> LuaResult<bool> {
        // Integer-integer
        if let (Some(i1), Some(i2)) = (a.as_integer(), b.as_integer()) {
            return Ok(i1 < i2);
        }
        // Float-float
        if let (Some(f1), Some(f2)) = (a.as_float(), b.as_float()) {
            return Ok(f1 < f2);
        }
        // Mixed number
        if let (Some(n1), Some(n2)) = (a.as_number(), b.as_number()) {
            return Ok(n1 < n2);
        }
        // String (including binary): compare raw bytes
        if a.is_string() && b.is_string() {
            let ba = a.as_str().map(|s| s.as_bytes()).or_else(|| a.as_binary());
            let bb = b.as_str().map(|s| s.as_bytes()).or_else(|| b.as_binary());
            if let (Some(ba), Some(bb)) = (ba, bb) {
                return Ok(ba < bb);
            }
        }
        // Try __lt metamethod
        match execute::metamethod::try_comp_tm(self, *a, *b, execute::TmKind::Lt) {
            Ok(Some(result)) => Ok(result),
            Ok(None) => Err(crate::stdlib::debug::ordererror(self, a, b)),
            Err(e) => Err(e),
        }
    }

    /// Get object length with metamethod support (__len)
    /// Returns the length as i64, going through __len if available.
    pub fn obj_len(&mut self, obj: &LuaValue) -> LuaResult<i64> {
        if let Some(s) = obj.as_str() {
            return Ok(s.len() as i64);
        }
        if let Some(table) = obj.as_table_mut() {
            let meta = table.meta_ptr();
            if !meta.is_null() {
                let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
                const TM_LEN_BIT: u8 = execute::TmKind::Len as u8;
                if !mt.no_tm(TM_LEN_BIT) {
                    let event_key = self
                        .vm_mut()
                        .const_strings
                        .get_tm_value(execute::TmKind::Len);
                    if let Some(mm) = mt.raw_get(&event_key) {
                        let result = execute::call_tm_res(self, mm, *obj, *obj)?;
                        return result.as_integer().ok_or_else(|| {
                            self.error("object length is not an integer".to_string())
                        });
                    } else {
                        mt.set_tm_absent(TM_LEN_BIT);
                    }
                }
            }
            return Ok(table.len() as i64);
        }
        if let Some(mm) = execute::get_metamethod_event(self, obj, execute::TmKind::Len) {
            let result = execute::call_tm_res(self, mm, *obj, *obj)?;
            return result
                .as_integer()
                .ok_or_else(|| self.error("object length is not an integer".to_string()));
        }
        Err(self.error(format!(
            "attempt to get length of a {} value",
            obj.type_name()
        )))
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

    // ===== Unprotected Call =====

    /// Unprotected call - like C Lua's lua_call / lua_callk.
    /// Errors propagate as Err(LuaError) to the enclosing pcall boundary.
    /// Does NOT create an error recovery boundary, so __close handlers
    /// see the correct error chain without an extra pcall frame.
    pub fn call(&mut self, func: LuaValue, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>> {
        let initial_depth = self.call_depth();
        // Use stack_top (logical top) instead of stack.len() (physical end).
        // The physical stack can be much larger than needed (e.g., after deep
        // recursion tests), so placing the function at stack.len() would waste
        // address space and risk hitting max_stack_size limits unnecessarily.
        let func_idx = self.stack_top;
        let arg_count = args.len();
        let needed = func_idx + 1 + arg_count;

        // Ensure physical stack has room for function + args
        if needed > self.stack.len() {
            self.resize(needed)?;
        }

        // Write function and args at stack_top position
        self.stack[func_idx] = func;
        for (i, arg) in args.into_iter().enumerate() {
            self.stack[func_idx + 1 + i] = arg;
        }
        self.stack_top = needed;

        // Resolve __call metamethod chain if needed
        let (actual_arg_count, ccmt_depth) = resolve_call_chain(self, func_idx, arg_count)?;

        let func_val = self
            .stack_get(func_idx)
            .ok_or_else(|| self.error("call: function not found".to_string()))?;

        if func_val.is_c_callable() {
            // C function - call directly via call_c_function (unprotected)
            call_c_function(self, func_idx, actual_arg_count, -1)?;
        } else {
            // Lua function - push frame and execute
            let base = func_idx + 1;
            self.push_frame(&func_val, base, actual_arg_count, -1)?;

            if ccmt_depth > 0 {
                let frame_idx = self.call_depth - 1;
                if let Some(frame) = self.call_stack.get_mut(frame_idx) {
                    frame.call_status = call_status::set_ccmt_count(frame.call_status, ccmt_depth);
                }
            }

            self.inc_n_ccalls()?;
            let r = lua_execute(self, initial_depth);
            self.dec_n_ccalls();
            r?; // Propagate errors without catching
        }

        // Collect results from func_idx to stack_top
        let mut results = Vec::new();
        for i in func_idx..self.stack_top {
            if let Some(val) = self.stack_get(i) {
                results.push(val);
            }
        }

        // Clean up: nil out used slots for GC safety, restore stack_top.
        // Don't truncate the physical stack — it may be needed by the caller's
        // frame (ci.top). The physical stack will be shrunk naturally by GC or
        // when the enclosing frame exits.
        {
            let clear_end = self.stack_top.min(self.stack.len());
            for i in func_idx..clear_end {
                self.stack[i] = LuaValue::nil();
            }
        }
        self.stack_top = func_idx;

        // Restore caller frame top if needed
        if self.call_depth() > 0 {
            let ci_idx = self.call_depth() - 1;
            let frame_top = self.get_call_info(ci_idx).top;
            if self.stack_top < frame_top {
                self.stack_top = frame_top;
            }
        }

        Ok(results)
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
        // This is equivalent to C Lua's lua_call → luaD_callnoyield:
        // the callback runs in a non-yieldable context.
        self.nny += 1;
        let result = self.pcall_inner(func, args);
        self.nny -= 1;
        result
    }

    /// Inner implementation of pcall (separated for nny scoping)
    fn pcall_inner(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Save state for cleanup
        let initial_depth = self.call_depth();
        let saved_stack_top = self.stack_top;
        // Use stack_top (logical top) instead of stack.len() (physical end).
        // See call() for rationale.
        let func_idx = self.stack_top;
        let arg_count = args.len();
        let needed = func_idx + 1 + arg_count;

        // Ensure physical stack has room for function + args
        if needed > self.stack.len() {
            self.resize(needed)?;
        }

        // Write function and args at stack_top position
        self.stack[func_idx] = func;
        for (i, arg) in args.into_iter().enumerate() {
            self.stack[func_idx + 1 + i] = arg;
        }

        // Sync logical stack top
        self.stack_top = needed;

        // Resolve __call chain if needed

        let (actual_arg_count, ccmt_depth) = match resolve_call_chain(self, func_idx, arg_count) {
            Ok((count, depth)) => (count, depth),
            Err(e) => {
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
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

            // CloseThread bypasses all pcalls — don't pop this frame,
            // handle_resume_result will pop everything.
            if matches!(result, Err(LuaError::CloseThread)) {
                return Err(LuaError::CloseThread);
            }

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
                    self.stack_top = saved_stack_top;

                    Ok((true, results))
                }
                Err(LuaError::Yield) => Err(LuaError::Yield),
                Err(LuaError::CloseThread) => Err(LuaError::CloseThread),
                Err(e) => {
                    let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                    let result_err = if !err_obj.is_nil() {
                        err_obj
                    } else {
                        let error_msg = self.get_error_msg(e);
                        self.create_string(&error_msg)?.into()
                    };
                    self.stack_top = saved_stack_top;
                    Ok((false, vec![result_err]))
                }
            }
        } else {
            // Lua function - use lua_execute
            let base = func_idx + 1;
            // pcall expects all return values
            if let Err(e) = self.push_frame(&func, base, actual_arg_count, -1) {
                self.stack_top = saved_stack_top;
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                return Ok((false, vec![err_str]));
            }

            // Execute via lua_execute — only execute the new frame
            self.inc_n_ccalls()?;
            let result = execute::lua_execute(self, initial_depth);
            self.dec_n_ccalls();

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
                    self.stack_top = saved_stack_top;

                    Ok((true, results))
                }
                Err(LuaError::Yield) => Err(LuaError::Yield),
                Err(LuaError::CloseThread) => Err(LuaError::CloseThread),
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
                    self.stack_top = saved_stack_top;

                    Ok((false, vec![result_err]))
                }
            }
        }
    }

    /// Unprotected call with stack-based arguments that supports yields.
    /// Like pcall_stack_based but errors propagate instead of being caught.
    /// Uses CIST_YCALL flag so finish_c_frame can properly move results after yield.
    /// Returns result_count on success; results are left on stack starting at func_idx.
    pub fn call_stack_based(&mut self, func_idx: usize, arg_count: usize) -> LuaResult<usize> {
        let initial_depth = self.call_depth();

        let (actual_arg_count, _ccmt_depth) = resolve_call_chain(self, func_idx, arg_count)?;

        let func = self
            .stack_get(func_idx)
            .ok_or_else(|| self.error("call: function not found".to_string()))?;

        let result = if func.is_c_callable() {
            call_c_function(self, func_idx, actual_arg_count, -1).map(|_| ())
        } else {
            let base = func_idx + 1;
            self.push_frame(&func, base, actual_arg_count, -1)?;
            self.inc_n_ccalls()?;
            let r = lua_execute(self, initial_depth);
            self.dec_n_ccalls();
            r
        };

        match result {
            Ok(()) => {
                let stack_top = self.get_top();
                let result_count = if stack_top > func_idx {
                    stack_top - func_idx
                } else {
                    0
                };
                Ok(result_count)
            }
            Err(LuaError::Yield) => {
                // Mark this C frame with CIST_YCALL so finish_c_frame
                // knows to move results (without prepending true/false).
                if initial_depth > 0 {
                    use crate::lua_vm::call_info::call_status::CIST_YCALL;
                    let frame_idx = initial_depth - 1;
                    if frame_idx < self.call_depth {
                        let ci = self.get_call_info_mut(frame_idx);
                        ci.call_status |= CIST_YCALL;
                    }
                }
                Err(LuaError::Yield)
            }
            Err(e) => Err(e), // Propagate all other errors
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

            self.inc_n_ccalls()?;
            let r = lua_execute(self, initial_depth);
            self.dec_n_ccalls();
            r
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
            Err(LuaError::CloseThread) => {
                // CloseThread bypasses all pcalls — propagate to resume
                Err(LuaError::CloseThread)
            }
            Err(LuaError::ErrorInErrorHandling) => {
                // Stack overflow in error handler zone - C Lua's stackerror
                let frame_base = if self.call_depth() > initial_depth {
                    self.call_stack.get(initial_depth).map(|f| f.base)
                } else {
                    None
                };
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }
                if let Some(base) = frame_base {
                    self.close_upvalues(base);
                    let _ = self.close_tbc_with_error(base, LuaValue::nil());
                }
                let err_msg = self.create_string("error in error handling")?.into();
                self.stack_set(func_idx, err_msg)?;
                self.set_top(func_idx + 1)?;
                Ok((false, 1))
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

    /// Protected call with error handler, stack-based (xpcall semantics).
    /// Like pcall_stack_based but calls the error handler at `handler_idx`
    /// BEFORE unwinding call frames, so debug.traceback can see the full stack.
    /// Returns (success, result_count) where results are left on stack at func_idx.
    pub fn xpcall_stack_based(
        &mut self,
        func_idx: usize,
        arg_count: usize,
        handler_idx: usize,
    ) -> LuaResult<(bool, usize)> {
        let initial_depth = self.call_depth();

        let (actual_arg_count, ccmt_depth) = match resolve_call_chain(self, func_idx, arg_count) {
            Ok((count, depth)) => (count, depth),
            Err(e) => {
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                self.stack_set(func_idx, err_str)?;
                self.set_top(func_idx + 1)?;
                return Ok((false, 1));
            }
        };

        let func = self
            .stack_get(func_idx)
            .ok_or_else(|| self.error("xpcall: function not found after resolution".to_string()))?;

        let result = if func.is_c_callable() {
            call_c_function(self, func_idx, actual_arg_count, -1).map(|_| ())
        } else {
            let base = func_idx + 1;
            self.push_frame(&func, base, actual_arg_count, -1)?;
            if ccmt_depth > 0 {
                let frame_idx = self.call_depth - 1;
                if let Some(frame) = self.call_stack.get_mut(frame_idx) {
                    frame.call_status = call_status::set_ccmt_count(frame.call_status, ccmt_depth);
                }
            }
            self.inc_n_ccalls()?;
            let r = lua_execute(self, initial_depth);
            self.dec_n_ccalls();
            r
        };

        match result {
            Ok(()) => {
                let stack_top = self.get_top();
                let result_count = if stack_top > func_idx {
                    stack_top - func_idx
                } else {
                    0
                };
                Ok((true, result_count))
            }
            Err(LuaError::Yield) => {
                if initial_depth > 0 {
                    let pcall_frame_idx = initial_depth - 1;
                    if pcall_frame_idx < self.call_depth {
                        let ci = self.get_call_info_mut(pcall_frame_idx);
                        ci.call_status |= CIST_YPCALL;
                    }
                }
                Err(LuaError::Yield)
            }
            Err(LuaError::CloseThread) => Err(LuaError::CloseThread),
            Err(LuaError::ErrorInErrorHandling) => {
                // Stack overflow in error handler zone - skip handler entirely
                let frame_base = if self.call_depth() > initial_depth {
                    self.call_stack.get(initial_depth).map(|f| f.base)
                } else {
                    None
                };
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }
                if let Some(base) = frame_base {
                    self.close_upvalues(base);
                    let _ = self.close_tbc_with_error(base, LuaValue::nil());
                }
                let err_msg = self.create_string("error in error handling")?.into();
                self.stack_set(func_idx, err_msg)?;
                self.set_top(func_idx + 1)?;
                Ok((false, 1))
            }
            Err(e) => {
                // Get error object BEFORE any cleanup
                let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                let error_msg_str = self.get_error_msg(e);

                let mut err_value = if !err_obj.is_nil() {
                    err_obj.clone()
                } else {
                    self.create_string(&error_msg_str)?.into()
                };

                let frame_base = if self.call_depth() > initial_depth {
                    self.call_stack.get(initial_depth).map(|f| f.base)
                } else {
                    None
                };

                // Temporarily increase max_call_depth for error handler
                let saved_max_depth = self.safe_option.max_call_depth;
                self.safe_option.max_call_depth = saved_max_depth + 30;

                // Call error handler WITH ALL ERROR FRAMES STILL ON STACK.
                // C Lua's luaG_errormsg recursively calls the handler when the
                // handler itself errors. We implement this as a loop.
                // Track accumulated "virtual depth" to simulate C Lua's nCcalls
                // accumulation during recursive handler calls.
                let handler = self.stack_get(handler_idx).unwrap_or(LuaValue::nil());

                let mut handler_failed = false;
                let mut transformed_error = LuaValue::nil();

                // Budget: how many retries before we hit the depth limit.
                // In C Lua, each recursive handler call adds ~1 to nCcalls.
                // At MAXCCALLS (200), "C stack overflow" fires.
                // At MAXCCALLS*1.1 (220), hard "error in error handling" fires.
                let depth_budget = saved_max_depth.saturating_sub(initial_depth);
                let hard_limit = depth_budget + 30; // extra room for error handling
                let mut retry_count: usize = 0;

                loop {
                    retry_count += 1;

                    // Hard limit: too many retries even in error zone
                    if retry_count > hard_limit {
                        handler_failed = true;
                        break;
                    }

                    // Soft limit: generate "C stack overflow" error for handler
                    if retry_count > depth_budget {
                        let overflow_str = self.create_string("C stack overflow")?;
                        err_value = overflow_str.into();
                    }

                    let current_top = self.stack_top;
                    self.push_value(handler.clone())?;
                    let handler_func_idx = current_top;
                    self.push_value(err_value.clone())?;

                    let handler_depth = self.call_depth();

                    let handler_result =
                        if handler.is_cfunction() || handler.as_cclosure().is_some() {
                            call_c_function(self, handler_func_idx, 1, -1)
                        } else {
                            match self.push_frame(&handler, handler_func_idx + 1, 1, -1) {
                                Ok(()) => {
                                    self.inc_n_ccalls()?;
                                    let r = lua_execute(self, handler_depth);
                                    self.dec_n_ccalls();
                                    r
                                }
                                Err(handler_err) => Err(handler_err),
                            }
                        };

                    match handler_result {
                        Ok(_) => {
                            transformed_error =
                                self.stack_get(handler_func_idx).unwrap_or(LuaValue::nil());
                            while self.call_depth() > handler_depth {
                                self.pop_frame();
                            }
                            break;
                        }
                        Err(LuaError::ErrorInErrorHandling) => {
                            handler_failed = true;
                            self.error_object = LuaValue::nil();
                            while self.call_depth() > handler_depth {
                                self.pop_frame();
                            }
                            self.set_top(current_top)?;
                            break;
                        }
                        Err(_handler_err) => {
                            // Handler failed with normal error — retry with new error
                            let new_err =
                                std::mem::replace(&mut self.error_object, LuaValue::nil());
                            while self.call_depth() > handler_depth {
                                self.pop_frame();
                            }
                            self.set_top(current_top)?;
                            if new_err.is_nil() {
                                handler_failed = true;
                                break;
                            }
                            err_value = new_err;
                        }
                    }
                }

                // Restore max_call_depth
                self.safe_option.max_call_depth = saved_max_depth;

                // NOW pop error frames (after handler has seen them)
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                // Close upvalues and TBC
                if let Some(base) = frame_base {
                    self.close_upvalues(base);
                    let close_result = self.close_tbc_with_error(base, err_obj);
                    match close_result {
                        Ok(()) => {}
                        Err(LuaError::Yield) => {
                            let pcall_ci_idx = initial_depth - 1;
                            if pcall_ci_idx < self.call_depth {
                                let ci = self.get_call_info_mut(pcall_ci_idx);
                                ci.call_status |= CIST_YPCALL | CIST_RECST;
                            }
                            let cascaded =
                                std::mem::replace(&mut self.error_object, LuaValue::nil());
                            self.error_object = if !cascaded.is_nil() {
                                cascaded
                            } else {
                                err_value
                            };
                            return Err(LuaError::Yield);
                        }
                        Err(_e2) => {}
                    }
                }

                // Determine final error value
                let final_error = if handler_failed {
                    self.create_string("error in error handling")?.into()
                } else {
                    transformed_error
                };

                self.stack_set(func_idx, final_error)?;
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

        let is_c_callable = func.is_c_callable();

        // Execute the function
        let result = if is_c_callable {
            // C function — call directly (like pcall_inner's C path)
            let base = func_idx + 1;
            if let Err(e) = self.push_frame(&func, base, actual_arg_count, -1) {
                self.set_top(handler_idx)?;
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                return Ok((false, vec![err_str]));
            }

            if ccmt_depth > 0 {
                use crate::lua_vm::call_info::call_status;
                let frame_idx = self.call_depth - 1;
                if let Some(frame) = self.call_stack.get_mut(frame_idx) {
                    frame.call_status = call_status::set_ccmt_count(frame.call_status, ccmt_depth);
                }
            }

            let cfunc = if let Some(c_func) = func.as_cfunction() {
                c_func
            } else if let Some(closure) = func.as_cclosure() {
                closure.func()
            } else {
                unreachable!()
            };

            let c_result = cfunc(self);

            // CloseThread bypasses everything
            if matches!(c_result, Err(LuaError::CloseThread)) {
                return Err(LuaError::CloseThread);
            }

            self.pop_frame();

            match c_result {
                Ok(nresults) => {
                    // Collect results
                    let result_start = if self.stack_top >= nresults {
                        self.stack_top - nresults
                    } else {
                        0
                    };
                    // Move results to func_idx
                    for i in 0..nresults {
                        let val = self.stack_get(result_start + i).unwrap_or(LuaValue::nil());
                        self.stack_set(func_idx + i, val)?;
                    }
                    self.set_top(func_idx + nresults)?;
                    Ok(())
                }
                Err(LuaError::Yield) => Err(LuaError::Yield),
                Err(e) => Err(e),
            }
        } else {
            // Lua function — push frame and execute
            let base = func_idx + 1;
            if let Err(e) = self.push_frame(&func, base, actual_arg_count, -1) {
                self.set_top(handler_idx)?;
                let error_msg = self.get_error_msg(e);
                let err_str = self.create_string(&error_msg)?;
                return Ok((false, vec![err_str]));
            }

            if ccmt_depth > 0 {
                use crate::lua_vm::call_info::call_status;
                let frame_idx = self.call_depth - 1;
                if let Some(frame) = self.call_stack.get_mut(frame_idx) {
                    frame.call_status = call_status::set_ccmt_count(frame.call_status, ccmt_depth);
                }
            }

            self.inc_n_ccalls()?;
            let r = execute::lua_execute(self, initial_depth);
            self.dec_n_ccalls();
            r
        };

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
            Err(LuaError::CloseThread) => Err(LuaError::CloseThread),
            Err(LuaError::ErrorInErrorHandling) => {
                // Stack overflow in error handler zone - skip handler entirely
                let frame_base = if self.call_depth() > initial_depth {
                    self.call_stack.get(initial_depth).map(|f| f.base)
                } else {
                    None
                };
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }
                if let Some(base) = frame_base {
                    self.close_upvalues(base);
                    let _ = self.close_tbc_with_error(base, LuaValue::nil());
                }
                let results = vec![self.create_string("error in error handling")?];
                self.set_top(handler_idx)?;
                Ok((false, results))
            }
            Err(e) => {
                // so that debug.traceback can see the full call stack
                // (mirrors CLua's luaG_errormsg which calls handler before longjmp)

                // Get error object BEFORE any cleanup
                let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                let error_msg_str = self.get_error_msg(e);

                // Prepare error value for the handler
                let mut err_value = if !err_obj.is_nil() {
                    err_obj.clone()
                } else {
                    self.create_string(&error_msg_str)?.into()
                };

                // Get frame_base for later cleanup (upvalues/TBC)
                let frame_base = if self.call_depth() > initial_depth {
                    self.call_stack.get(initial_depth).map(|f| f.base)
                } else {
                    None
                };

                // Temporarily increase max_call_depth for error handler
                // (like CLua's EXTRA_STACK — allows error handlers to run
                // even after stack overflow)
                let saved_max_depth = self.safe_option.max_call_depth;
                self.safe_option.max_call_depth = saved_max_depth + 30;

                // Call error handler with error value.
                // C Lua's luaG_errormsg recursively calls the handler when the
                // handler itself errors. We implement this as a loop: if the
                // handler fails with a normal error, retry with the new error.
                let handler = self.stack_get(handler_idx).unwrap_or(LuaValue::nil());

                let mut results = Vec::new();
                let mut handler_failed = false;

                loop {
                    let current_top = self.stack_top;
                    self.push_value(handler.clone())?;
                    let handler_func_idx = current_top;
                    self.push_value(err_value.clone())?;

                    let handler_depth = self.call_depth();

                    let handler_result =
                        if handler.is_cfunction() || handler.as_cclosure().is_some() {
                            execute::call::call_c_function(self, handler_func_idx, 1, -1)
                        } else {
                            match self.push_frame(&handler, handler_func_idx + 1, 1, -1) {
                                Ok(()) => {
                                    self.inc_n_ccalls()?;
                                    let r = execute::lua_execute(self, handler_depth);
                                    self.dec_n_ccalls();
                                    r
                                }
                                Err(handler_err) => Err(handler_err),
                            }
                        };

                    match handler_result {
                        Ok(()) => {
                            // Handler succeeded — collect results
                            let result_top = self.stack_top;
                            if result_top > handler_func_idx {
                                for i in handler_func_idx..result_top {
                                    if let Some(val) = self.stack_get(i) {
                                        results.push(val);
                                    }
                                }
                            }
                            // Clean up handler frames
                            while self.call_depth() > handler_depth {
                                self.pop_frame();
                            }
                            break;
                        }
                        Err(LuaError::ErrorInErrorHandling) => {
                            // Stack overflow in error handler zone — give up
                            handler_failed = true;
                            self.error_object = LuaValue::nil();
                            while self.call_depth() > handler_depth {
                                self.pop_frame();
                            }
                            self.set_top(current_top)?;
                            break;
                        }
                        Err(_handler_err) => {
                            // Handler failed with a normal error.
                            // C Lua retries: luaG_errormsg calls errfunc again.
                            // Get the new error value and retry.
                            let new_err =
                                std::mem::replace(&mut self.error_object, LuaValue::nil());
                            // Clean up handler frames before retrying
                            while self.call_depth() > handler_depth {
                                self.pop_frame();
                            }
                            // Reset stack top to before handler push
                            self.set_top(current_top)?;
                            if new_err.is_nil() {
                                // No error object — can't retry, treat as failure
                                handler_failed = true;
                                break;
                            }
                            err_value = new_err;
                            // Loop continues — retry handler with new error value
                        }
                    }
                }

                // Restore max_call_depth after error handler completes
                self.safe_option.max_call_depth = saved_max_depth;

                // NOW pop the error frames (after handler has seen them)
                while self.call_depth() > initial_depth {
                    self.pop_frame();
                }

                // Close upvalues and TBC
                if let Some(base) = frame_base {
                    self.close_upvalues(base);
                    let _ = self.close_tbc_with_error(base, err_obj);
                }

                // Check for cascading error from TBC
                let cascaded_err = std::mem::replace(&mut self.error_object, LuaValue::nil());
                if !cascaded_err.is_nil() && results.is_empty() {
                    if let Some(s) = cascaded_err.as_str() {
                        results.push(self.create_string(s)?);
                    } else {
                        results.push(cascaded_err);
                    }
                }

                if results.is_empty() {
                    if handler_failed {
                        results.push(self.create_string("error in error handling")?);
                    } else {
                        results.push(self.create_string(&error_msg_str)?);
                    }
                }

                self.set_top(handler_idx)?;
                Ok((false, results))
            }
        }
    }

    // ===== Coroutine Support (resume/yield) =====

    /// Resume a coroutine (should be called on the thread's LuaState)
    /// Returns (finished, results) where:
    /// - finished=true: coroutine completed normally
    /// - finished=false: coroutine yielded
    pub fn resume(&mut self, args: Vec<LuaValue>) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Check coroutine state:
        // - call_depth > 0 && !yielded → running (cannot resume)
        // - call_depth > 0 && yielded → suspended after yield (can resume)
        // - call_depth == 0 && stack not empty → initial state (can resume)
        // - call_depth == 0 && stack empty → dead (cannot resume)
        if self.call_depth > 0 && !self.yielded {
            return Err(self.error("cannot resume non-suspended coroutine".to_string()));
        }

        // Mark as running (not yielded)
        self.yielded = false;

        // Check if this is the first resume (no active frames)
        if self.call_depth == 0 {
            // Initial resume - need to set up the function
            // The function should be at stack[0] (set by create_thread)
            if self.stack.is_empty() {
                self.error_object = LuaValue::nil(); // clear stale error object
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
                self.inc_n_ccalls()?;
                let r = execute::lua_execute(self, 0);
                self.dec_n_ccalls();
                r
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

            // Update stack top to result extent.
            // Values above this position were nil'd by GC's atomic phase
            // (traverse_thread clears top..stack_end, matching C Lua).
            let new_top = func_idx + actual_nresults;
            self.set_top_raw(new_top);

            // Execute until yield or completion
            self.inc_n_ccalls()?;
            let result = execute::lua_execute(self, 0);
            self.dec_n_ccalls();

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
                    // Coroutine completed — pop any remaining frames
                    // (e.g., the initial frame pushed by resume for C functions)
                    let results = self.get_all_return_values(0);
                    while self.call_depth() > 0 {
                        self.pop_frame();
                    }
                    self.stack.clear();
                    self.stack_top = 0;
                    return Ok((true, results));
                }
                Err(LuaError::Yield) => {
                    // Coroutine yielded — mark as yielded for resume detection
                    self.yielded = true;
                    let yield_vals = self.take_yield();
                    return Ok((false, yield_vals));
                }
                Err(LuaError::CloseThread) => {
                    // Self-close: coroutine.close() closed TBC vars/upvalues
                    // and threw CloseThread to bypass all pcalls.
                    // Pop all remaining frames and mark thread as dead.
                    while self.call_depth() > 0 {
                        self.pop_frame();
                    }
                    // Check if __close set an error
                    let err_obj = std::mem::replace(&mut self.error_object, LuaValue::nil());
                    self.stack.clear();
                    self.stack_top = 0;
                    if err_obj.is_nil() {
                        // Normal close — success with no return values
                        return Ok((true, vec![]));
                    } else {
                        // __close errored — coroutine dies with error.
                        // Store error back so coroutine_resume can retrieve it.
                        self.error_object = err_obj;
                        return Err(LuaError::RuntimeError);
                    }
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

                        // Mark coroutine as dead by clearing the stack.
                        // error_msg is stored separately and remains accessible.
                        self.stack.clear();
                        self.stack_top = 0;

                        return Err(_e);
                    }
                    let pcall_frame_idx = pcall_idx.unwrap();

                    // Get pcall's info before cleanup
                    let pcall_ci = self.get_call_info(pcall_frame_idx);
                    let pcall_func_pos = pcall_ci.base - pcall_ci.func_offset;
                    let pcall_nresults = pcall_ci.nresults;
                    let close_level = pcall_ci.base; // close from body position
                    let is_xpcall = pcall_ci.call_status & CIST_XPCALL != 0;

                    // Save the xpcall handler before anything overwrites it
                    let xpcall_handler = if is_xpcall {
                        self.stack_get(pcall_func_pos).unwrap_or(LuaValue::nil())
                    } else {
                        LuaValue::nil()
                    };

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

                            // If xpcall, call error handler to transform the error
                            let result_err = if is_xpcall {
                                self.nny += 1;
                                let handler_result =
                                    self.pcall(xpcall_handler.clone(), vec![result_err.clone()]);
                                self.nny -= 1;
                                match handler_result {
                                    Ok((true, results)) => {
                                        results.into_iter().next().unwrap_or(LuaValue::nil())
                                    }
                                    _ => self.create_string("error in error handling")?,
                                }
                            } else {
                                result_err
                            };

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
                            if let Err(e) = self.inc_n_ccalls() {
                                result = Err(e);
                            } else {
                                result = execute::lua_execute(self, 0);
                                self.dec_n_ccalls();
                            }
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

                            // Mark coroutine as yielded so resume works
                            self.yielded = true;

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
                // Check for trait-based __tostring on userdata
                if value.ttisfulluserdata() {
                    if let Some(ud) = value.as_userdata_mut() {
                        if let Some(s) = ud.get_trait().lua_tostring() {
                            return Ok(s);
                        }
                    }
                }
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
        // Check for __name in metatable (luaT_objtypename)
        let type_prefix = crate::stdlib::debug::objtypename(self, value);
        if type_prefix != value.type_name() {
            // Use __name as prefix instead of built-in type name
            Ok(format!(
                "{}: 0x{:x}",
                type_prefix,
                value.raw_ptr_repr() as usize
            ))
        } else {
            Ok(format!("{}", value))
        }
    }

    pub fn is_main_thread(&self) -> bool {
        self.is_main
    }

    /// Check if this coroutine has yielded and is waiting to be resumed.
    pub fn is_yielded(&self) -> bool {
        self.yielded
    }
}

impl Default for LuaState {
    fn default() -> Self {
        Self::new(1, std::ptr::null_mut(), false, SafeOption::default())
    }
}
