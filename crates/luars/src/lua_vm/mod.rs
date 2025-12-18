// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
mod execute;
mod lua_call_frame;
mod lua_error;
mod opcode;

use crate::compiler::{compile_code, compile_code_with_name};
use crate::gc::{FunctionId, GC, GcFunction, GcId, TableId, ThreadId, UpvalueId};
#[cfg(feature = "async")]
use crate::lua_async::AsyncExecutor;
use crate::lua_value::{
    Chunk, CoroutineStatus, LuaString, LuaTable, LuaThread, LuaValue, LuaValueKind, tm_flags,
};
pub use crate::lua_vm::lua_call_frame::LuaCallFrame;
pub use crate::lua_vm::lua_error::LuaError;
use crate::{ObjectPool, lib_registry};
pub use opcode::{Instruction, OpCode};
use std::cell::RefCell;
use std::rc::Rc;

pub type LuaResult<T> = Result<T, LuaError>;

/// Maximum call stack depth (similar to LUAI_MAXCCALLS in Lua)
/// Using a lower limit because Rust stack space is limited
pub const MAX_CALL_DEPTH: usize = 64;

pub struct LuaVM {
    // Global environment table (_G and _ENV point to this)
    pub(crate) global: TableId,

    // Registry table (like Lua's LUA_REGISTRYINDEX)
    // Used to store objects that should be protected from GC but not visible to Lua code
    // This is a GC root and all values in it are protected
    pub(crate) registry: TableId,

    // GC roots buffer - pre-allocated to avoid allocation during GC
    gc_roots_buffer: Vec<LuaValue>,

    // Call stack - Pre-allocated Vec with fixed capacity
    // Using Vec<LuaCallFrame> directly (no Box indirection) for cache efficiency
    // Vec is pre-allocated to MAX_CALL_DEPTH and never reallocated
    pub frames: Vec<LuaCallFrame>,

    // Current frame index (0 = empty, 1 = first frame, etc.)
    // This replaces the linked-list traversal in Lua's L->ci
    pub(crate) frame_count: usize,

    // Global register stack (unified stack architecture, like Lua 5.4)
    pub register_stack: Vec<LuaValue>,

    // Object pool for unified object management (new architecture)
    // Placed near top for cache locality with hot operations
    pub(crate) object_pool: ObjectPool,

    // Garbage collector (cold path - only accessed during actual GC)
    pub(crate) gc: GC,

    // Multi-return value buffer (temporary storage for function returns)
    pub return_values: Vec<LuaValue>,

    // Open upvalues list (for closing when frames exit) - uses UpvalueId for new architecture
    pub(crate) open_upvalues: Vec<UpvalueId>,

    // To-be-closed variables stack (for __close metamethod)
    // Stores (register_index, value) pairs that need __close called when they go out of scope
    pub(crate) to_be_closed: Vec<(usize, LuaValue)>,

    // Next frame ID (for tracking frames)
    pub(crate) next_frame_id: usize,

    // Error handling state
    pub error_handler: Option<LuaValue>, // Current error handler for xpcall

    // FFI state
    #[cfg(feature = "loadlib")]
    pub(crate) ffi_state: crate::ffi::FFIState,

    // Current running thread (for coroutine.running()) - legacy Rc-based
    pub current_thread: Option<Rc<RefCell<LuaThread>>>,

    // Current running thread ID (for new ObjectPool-based architecture)
    pub current_thread_id: Option<ThreadId>,

    // Current thread as LuaValue (for comparison in coroutine.running())
    pub current_thread_value: Option<LuaValue>,

    // Main thread representation (for coroutine.running() in main thread)
    pub main_thread_value: Option<LuaValue>,

    // String metatable (shared by all strings) - stored as TableId in LuaValue
    pub(crate) string_metatable: Option<LuaValue>,

    // C closure upvalues (temporary storage during C closure call)
    // Points to the upvalues array of the currently executing C closure
    pub(crate) c_closure_upvalues_ptr: *const UpvalueId,
    pub(crate) c_closure_upvalues_len: usize,

    // Single inline upvalue for CClosureInline1 (no indirection needed)
    pub(crate) c_closure_inline_upvalue: LuaValue,

    // Async executor for Lua-Rust async bridge
    #[cfg(feature = "async")]
    pub(crate) async_executor: AsyncExecutor,

    // ===== Lightweight Error Storage =====
    // Store error/yield data here instead of in Result<T, LuaError>
    // This reduces Result size from ~24 bytes to 1 byte!
    /// Error message for RuntimeError/CompileError
    pub(crate) error_message: String,

    /// Yield values for coroutine yield
    pub(crate) yield_values: Vec<LuaValue>,
}

impl LuaVM {
    pub fn new() -> Self {
        // Pre-allocate call stack with fixed size (like Lua's CallInfo pool)
        // Vec is pre-filled to MAX_CALL_DEPTH so we can use direct indexing
        // frame_count tracks the actual number of active frames
        let mut frames = Vec::with_capacity(MAX_CALL_DEPTH);
        frames.resize_with(MAX_CALL_DEPTH, LuaCallFrame::default);

        let mut vm = LuaVM {
            global: TableId(0),                       // Will be initialized below
            registry: TableId(1),                     // Will be initialized below
            gc_roots_buffer: Vec::with_capacity(512), // Pre-allocate roots buffer
            frames,
            frame_count: 0,
            register_stack: Vec::with_capacity(256), // Pre-allocate for initial stack
            object_pool: ObjectPool::new(),
            gc: GC::new(),
            return_values: Vec::with_capacity(16),
            open_upvalues: Vec::new(),
            to_be_closed: Vec::new(),
            next_frame_id: 0,
            error_handler: None,
            #[cfg(feature = "loadlib")]
            ffi_state: crate::ffi::FFIState::new(),
            current_thread: None,
            current_thread_id: None,
            current_thread_value: None,
            main_thread_value: None, // Will be initialized lazily
            string_metatable: None,
            c_closure_upvalues_ptr: std::ptr::null(),
            c_closure_upvalues_len: 0,
            c_closure_inline_upvalue: LuaValue::nil(),
            #[cfg(feature = "async")]
            async_executor: AsyncExecutor::new(),
            // Initialize error storage
            error_message: String::new(),
            yield_values: Vec::new(),
        };

        // Initialize registry (like Lua's init_registry)
        // Registry is a GC root and protects all values stored in it
        let registry = vm.create_table(2, 8);
        if let Some(registry_id) = registry.as_table_id() {
            // Fix the registry table so it's never collected
            vm.object_pool.fix_table(registry_id);
            vm.registry = registry_id;
        }

        // Set _G to point to the global table itself
        let globals_value = vm.create_table(0, 20);
        if let Some(globals_id) = globals_value.as_table_id() {
            // Fix the global table so it's never collected
            vm.object_pool.fix_table(globals_id);
            vm.global = globals_id;
        }

        vm.set_global("_G", globals_value);
        vm.set_global("_ENV", globals_value);

        // Store globals in registry (like Lua's LUA_RIDX_GLOBALS)
        vm.registry_set_integer(1, globals_value);

        // Reset GC debt after initialization (like Lua's luaC_fullgc at start)
        // The objects created during initialization should not count towards the first GC
        vm.gc.gc_debt = -(8 * 1024);
        vm.gc.gc_estimate = vm.gc.total_bytes;

        vm
    }

    /// Set a value in the registry by integer key
    pub fn registry_set_integer(&mut self, key: i64, value: LuaValue) {
        if let Some(reg_table) = self.object_pool.get_table_mut(self.registry) {
            reg_table.set_int(key, value);
        }
    }

    /// Get a value from the registry by integer key
    pub fn registry_get_integer(&self, key: i64) -> Option<LuaValue> {
        if let Some(reg_table) = self.object_pool.get_table(self.registry) {
            return reg_table.get_int(key);
        }

        None
    }

    /// Set a value in the registry by string key
    pub fn registry_set(&mut self, key: &str, value: LuaValue) {
        let key_value = self.create_string(key);

        if let Some(reg_table) = self.object_pool.get_table_mut(self.registry) {
            reg_table.raw_set(key_value, value);
        }
    }

    /// Get a value from the registry by string key
    pub fn registry_get(&mut self, key: &str) -> Option<LuaValue> {
        let key = self.create_string(key);
        if let Some(reg_table) = self.object_pool.get_table(self.registry) {
            return reg_table.raw_get(&key);
        }

        None
    }

    #[inline(always)]
    fn set_register(&mut self, base_ptr: usize, reg: usize, value: LuaValue) {
        self.register_stack[base_ptr + reg] = value;
    }

    #[inline(always)]
    fn ensure_stack_capacity(&mut self, required: usize) {
        if self.register_stack.len() < required {
            self.register_stack.resize(required, LuaValue::nil());
        }
    }

    pub fn open_libs(&mut self) {
        let _ = lib_registry::create_standard_registry().load_all(self);

        // Register async functions
        #[cfg(feature = "async")]
        crate::stdlib::async_lib::register_async_functions(self);

        // Reset GC state after loading standard libraries
        // Like Lua's initial full GC after loading base libs
        self.gc.gc_debt = -(8 * 1024);
        self.gc.gc_estimate = self.gc.total_bytes;
    }

    /// Execute a chunk directly (convenience method)
    pub fn execute(&mut self, chunk: Rc<Chunk>) -> LuaResult<LuaValue> {
        // Register all constants in the chunk with GC
        self.register_chunk_constants(&chunk);

        // Create upvalue for _ENV (global table)
        // Main chunks in Lua 5.4 always have _ENV as upvalue[0]
        let env_upvalue_id = self.create_upvalue_closed(LuaValue::table(self.global));
        let upvalues = vec![env_upvalue_id];

        // Create main function in object pool with _ENV upvalue
        let main_func_value = self.create_function(chunk.clone(), upvalues);
        let func_id = main_func_value.as_function_id().unwrap();

        // Create initial call frame using unified stack
        let base_ptr = self.register_stack.len();
        let required_size = base_ptr + chunk.max_stack_size;
        self.ensure_stack_capacity(required_size);

        // Get code, constants, and upvalues pointers from chunk/function
        let code_ptr = chunk.code.as_ptr();
        let constants_ptr = chunk.constants.as_ptr();
        let upvalues_ptr = self
            .object_pool
            .get_function(func_id)
            .map(|f| f.upvalues.as_ptr())
            .unwrap_or(std::ptr::null());

        let frame = LuaCallFrame::new_lua_function(
            func_id,
            code_ptr,
            constants_ptr,
            upvalues_ptr,
            base_ptr,
            chunk.max_stack_size, // top
            0,                    // result_reg
            0,                    // nresults
            chunk.max_stack_size, // max_stack_size
        );

        self.push_frame(frame);

        // Execute
        let result = match self.run() {
            Ok(v) => v,
            Err(LuaError::Exit) => {
                // Normal exit - get the return value from return_values
                self.return_values
                    .first()
                    .cloned()
                    .unwrap_or(LuaValue::nil())
            }
            Err(e) => return Err(e),
        };

        // Clean up - clear stack used by this execution
        self.register_stack.clear();
        self.open_upvalues.clear();
        self.frame_count = 0;

        Ok(result)
    }

    /// Call a function value (for testing and runtime calls)
    pub fn call_function(&mut self, func: LuaValue, args: Vec<LuaValue>) -> LuaResult<LuaValue> {
        // Clear previous state
        self.register_stack.clear();
        self.frame_count = 0; // Reset frame count (frames Vec stays pre-allocated)
        self.open_upvalues.clear();

        // Get function from object pool
        let Some(func_id) = func.as_function_id() else {
            return Err(self.error("Not a function".to_string()));
        };

        // Clone chunk and get info before borrowing self mutably
        let (chunk, max_stack, code_ptr, constants_ptr, upvalues_ptr) = {
            let Some(func_ref) = self.object_pool.get_function(func_id) else {
                return Err(self.error("Invalid function ID".to_string()));
            };
            let chunk = func_ref.lua_chunk().clone();
            let max_stack = chunk.max_stack_size;
            let code_ptr = chunk.code.as_ptr();
            let constants_ptr = chunk.constants.as_ptr();
            let upvalues_ptr = func_ref.upvalues.as_ptr();
            (chunk, max_stack, code_ptr, constants_ptr, upvalues_ptr)
        };

        // Register chunk constants
        self.register_chunk_constants(&chunk);

        // Setup stack and frame
        let base_ptr = 0; // Start from beginning of cleared stack
        let required_size = max_stack; // Need at least max_stack registers

        // Initialize stack with nil values
        self.register_stack.resize(required_size, LuaValue::nil());

        // Copy arguments to registers
        for (i, arg) in args.iter().enumerate() {
            if i < max_stack {
                self.register_stack[base_ptr + i] = *arg;
            }
        }

        let frame = LuaCallFrame::new_lua_function(
            func_id,
            code_ptr,
            constants_ptr,
            upvalues_ptr,
            base_ptr,
            max_stack,
            0,
            0,
            max_stack,
        );

        self.push_frame(frame);

        // Execute
        let result = self.run()?;

        // Clean up
        self.frame_count = 0;

        Ok(result)
    }

    pub fn execute_string(&mut self, source: &str) -> LuaResult<LuaValue> {
        let chunk = self.compile(source)?;
        self.execute(Rc::new(chunk))
    }

    /// Compile source code using VM's string pool
    pub fn compile(&mut self, source: &str) -> LuaResult<Chunk> {
        let chunk = match compile_code(source, &mut self.object_pool) {
            Ok(c) => c,
            Err(e) => return Err(self.compile_error(e)),
        };

        Ok(chunk)
    }

    pub fn compile_with_name(&mut self, source: &str, chunk_name: &str) -> LuaResult<Chunk> {
        let chunk = match compile_code_with_name(source, &mut self.object_pool, chunk_name) {
            Ok(c) => c,
            Err(e) => return Err(self.compile_error(e)),
        };

        Ok(chunk)
    }

    /// Main execution loop - interprets bytecode instructions
    /// Lua 5.4 style: CALL pushes frame, RETURN pops frame, loop continues
    /// No recursion - pure state machine
    fn run(&mut self) -> LuaResult<LuaValue> {
        // Delegate to the optimized dispatcher loop
        execute::luavm_execute(self)
    }

    // ============ Frame Management (Lua 5.4 style) ============
    // Uses pre-allocated Vec for O(1) operations
    // Key optimization:
    // - Main VM: pre-filled to MAX_CALL_DEPTH for direct index access
    // - Coroutines: start small and grow on demand (like Lua's linked list CallInfo)

    /// Push a new frame onto the call stack and return stable pointer
    /// OPTIMIZED: Direct index write when capacity allows, grow on demand otherwise
    /// Returns None if call stack overflow (for pcall error handling)
    #[inline(always)]
    pub(crate) fn try_push_frame(&mut self, frame: LuaCallFrame) -> Option<*mut LuaCallFrame> {
        let idx = self.frame_count;
        if idx >= MAX_CALL_DEPTH {
            return None; // Stack overflow
        }

        // Fast path: direct write if Vec is pre-filled or has space
        if idx < self.frames.len() {
            self.frames[idx] = frame;
        } else {
            // Slow path: grow the Vec (for coroutines with on-demand allocation)
            self.frames.push(frame);
        }
        self.frame_count = idx + 1;
        Some(&mut self.frames[idx] as *mut LuaCallFrame)
    }

    /// Push a new frame onto the call stack and return stable pointer
    /// OPTIMIZED: Direct index write when capacity allows, grow on demand otherwise
    #[inline(always)]
    pub(crate) fn push_frame(&mut self, frame: LuaCallFrame) -> *mut LuaCallFrame {
        let idx = self.frame_count;
        assert!(idx < MAX_CALL_DEPTH, "call stack overflow");

        // Fast path: direct write if Vec is pre-filled or has space
        if idx < self.frames.len() {
            self.frames[idx] = frame;
        } else {
            // Slow path: grow the Vec (for coroutines with on-demand allocation)
            self.frames.push(frame);
        }
        self.frame_count = idx + 1;
        &mut self.frames[idx] as *mut LuaCallFrame
    }

    /// Pop frame without returning it
    /// ULTRA-OPTIMIZED: Just decrement counter
    #[inline(always)]
    pub(crate) fn pop_frame_discard(&mut self) {
        debug_assert!(self.frame_count > 0, "pop from empty call stack");
        self.frame_count -= 1;
        // Note: Don't truncate Vec - keep the capacity for reuse
        // The frame data will be overwritten on next push
    }

    /// Pop the current frame from the call stack
    #[inline(always)]
    #[allow(dead_code)]
    pub(crate) fn pop_frame(&mut self) -> Option<LuaCallFrame> {
        if self.frame_count > 0 {
            self.frame_count -= 1;
            Some(unsafe { std::ptr::read(self.frames.as_ptr().add(self.frame_count)) })
        } else {
            None
        }
    }

    /// Check if call stack is empty
    #[inline(always)]
    pub(crate) fn frames_is_empty(&self) -> bool {
        self.frame_count == 0
    }

    // Helper methods - direct Vec access with unsafe get_unchecked for hot path
    #[inline(always)]
    pub(crate) fn current_frame(&self) -> &LuaCallFrame {
        debug_assert!(self.frame_count > 0);
        unsafe { self.frames.get_unchecked(self.frame_count - 1) }
    }

    #[inline(always)]
    pub(crate) fn current_frame_mut(&mut self) -> &mut LuaCallFrame {
        debug_assert!(self.frame_count > 0);
        unsafe { self.frames.get_unchecked_mut(self.frame_count - 1) }
    }

    /// Get stable pointer to current frame (for execute loop)
    #[inline(always)]
    pub(crate) fn current_frame_ptr(&mut self) -> *mut LuaCallFrame {
        debug_assert!(self.frame_count > 0);
        unsafe { self.frames.as_mut_ptr().add(self.frame_count - 1) }
    }

    pub fn get_global(&mut self, name: &str) -> Option<LuaValue> {
        let key = self.create_string(name);

        let global = self.object_pool.get_table(self.global)?;
        global.raw_get(&key)
    }

    pub fn get_global_by_lua_value(&self, key: &LuaValue) -> Option<LuaValue> {
        let global = self.object_pool.get_table(self.global)?;
        global.raw_get(key)
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        let key = self.create_string(name);

        if let Some(global) = self.object_pool.get_table_mut(self.global) {
            global.raw_set(key.clone(), value.clone());
        }

        // Write barrier: global table (old) may now reference new object
        // self.gc
        //     .barrier_forward(crate::gc::GcObjectType::Table, self.global.0);
        // self.gc.barrier_back(&value);
    }

    pub fn set_global_by_lua_value(&mut self, key: &LuaValue, value: LuaValue) {
        if let Some(global) = self.object_pool.get_table_mut(self.global) {
            global.raw_set(key.clone(), value.clone());
        }

        // Write barrier
        // self.gc
        //     .barrier_forward(crate::gc::GcObjectType::Table, global_id.0);
        // self.gc.barrier_back(&value);
    }

    /// Set the metatable for all strings
    /// In Lua, all strings share a metatable with __index pointing to the string library
    pub fn set_string_metatable(&mut self, string_lib: LuaValue) {
        // Create the metatable
        let metatable = self.create_table(0, 1);

        // Use pre-cached __index StringId for fast lookup
        let index_key = LuaValue::string(self.object_pool.tm_index);

        // Get the table reference to set __index
        if let Some(mt_ref) = self.get_table_mut(&metatable) {
            // Set __index to the string library table
            mt_ref.raw_set(index_key, string_lib);
        }

        // Store the metatable as LuaValue (contains TableId)
        self.string_metatable = Some(metatable);
    }

    /// Get the shared string metatable
    pub fn get_string_metatable(&self) -> Option<LuaValue> {
        self.string_metatable.clone()
    }

    /// Get FFI state (immutable)
    #[cfg(feature = "loadlib")]
    pub fn get_ffi_state(&self) -> &crate::ffi::FFIState {
        &self.ffi_state
    }

    /// Get FFI state (mutable)
    #[cfg(feature = "loadlib")]
    pub fn get_ffi_state_mut(&mut self) -> &mut crate::ffi::FFIState {
        &mut self.ffi_state
    }

    // ============ Coroutine Support ============

    /// Initial call depth for coroutines (grows on demand, like Lua's linked list CallInfo)
    const INITIAL_COROUTINE_CALL_DEPTH: usize = 4;

    /// Create a new thread (coroutine) - returns ThreadId-based LuaValue
    /// OPTIMIZED: Minimal initial allocations - grows on demand
    pub fn create_thread_value(&mut self, func: LuaValue) -> LuaValue {
        // Only allocate capacity, don't pre-fill (unlike main VM)
        // Coroutines typically have shallow call stacks, so we grow on demand
        let frames = Vec::with_capacity(Self::INITIAL_COROUTINE_CALL_DEPTH);

        // Start with smaller register stack - grows on demand
        let mut register_stack = Vec::with_capacity(64);
        register_stack.push(func);

        let thread = LuaThread {
            status: CoroutineStatus::Suspended,
            frames,
            frame_count: 0,
            register_stack,
            return_values: Vec::new(),
            open_upvalues: Vec::new(),
            next_frame_id: 0,
            error_handler: None,
            yield_values: Vec::new(),
            resume_values: Vec::new(),
            yield_call_reg: None,
            yield_call_nret: None,
            yield_pc: None,
            yield_frame_id: None,
        };

        // Create thread in ObjectPool and return LuaValue
        let thread_id = self.object_pool.create_thread(thread);
        LuaValue::thread(thread_id)
    }

    /// Create a new thread (coroutine) - legacy version returning Rc<RefCell<>>
    /// This is still needed for internal VM state tracking (current_thread)
    pub fn create_thread(&mut self, func: LuaValue) -> Rc<RefCell<LuaThread>> {
        // Only allocate capacity, don't pre-fill (unlike main VM)
        // Coroutines typically have shallow call stacks, so we grow on demand
        let frames = Vec::with_capacity(Self::INITIAL_COROUTINE_CALL_DEPTH);

        let thread = LuaThread {
            status: CoroutineStatus::Suspended,
            frames,
            frame_count: 0,
            register_stack: Vec::with_capacity(256),
            return_values: Vec::new(),
            open_upvalues: Vec::new(),
            next_frame_id: 0,
            error_handler: None,
            yield_values: Vec::new(),
            resume_values: Vec::new(),
            yield_call_reg: None,
            yield_call_nret: None,
            yield_pc: None,
            yield_frame_id: None,
        };

        let thread_rc = Rc::new(RefCell::new(thread));

        // Store the function in the thread's first register
        thread_rc.borrow_mut().register_stack.push(func);

        thread_rc
    }

    /// Resume a coroutine using ThreadId-based LuaValue
    /// ULTRA-OPTIMIZED: Minimized object_pool lookups using raw pointers
    pub fn resume_thread(
        &mut self,
        thread_val: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Get ThreadId from LuaValue
        let Some(thread_id) = thread_val.as_thread_id() else {
            return Err(self.error("invalid thread".to_string()));
        };

        // OPTIMIZATION: Get thread pointer once and reuse
        let thread_ptr: *mut LuaThread = {
            let Some(thread) = self.object_pool.get_thread_mut(thread_id) else {
                return Err(self.error("invalid thread".to_string()));
            };

            // Check status
            match thread.status {
                CoroutineStatus::Dead => {
                    return Ok((
                        false,
                        vec![self.create_string("cannot resume dead coroutine")],
                    ));
                }
                CoroutineStatus::Running => {
                    return Ok((
                        false,
                        vec![self.create_string("cannot resume running coroutine")],
                    ));
                }
                _ => {}
            }

            thread.status = CoroutineStatus::Running;
            thread as *mut _
        };

        // SAFETY: thread_ptr is valid for the duration of this function
        // as we don't modify the object_pool's thread storage
        let is_first_resume = unsafe { (*thread_ptr).frame_count == 0 };

        // Handle first resume upvalue closing (only when needed)
        // This is required to properly capture upvalues from the parent scope
        if is_first_resume {
            let func = unsafe { (&(*thread_ptr).register_stack).get(0).cloned() };
            if let Some(func) = func {
                if let Some(func_id) = func.as_function_id() {
                    self.close_function_upvalues_for_thread(func_id);
                }
            }
        }

        // Swap state between VM and thread (O(1) pointer swaps)
        unsafe {
            std::mem::swap(&mut self.frames, &mut (*thread_ptr).frames);
            std::mem::swap(&mut self.register_stack, &mut (*thread_ptr).register_stack);
            std::mem::swap(&mut self.return_values, &mut (*thread_ptr).return_values);
            std::mem::swap(&mut self.open_upvalues, &mut (*thread_ptr).open_upvalues);
            std::mem::swap(&mut self.frame_count, &mut (*thread_ptr).frame_count);
            std::mem::swap(&mut self.next_frame_id, &mut (*thread_ptr).next_frame_id);
        }

        // Save and set thread tracking
        let saved_thread = self.current_thread.take();
        let saved_thread_id = self.current_thread_id.take();
        self.current_thread_id = Some(thread_id);
        self.current_thread_value = Some(thread_val.clone());

        // Execute
        let result = if is_first_resume {
            let func = self
                .register_stack
                .get(0)
                .cloned()
                .unwrap_or(LuaValue::nil());
            match self.call_function_internal(func, args) {
                Ok(values) => Ok(values),
                Err(LuaError::Yield) => Ok(self.take_yield_values()),
                Err(e) => Err(e),
            }
        } else {
            // Resumed from yield: handle return values
            let call_reg = unsafe { (*thread_ptr).yield_call_reg };
            let call_nret = unsafe { (*thread_ptr).yield_call_nret };

            if let (Some(a), Some(num_expected)) = (call_reg, call_nret) {
                if self.frame_count > 0 {
                    let frame = &self.frames[self.frame_count - 1];
                    let base_ptr = frame.base_ptr as usize;
                    let top = frame.top as usize;

                    let num_returns = args.len();
                    let n = if num_expected == usize::MAX {
                        num_returns
                    } else {
                        num_expected.min(num_returns)
                    };

                    for (i, value) in args.iter().take(n).enumerate() {
                        if base_ptr + a + i < self.register_stack.len() && a + i < top {
                            self.register_stack[base_ptr + a + i] = *value;
                        }
                    }
                    for i in num_returns..num_expected.min(top.saturating_sub(a)) {
                        if base_ptr + a + i < self.register_stack.len() {
                            self.register_stack[base_ptr + a + i] = LuaValue::nil();
                        }
                    }
                }

                unsafe {
                    (*thread_ptr).yield_call_reg = None;
                    (*thread_ptr).yield_call_nret = None;
                }
            }

            self.return_values = args;

            match self.run() {
                Ok(_) => Ok(std::mem::take(&mut self.return_values)),
                Err(LuaError::Yield) => Ok(self.take_yield_values()),
                Err(e) => Err(e),
            }
        };

        // Check if thread yielded
        let did_yield = matches!(&result, Ok(_) if self.frame_count > 0);

        // Swap state back to thread
        unsafe {
            std::mem::swap(&mut self.frames, &mut (*thread_ptr).frames);
            std::mem::swap(&mut self.register_stack, &mut (*thread_ptr).register_stack);
            std::mem::swap(&mut self.return_values, &mut (*thread_ptr).return_values);
            std::mem::swap(&mut self.open_upvalues, &mut (*thread_ptr).open_upvalues);
            std::mem::swap(&mut self.frame_count, &mut (*thread_ptr).frame_count);
            std::mem::swap(&mut self.next_frame_id, &mut (*thread_ptr).next_frame_id);
        }

        // Finalize result
        let final_result = if did_yield {
            unsafe {
                (*thread_ptr).status = CoroutineStatus::Suspended;
                let values = std::mem::take(&mut (*thread_ptr).yield_values);
                Ok((true, values))
            }
        } else {
            match result {
                Ok(values) => {
                    unsafe { (*thread_ptr).status = CoroutineStatus::Dead };
                    Ok((true, values))
                }
                Err(LuaError::Exit) => {
                    unsafe { (*thread_ptr).status = CoroutineStatus::Dead };
                    let values = unsafe { std::mem::take(&mut (*thread_ptr).return_values) };
                    Ok((true, values))
                }
                Err(_) => {
                    unsafe { (*thread_ptr).status = CoroutineStatus::Dead };
                    let error_msg = self.get_error_message().to_string();
                    Ok((false, vec![self.create_string(&error_msg)]))
                }
            }
        };

        // Restore thread tracking
        self.current_thread = saved_thread;
        self.current_thread_id = saved_thread_id;
        self.current_thread_value = None;

        final_result
    }

    /// Helper: close upvalues for a function being resumed in a coroutine
    /// OPTIMIZED: Skip if function has no upvalues, avoid clone
    #[inline(always)]
    fn close_function_upvalues_for_thread(&mut self, func_id: FunctionId) {
        // First check: does this function have any upvalues?
        let upvalue_count = {
            if let Some(func_ref) = self.object_pool.get_function(func_id) {
                func_ref.upvalues.len()
            } else {
                return;
            }
        };

        // Fast path: no upvalues, nothing to close
        if upvalue_count == 0 {
            return;
        }

        // Clone only if we have upvalues to process
        let upvalue_ids: Vec<UpvalueId> = {
            if let Some(func_ref) = self.object_pool.get_function(func_id) {
                func_ref.upvalues.clone()
            } else {
                return;
            }
        };

        for uv_id in upvalue_ids {
            if let Some(uv) = self.object_pool.get_upvalue(uv_id) {
                if let Some(stack_idx) = uv.get_stack_index() {
                    let value = if stack_idx < self.register_stack.len() {
                        self.register_stack[stack_idx]
                    } else {
                        LuaValue::nil()
                    };
                    if let Some(uv_mut) = self.object_pool.get_upvalue_mut(uv_id) {
                        uv_mut.close(value);
                    }
                }
            }
        }
    }

    /// Yield from current coroutine
    /// Returns Err(LuaError::Yield) which will be caught by run() loop
    pub fn yield_thread(&mut self, values: Vec<LuaValue>) -> LuaResult<()> {
        if let Some(thread_id) = self.current_thread_id {
            // Store yield values in the thread
            if let Some(thread) = self.object_pool.get_thread_mut(thread_id) {
                // Avoid clone - move directly
                thread.yield_values = values;
                thread.status = CoroutineStatus::Suspended;
            }
            // Return Yield "error" to unwind the call stack
            Err(LuaError::Yield)
        } else {
            Err(self.error("attempt to yield from outside a coroutine".to_string()))
        }
    }

    /// Fast table get - NO metatable support!
    /// Use this for normal field access (GETFIELD, GETTABLE, GETI)
    /// This is the correct behavior for Lua bytecode instructions
    /// Only use table_get_with_meta when you explicitly need __index metamethod
    #[inline(always)]
    pub fn table_get(&self, lua_table_value: &LuaValue, key: &LuaValue) -> LuaValue {
        // ObjectPool lookup
        if let Some(table_id) = lua_table_value.as_table_id() {
            if let Some(table) = self.object_pool.get_table(table_id) {
                // Fast path for integer keys
                if let Some(i) = key.as_integer() {
                    if i > 0 {
                        let idx = (i - 1) as usize;
                        if idx < table.array.len() {
                            let val = unsafe { table.array.get_unchecked(idx) };
                            if !val.is_nil() {
                                return *val;
                            }
                        }
                    }
                }

                // Hash part lookup - use table's get_from_hash method
                if let Some(val) = table.get_from_hash(key) {
                    return val;
                }

                return table.raw_get(key).unwrap_or(LuaValue::nil());
            }
        }

        LuaValue::nil()
    }

    /// Get value from table with metatable support (__index metamethod)
    /// Use this for GETTABLE, GETFIELD, GETI instructions
    /// For raw access without metamethods, use table_get_raw() instead
    pub fn table_get_with_meta(
        &mut self,
        lua_table_value: &LuaValue,
        key: &LuaValue,
    ) -> Option<LuaValue> {
        // Handle strings with metatable support
        if lua_table_value.is_string() {
            // Strings use a shared metatable
            if let Some(string_mt) = self.get_string_metatable() {
                // Use pre-cached __index StringId for fast lookup
                let index_key = LuaValue::string(self.object_pool.tm_index);

                // Get the __index field from string metatable
                if let Some(index_table) = self.table_get_with_meta(&string_mt, &index_key) {
                    // Look up the key in the __index table (the string library)
                    return self.table_get_with_meta(&index_table, key);
                }
            }
            return None;
        }

        // Use ObjectPool lookup
        let Some(table_id) = lua_table_value.as_table_id() else {
            return None;
        };

        // First try raw get
        let (value, meta_value) = {
            let table = self.object_pool.get_table(table_id)?;
            let val = table.raw_get(key).unwrap_or(LuaValue::nil());
            let meta = table.get_metatable();
            (val, meta)
        };

        if !value.is_nil() {
            return Some(value);
        }

        // Check for __index metamethod
        if let Some(mt) = meta_value
            && let Some(meta_id) = mt.as_table_id()
        {
            // Use pre-cached __index StringId - avoids hash computation and intern lookup
            let index_key = LuaValue::string(self.object_pool.tm_index);

            // Single table lookup: check tm_flags AND get __index value together
            let index_value = {
                let metatable = self.object_pool.get_table(meta_id)?;
                // FAST PATH: Check tm_flags first (like Lua 5.4's fasttm)
                // If flag is set, __index is known to be absent - skip lookup
                if metatable.tm_absent(crate::lua_value::tm_flags::TM_INDEX) {
                    return None;
                }
                metatable.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    LuaValueKind::Table => {
                        return self.table_get_with_meta(&index_val, key);
                    }
                    // Fast path for CFunction __index
                    LuaValueKind::CFunction => {
                        if let Some(cfunc) = index_val.as_cfunction() {
                            match self.call_cfunc_metamethod_2(cfunc, *lua_table_value, *key) {
                                Ok(result) => return result,
                                Err(_) => return None,
                            }
                        }
                    }
                    LuaValueKind::Function => {
                        let args = [*lua_table_value, *key];
                        match self.call_metamethod(&index_val, &args) {
                            Ok(result) => return result,
                            Err(_) => return None,
                        }
                    }
                    _ => {}
                }
            } else {
                // __index not found - cache this fact for future lookups
                if let Some(metatable) = self.object_pool.get_table_mut(meta_id) {
                    metatable.set_tm_absent(tm_flags::TM_INDEX);
                }
            }
        }

        None
    }

    /// Get value from userdata with metatable support
    /// Handles __index metamethod
    pub fn userdata_get(
        &mut self,
        lua_userdata_value: &LuaValue,
        key: &LuaValue,
    ) -> Option<LuaValue> {
        let Some(userdata_id) = lua_userdata_value.as_userdata_id() else {
            return None;
        };

        // Get metatable from userdata
        let metatable = {
            let userdata = self.object_pool.get_userdata(userdata_id)?;
            userdata.get_metatable()
        };

        if let Some(mt_id) = metatable.as_table_id() {
            // Use pre-cached __index StringId
            let index_key = LuaValue::string(self.object_pool.tm_index);

            let index_value = {
                let mt = self.object_pool.get_table(mt_id)?;
                mt.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table
                    LuaValueKind::Table => return self.table_get_with_meta(&index_val, key),

                    // Fast path for CFunction __index
                    LuaValueKind::CFunction => {
                        if let Some(cfunc) = index_val.as_cfunction() {
                            match self.call_cfunc_metamethod_2(cfunc, *lua_userdata_value, *key) {
                                Ok(result) => return result,
                                Err(_) => return None,
                            }
                        }
                    }
                    // Lua function - use slower path
                    LuaValueKind::Function => {
                        let args = [*lua_userdata_value, *key];
                        match self.call_metamethod(&index_val, &args) {
                            Ok(result) => return result,
                            Err(_) => return None,
                        }
                    }
                    _ => {}
                }
            }
        }

        None
    }

    /// Get value from string with metatable support
    /// Handles __index metamethod for strings
    pub fn string_get(&mut self, string_val: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        // Use pre-cached __index StringId
        let index_key = LuaValue::string(self.object_pool.tm_index);
        // Check for __index metamethod in string metatable
        if let Some(mt) = &self.string_metatable.clone() {
            let index_value = if let Some(mt_ref) = self.get_table(mt) {
                mt_ref.raw_get(&index_key)
            } else {
                None
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table (this is the common case for strings)
                    LuaValueKind::Table => return self.table_get_with_meta(&index_val, key),
                    // Fast path for CFunction __index
                    LuaValueKind::CFunction => {
                        if let Some(cfunc) = index_val.as_cfunction() {
                            match self.call_cfunc_metamethod_2(cfunc, *string_val, *key) {
                                Ok(result) => return result,
                                Err(_) => return None,
                            }
                        }
                    }
                    // Lua function - slower path
                    LuaValueKind::Function => {
                        let args = [*string_val, *key];
                        match self.call_metamethod(&index_val, &args) {
                            Ok(result) => return result,
                            Err(_) => return None,
                        }
                    }
                    _ => {}
                }
            }
        }

        None
    }

    /// Set value in table with metatable support (__newindex metamethod)
    /// Use this for SETTABLE, SETFIELD, SETI instructions
    /// For raw set without metamethods, use table_set_raw() instead
    pub fn table_set_with_meta(
        &mut self,
        lua_table_val: LuaValue,
        key: LuaValue,
        value: LuaValue,
    ) -> LuaResult<()> {
        // Use ObjectPool lookup
        let Some(table_id) = lua_table_val.as_table_id() else {
            return Err(self.error("table_set: not a table".to_string()));
        };

        // Check if key already exists and get metatable info
        let (has_key, meta_value) = {
            let Some(table) = self.object_pool.get_table(table_id) else {
                return Err(self.error("invalid table".to_string()));
            };
            let has_k = table.raw_get(&key).map(|v| !v.is_nil()).unwrap_or(false);
            let meta = table.get_metatable();
            (has_k, meta)
        };

        // If key exists, just do raw set (no metamethod check needed)
        if has_key {
            if let Some(table) = self.object_pool.get_table_mut(table_id) {
                table.raw_set(key.clone(), value.clone());
            }
            self.gc
                .barrier_forward(crate::gc::GcObjectType::Table, table_id.0);
            self.gc.barrier_back(&value);
            return Ok(());
        }

        // Key doesn't exist, check for __newindex metamethod
        if let Some(mt) = meta_value
            && let Some(mt_id) = mt.as_table_id()
        {
            // Use pre-cached __newindex StringId - avoids hash computation and intern lookup
            let newindex_key = LuaValue::string(self.object_pool.tm_newindex);

            let newindex_value = {
                let Some(metatable) = self.object_pool.get_table(mt_id) else {
                    return Err(self.error("missing metatable".to_string()));
                };
                metatable.raw_get(&newindex_key)
            };

            if let Some(newindex_val) = newindex_value {
                match newindex_val.kind() {
                    LuaValueKind::Table => {
                        return self.table_set_with_meta(newindex_val, key, value);
                    }
                    // Fast path for CFunction __newindex
                    LuaValueKind::CFunction => {
                        if let Some(cfunc) = newindex_val.as_cfunction() {
                            match self.call_cfunc_metamethod_3(cfunc, lua_table_val, key, value) {
                                Ok(_) => return Ok(()),
                                Err(e) => return Err(e),
                            }
                        }
                    }
                    // Lua function - slower path
                    LuaValueKind::Function => {
                        let args = [lua_table_val, key, value];
                        match self.call_metamethod(&newindex_val, &args) {
                            Ok(_) => return Ok(()),
                            Err(e) => return Err(e),
                        }
                    }
                    _ => {}
                }
            }
        }

        // No metamethod, use raw set
        if let Some(table) = self.object_pool.get_table_mut(table_id) {
            table.raw_set(key.clone(), value.clone());
        }

        // Write barrier for new insertion
        self.gc
            .barrier_forward(crate::gc::GcObjectType::Table, table_id.0);
        self.gc.barrier_back(&value);
        Ok(())
    }

    /// Call a Lua value (function or CFunction) with the given arguments
    /// Returns the first return value, or None if the call fails
    pub fn call_metamethod(
        &mut self,
        func: &LuaValue,
        args: &[LuaValue],
    ) -> LuaResult<Option<LuaValue>> {
        // Fast path for CFunction
        if let Some(cfunc) = func.as_cfunction() {
            match args.len() {
                1 => return self.call_cfunc_metamethod_1(cfunc, args[0]),
                2 => return self.call_cfunc_metamethod_2(cfunc, args[0], args[1]),
                3 => return self.call_cfunc_metamethod_3(cfunc, args[0], args[1], args[2]),
                _ => {}
            }
        }

        // Fast path for Lua functions with 1-2 args (common case)
        if let Some(func_id) = func.as_function_id() {
            match args.len() {
                1 => return self.call_lua_metamethod_1(func_id, args[0]),
                2 => return self.call_lua_metamethod_2(func_id, args[0], args[1]),
                _ => {}
            }
        }

        // Slow path for general cases
        let result = self.call_function_internal(func.clone(), args.to_vec())?;
        Ok(result.get(0).cloned())
    }

    /// ULTRA-OPTIMIZED: Call Lua function metamethod with 2 args
    /// Used by __index, __eq, __lt, __le, etc.
    /// Zero Vec allocation - copies args directly to stack
    #[inline]
    fn call_lua_metamethod_2(
        &mut self,
        func_id: FunctionId,
        arg1: LuaValue,
        arg2: LuaValue,
    ) -> LuaResult<Option<LuaValue>> {
        let (max_stack_size, code_ptr, constants_ptr, upvalues_ptr) = {
            let Some(func_ref) = self.object_pool.get_function(func_id) else {
                return Err(self.error("Invalid function".to_string()));
            };
            let chunk = func_ref.lua_chunk();
            let size = chunk.max_stack_size.max(2);
            (
                size,
                chunk.code.as_ptr(),
                chunk.constants.as_ptr(),
                func_ref.upvalues.as_ptr(),
            )
        };

        let new_base = if self.frame_count > 0 {
            let current_frame = &self.frames[self.frame_count - 1];
            (current_frame.base_ptr as usize) + 256
        } else {
            0
        };

        self.ensure_stack_capacity(new_base + max_stack_size);

        // Set up args directly - no Vec allocation
        unsafe {
            let dst = self.register_stack.as_mut_ptr().add(new_base);
            *dst = arg1;
            *dst.add(1) = arg2;
            // Initialize remaining with nil
            let nil_val = LuaValue::nil();
            for i in 2..max_stack_size {
                *dst.add(i) = nil_val;
            }
        }

        // Push boundary + Lua frame
        let boundary_frame = LuaCallFrame::new_c_function(new_base, 0);
        self.push_frame(boundary_frame);

        let new_frame = LuaCallFrame::new_lua_function(
            func_id,
            code_ptr,
            constants_ptr,
            upvalues_ptr,
            new_base,
            max_stack_size,
            0,
            -1,
            max_stack_size,
        );
        self.push_frame(new_frame);

        let exec_result = execute::luavm_execute(self);

        match exec_result {
            Ok(_) | Err(LuaError::Exit) => {
                self.pop_frame_discard();
                Ok(self.return_values.first().cloned())
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(e) => {
                self.pop_frame_discard();
                Err(e)
            }
        }
    }

    /// ULTRA-OPTIMIZED: Call Lua function metamethod with 1 arg
    /// Used by __len, __unm, __bnot, __tostring
    #[inline]
    fn call_lua_metamethod_1(
        &mut self,
        func_id: FunctionId,
        arg1: LuaValue,
    ) -> LuaResult<Option<LuaValue>> {
        let (max_stack_size, code_ptr, constants_ptr, upvalues_ptr) = {
            let Some(func_ref) = self.object_pool.get_function(func_id) else {
                return Err(self.error("Invalid function".to_string()));
            };
            let chunk = func_ref.lua_chunk();
            let size = chunk.max_stack_size.max(1);
            (
                size,
                chunk.code.as_ptr(),
                chunk.constants.as_ptr(),
                func_ref.upvalues.as_ptr(),
            )
        };

        let new_base = if self.frame_count > 0 {
            let current_frame = &self.frames[self.frame_count - 1];
            (current_frame.base_ptr as usize) + 256
        } else {
            0
        };

        self.ensure_stack_capacity(new_base + max_stack_size);

        unsafe {
            let dst = self.register_stack.as_mut_ptr().add(new_base);
            *dst = arg1;
            let nil_val = LuaValue::nil();
            for i in 1..max_stack_size {
                *dst.add(i) = nil_val;
            }
        }

        let boundary_frame = LuaCallFrame::new_c_function(new_base, 0);
        self.push_frame(boundary_frame);

        let new_frame = LuaCallFrame::new_lua_function(
            func_id,
            code_ptr,
            constants_ptr,
            upvalues_ptr,
            new_base,
            max_stack_size,
            0,
            -1,
            max_stack_size,
        );
        self.push_frame(new_frame);

        let exec_result = execute::luavm_execute(self);

        match exec_result {
            Ok(_) | Err(LuaError::Exit) => {
                self.pop_frame_discard();
                Ok(self.return_values.first().cloned())
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(e) => {
                self.pop_frame_discard();
                Err(e)
            }
        }
    }

    /// Fast path for calling CFunction metamethods with 2 arguments
    /// Used by __index, __newindex, etc. Avoids Vec allocation.
    /// Returns the first return value.
    /// OPTIMIZED: Skip expensive get_function lookup by using a fixed offset from current base
    #[inline(always)]
    pub fn call_cfunc_metamethod_2(
        &mut self,
        cfunc: crate::lua_value::CFunction,
        arg1: LuaValue,
        arg2: LuaValue,
    ) -> LuaResult<Option<LuaValue>> {
        // Fast path: use a fixed offset from current base (256 slots is enough for most cases)
        // This avoids the expensive object_pool.get_function lookup
        let new_base = if self.frame_count > 0 {
            let current_frame = &self.frames[self.frame_count - 1];
            // Use top as the base for nested calls, since all args are already there
            // Adding 256 ensures we don't overwrite the caller's stack
            (current_frame.base_ptr as usize) + 256
        } else {
            0
        };

        let stack_size = 3; // func + 2 args
        self.ensure_stack_capacity(new_base + stack_size);

        // Set up arguments directly (no Vec allocation)
        unsafe {
            let base = self.register_stack.as_mut_ptr().add(new_base);
            *base = LuaValue::cfunction(cfunc);
            *base.add(1) = arg1;
            *base.add(2) = arg2;
        }

        // Create C function frame
        let temp_frame = LuaCallFrame::new_c_function(new_base, stack_size);
        self.push_frame(temp_frame);

        // Call CFunction
        let result = match cfunc(self) {
            Ok(r) => {
                self.pop_frame_discard();
                Ok(r.first())
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(e) => {
                self.pop_frame_discard();
                Err(e)
            }
        };

        result
    }

    /// Fast path for calling CFunction metamethods with 1 argument
    /// Used by __len, __unm, __bnot, etc. Avoids Vec allocation.
    /// OPTIMIZED: Skip expensive get_function lookup
    #[inline(always)]
    pub fn call_cfunc_metamethod_1(
        &mut self,
        cfunc: crate::lua_value::CFunction,
        arg1: LuaValue,
    ) -> LuaResult<Option<LuaValue>> {
        let new_base = if self.frame_count > 0 {
            let current_frame = &self.frames[self.frame_count - 1];
            (current_frame.base_ptr as usize) + 256
        } else {
            0
        };

        let stack_size = 2; // func + 1 arg
        self.ensure_stack_capacity(new_base + stack_size);

        unsafe {
            let base = self.register_stack.as_mut_ptr().add(new_base);
            *base = LuaValue::cfunction(cfunc);
            *base.add(1) = arg1;
        }

        let temp_frame = LuaCallFrame::new_c_function(new_base, stack_size);
        self.push_frame(temp_frame);

        let result = match cfunc(self) {
            Ok(r) => {
                self.pop_frame_discard();
                Ok(r.first())
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(e) => {
                self.pop_frame_discard();
                Err(e)
            }
        };

        result
    }

    /// Fast path for calling CFunction metamethods with 3 arguments
    /// Used by __newindex. Avoids Vec allocation.
    /// OPTIMIZED: Skip expensive get_function lookup
    #[inline(always)]
    pub fn call_cfunc_metamethod_3(
        &mut self,
        cfunc: crate::lua_value::CFunction,
        arg1: LuaValue,
        arg2: LuaValue,
        arg3: LuaValue,
    ) -> LuaResult<Option<LuaValue>> {
        let new_base = if self.frame_count > 0 {
            let current_frame = &self.frames[self.frame_count - 1];
            (current_frame.base_ptr as usize) + 256
        } else {
            0
        };

        let stack_size = 4; // func + 3 args
        self.ensure_stack_capacity(new_base + stack_size);

        unsafe {
            let base = self.register_stack.as_mut_ptr().add(new_base);
            *base = LuaValue::cfunction(cfunc);
            *base.add(1) = arg1;
            *base.add(2) = arg2;
            *base.add(3) = arg3;
        }

        let temp_frame = LuaCallFrame::new_c_function(new_base, stack_size);
        self.push_frame(temp_frame);

        let result = match cfunc(self) {
            Ok(r) => {
                self.pop_frame_discard();
                Ok(r.first())
            }
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(e) => {
                self.pop_frame_discard();
                Err(e)
            }
        };

        result
    }

    // Integer division

    /// Close all open upvalues for a specific stack range
    /// Called when a frame exits to move values from stack to heap
    /// Now uses absolute stack indices instead of frame_id
    #[allow(dead_code)]
    fn close_upvalues_in_range(&mut self, start_idx: usize, end_idx: usize) {
        // Find all open upvalues pointing to this range
        let upvalues_to_close: Vec<UpvalueId> = self
            .open_upvalues
            .iter()
            .filter(|uv_id| {
                if let Some(uv) = self.object_pool.get_upvalue(**uv_id) {
                    if let Some(stack_idx) = uv.get_stack_index() {
                        return stack_idx >= start_idx && stack_idx < end_idx;
                    }
                }
                false
            })
            .cloned()
            .collect();

        // Close each upvalue
        for uv_id in upvalues_to_close.iter() {
            // Get the value from the stack before closing
            if let Some(uv) = self.object_pool.get_upvalue(*uv_id) {
                if let Some(stack_idx) = uv.get_stack_index() {
                    let value = if stack_idx < self.register_stack.len() {
                        self.register_stack[stack_idx]
                    } else {
                        LuaValue::nil()
                    };
                    if let Some(uv_mut) = self.object_pool.get_upvalue_mut(*uv_id) {
                        uv_mut.close(value);
                    }
                }
            }
        }

        // Remove closed upvalues from the open list
        self.open_upvalues.retain(|uv_id| {
            self.object_pool
                .get_upvalue(*uv_id)
                .map(|uv| uv.is_open())
                .unwrap_or(false)
        });
    }

    /// Helper: Get value from stack for an open upvalue (using absolute index)
    fn get_upvalue_value_at(&self, stack_idx: usize) -> LuaValue {
        if stack_idx < self.register_stack.len() {
            self.register_stack[stack_idx]
        } else {
            LuaValue::nil()
        }
    }

    /// Get upvalue value by UpvalueId
    /// For open upvalues, reads from register stack
    /// For closed upvalues, returns the stored value
    pub fn read_upvalue(&self, uv_id: UpvalueId) -> LuaValue {
        if let Some(uv) = self.object_pool.get_upvalue(uv_id) {
            if let Some(stack_idx) = uv.get_stack_index() {
                // Open upvalue - read from stack
                if stack_idx < self.register_stack.len() {
                    return self.register_stack[stack_idx];
                }
                LuaValue::nil()
            } else if let Some(value) = uv.get_closed_value() {
                // Closed upvalue - return stored value
                value
            } else {
                LuaValue::nil()
            }
        } else {
            LuaValue::nil()
        }
    }

    /// Fast path for reading upvalue - optimized with branch prediction hint
    /// SAFETY: uv_id must be valid
    #[inline(always)]
    pub unsafe fn read_upvalue_unchecked(&self, uv_id: UpvalueId) -> LuaValue {
        unsafe {
            let uv = self.object_pool.get_upvalue_unchecked(uv_id);
            // Use likely hint: open upvalues are more common in hot paths
            if uv.is_open {
                *self.register_stack.get_unchecked(uv.stack_index)
            } else {
                uv.closed_value
            }
        }
    }

    /// Set upvalue value by UpvalueId
    /// For open upvalues, writes to register stack
    /// For closed upvalues, updates the stored value
    pub fn write_upvalue(&mut self, uv_id: UpvalueId, value: LuaValue) {
        if let Some(uv) = self.object_pool.get_upvalue(uv_id) {
            if let Some(stack_idx) = uv.get_stack_index() {
                // Open upvalue - write to stack
                if stack_idx < self.register_stack.len() {
                    self.register_stack[stack_idx] = value;
                }
            } else {
                // Closed upvalue - update stored value
                if let Some(uv_mut) = self.object_pool.get_upvalue_mut(uv_id) {
                    uv_mut.closed_value = value;
                }
            }
        }
    }

    /// Fast path for writing upvalue - optimized with branch prediction hint
    /// SAFETY: uv_id must be valid
    #[inline(always)]
    pub unsafe fn write_upvalue_unchecked(&mut self, uv_id: UpvalueId, value: LuaValue) {
        unsafe {
            let uv = self.object_pool.get_upvalue_mut_unchecked(uv_id);
            // Use likely hint: open upvalues are more common in hot paths
            if uv.is_open {
                *self.register_stack.get_unchecked_mut(uv.stack_index) = value;
            } else {
                uv.closed_value = value;
            }
        }
    }

    /// Close all open upvalues at or above the given stack position
    /// Used by RETURN (k bit) and CLOSE instructions
    /// Simplified: uses absolute stack indices directly
    pub fn close_upvalues_from(&mut self, stack_pos: usize) {
        let upvalues_to_close: Vec<UpvalueId> = self
            .open_upvalues
            .iter()
            .filter(|uv_id| {
                if let Some(uv) = self.object_pool.get_upvalue(**uv_id) {
                    // Check if this upvalue points to stack_pos or higher
                    if let Some(stack_idx) = uv.get_stack_index() {
                        return stack_idx >= stack_pos;
                    }
                }
                false
            })
            .cloned()
            .collect();

        // Close each upvalue
        for uv_id in upvalues_to_close.iter() {
            if let Some(uv) = self.object_pool.get_upvalue(*uv_id) {
                if let Some(stack_idx) = uv.get_stack_index() {
                    let value = self.get_upvalue_value_at(stack_idx);
                    if let Some(uv_mut) = self.object_pool.get_upvalue_mut(*uv_id) {
                        uv_mut.close(value);
                    }
                }
            }
        }

        // Remove closed upvalues from the open list
        self.open_upvalues.retain(|uv_id| {
            self.object_pool
                .get_upvalue(*uv_id)
                .map(|uv| uv.is_open())
                .unwrap_or(false)
        });
    }

    /// Call __close metamethods for to-be-closed variables >= stack_pos
    pub fn close_to_be_closed(&mut self, stack_pos: usize) -> LuaResult<()> {
        // Process in reverse order (LIFO - last marked is closed first)
        while let Some(&(reg_idx, value)) = self.to_be_closed.last() {
            if reg_idx < stack_pos {
                break;
            }

            self.to_be_closed.pop();

            // Skip nil values
            if value.is_nil() {
                continue;
            }

            // Try to get __close metamethod using pre-cached StringId
            let close_key = LuaValue::string(self.object_pool.tm_close);
            let metamethod = if let Some(mt) = self.table_get_metatable(&value) {
                self.table_get_with_meta(&mt, &close_key)
            } else {
                None
            };

            if let Some(mm) = metamethod {
                if !mm.is_nil() {
                    // Call __close(value, error)
                    // error is nil in normal close, contains error object during unwinding
                    let args = vec![value, LuaValue::nil()];
                    // Ignore errors from __close to prevent infinite loops
                    let _ = self.call_metamethod(&mm, &args);
                }
            }
        }
        Ok(())
    }

    /// Create a new table and register it with GC
    /// Create a string and register it with GC
    /// For short strings (�?4 bytes), use interning (global deduplication)
    /// Create a string value with automatic interning for short strings
    /// Returns LuaValue directly with ZERO allocation overhead for interned strings
    ///
    /// Performance characteristics:
    /// - Cache hit (interned): O(1) hash lookup, 0 allocations, 0 atomic ops
    /// - Cache miss (new): 1 Box allocation, GC registration, pool insertion
    /// - Long string: 1 Box allocation, GC registration, no pooling
    pub fn create_string(&mut self, s: &str) -> LuaValue {
        let (id, is_new) = self.object_pool.create_string(s);
        if is_new {
            let size = 32 + s.len();
            self.gc.track_object(GcId::StringId(id), size);
        }
        LuaValue::string(id)
    }

    /// Create string from owned String (avoids clone for non-interned strings)
    #[inline]
    pub fn create_string_owned(&mut self, s: String) -> LuaValue {
        let len = s.len();
        let (id, is_new) = self.object_pool.create_string_owned(s);
        if is_new {
            let size = 32 + len;
            self.gc.track_object(GcId::StringId(id), size);
        }
        LuaValue::string(id)
    }

    /// Get string by LuaValue (resolves ID from object pool)
    pub fn get_string(&self, value: &LuaValue) -> Option<&LuaString> {
        if let Some(id) = value.as_string_id() {
            self.object_pool.get_string(id)
        } else {
            None
        }
    }

    // ============ GC Write Barriers ============
    // These are called when modifying old objects to point to young objects
    // Critical for correct generational GC behavior

    /// Write barrier for table modification
    /// Called when: table[key] = value (fast path)
    /// If table is old and value is young/collectable, mark table as touched
    #[inline(always)]
    pub fn gc_barrier_back_table(&mut self, table_id: TableId, value: &LuaValue) {
        // Only process in generational mode and if value is collectable
        if self.gc.gc_kind() != crate::gc::GcKind::Generational {
            return;
        }

        // Check if value is a collectable GC object
        let value_gc_id = match value.kind() {
            LuaValueKind::Table => value.as_table_id().map(crate::gc::GcId::TableId),
            LuaValueKind::Function => value.as_function_id().map(crate::gc::GcId::FunctionId),
            LuaValueKind::Thread => value.as_thread_id().map(crate::gc::GcId::ThreadId),
            _ => None,
        };

        if value_gc_id.is_some() {
            // Call back barrier on the table
            let table_gc_id = crate::gc::GcId::TableId(table_id);
            self.gc.barrier_back_gen(table_gc_id, &mut self.object_pool);
        }
    }

    /// Write barrier for upvalue modification
    /// Called when: upvalue = value (SETUPVAL)
    /// If upvalue is old/closed and value is young, mark upvalue as touched
    #[inline(always)]
    pub fn gc_barrier_upvalue(&mut self, upvalue_id: UpvalueId, value: &LuaValue) {
        // Only process in generational mode
        if self.gc.gc_kind() != crate::gc::GcKind::Generational {
            return;
        }

        // Check if value is a collectable GC object
        let is_collectable = matches!(
            value.kind(),
            LuaValueKind::Table
                | LuaValueKind::Function
                | LuaValueKind::Thread
                | LuaValueKind::String
        );

        if is_collectable {
            // Forward barrier: mark the value if upvalue is old
            let uv_gc_id = crate::gc::GcId::UpvalueId(upvalue_id);

            // Get value's GcId for forward barrier
            if let Some(value_gc_id) = match value.kind() {
                LuaValueKind::Table => value.as_table_id().map(crate::gc::GcId::TableId),
                LuaValueKind::Function => value.as_function_id().map(crate::gc::GcId::FunctionId),
                LuaValueKind::Thread => value.as_thread_id().map(crate::gc::GcId::ThreadId),
                LuaValueKind::String => value.as_string_id().map(crate::gc::GcId::StringId),
                _ => None,
            } {
                self.gc
                    .barrier_forward_gen(uv_gc_id, value_gc_id, &mut self.object_pool);
            }
        }
    }

    /// Create a new table in object pool
    /// GC tracks objects via ObjectPool iteration, no allgc list needed
    #[inline(always)]
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> LuaValue {
        let id = self.object_pool.create_table(array_size, hash_size);
        // Track object for GC - adds to young_list in generational mode and updates gc_debt
        self.gc.track_object(GcId::TableId(id), 256);
        LuaValue::table(id)
    }

    /// Get table by LuaValue (resolves ID from object pool)
    pub fn get_table(&self, value: &LuaValue) -> Option<&LuaTable> {
        if let Some(id) = value.as_table_id() {
            self.object_pool.get_table(id)
        } else {
            None
        }
    }

    /// Get mutable table by LuaValue
    pub fn get_table_mut(&mut self, value: &LuaValue) -> Option<&mut LuaTable> {
        if let Some(id) = value.as_table_id() {
            self.object_pool.get_table_mut(id)
        } else {
            None
        }
    }

    /// Helper: Set table field via raw_set
    pub fn table_set_raw(&mut self, table: &LuaValue, key: LuaValue, value: LuaValue) {
        if let Some(table_ref) = self.get_table_mut(table) {
            table_ref.raw_set(key, value);
        }
    }

    /// Helper: Get table field via raw_get
    pub fn table_get_raw(&self, table: &LuaValue, key: &LuaValue) -> LuaValue {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.raw_get(key).unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        }
    }

    /// Helper: Set table metatable
    pub fn table_set_metatable(&mut self, table: &LuaValue, metatable: Option<LuaValue>) {
        if let Some(table_ref) = self.get_table_mut(table) {
            table_ref.set_metatable(metatable);
        }
    }

    /// Helper: Get table metatable (also supports userdata and strings)
    pub fn table_get_metatable(&self, value: &LuaValue) -> Option<LuaValue> {
        match value.kind() {
            LuaValueKind::Table => {
                if let Some(table_ref) = self.get_table(value) {
                    table_ref.get_metatable()
                } else {
                    None
                }
            }
            LuaValueKind::Userdata => {
                if let Some(id) = value.as_userdata_id() {
                    self.object_pool.get_userdata(id).and_then(|ud| {
                        let mt = ud.get_metatable();
                        if mt.is_nil() { None } else { Some(mt) }
                    })
                } else {
                    None
                }
            }
            LuaValueKind::String => self.get_string_metatable(),
            _ => None,
        }
    }

    /// Create new userdata in object pool
    pub fn create_userdata(&mut self, data: crate::lua_value::LuaUserdata) -> LuaValue {
        let id = self.object_pool.create_userdata(data);
        LuaValue::userdata(id)
    }

    /// Get userdata by LuaValue (resolves ID from object pool)
    pub fn get_userdata(&self, value: &LuaValue) -> Option<&crate::lua_value::LuaUserdata> {
        if let Some(id) = value.as_userdata_id() {
            self.object_pool.get_userdata(id)
        } else {
            None
        }
    }

    /// Get mutable userdata by LuaValue
    pub fn get_userdata_mut(
        &mut self,
        value: &LuaValue,
    ) -> Option<&mut crate::lua_value::LuaUserdata> {
        if let Some(id) = value.as_userdata_id() {
            self.object_pool.get_userdata_mut(id)
        } else {
            None
        }
    }

    /// Create a function in object pool
    /// Tracks the object in GC's allgc list for efficient sweep
    #[inline(always)]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalue_ids: Vec<UpvalueId>) -> LuaValue {
        let id = self.object_pool.create_function(chunk, upvalue_ids);
        self.gc.track_object(GcId::FunctionId(id), 128);
        LuaValue::function(id)
    }

    /// Create a C closure (native function with upvalues stored as closed upvalues)
    /// The upvalues are automatically created as closed upvalues with the given values
    #[inline]
    pub fn create_c_closure(
        &mut self,
        func: crate::gc::CFunction,
        upvalues: Vec<LuaValue>,
    ) -> LuaValue {
        // Create closed upvalues for each value
        let upvalue_ids: Vec<UpvalueId> = upvalues
            .into_iter()
            .map(|v| self.create_upvalue_closed(v))
            .collect();

        let id = self.object_pool.create_c_closure(func, upvalue_ids);
        self.gc.track_object(GcId::FunctionId(id), 128);
        LuaValue::function(id)
    }

    /// Create a C closure with a single inline upvalue (fast path)
    /// This avoids all upvalue indirection and allocation overhead
    #[inline]
    pub fn create_c_closure_inline1(
        &mut self,
        func: crate::gc::CFunction,
        upvalue: LuaValue,
    ) -> LuaValue {
        let id = self.object_pool.create_c_closure_inline1(func, upvalue);
        self.gc.track_object(GcId::FunctionId(id), 128);
        LuaValue::function(id)
    }

    /// Create an open upvalue pointing to a stack index
    #[inline(always)]
    pub fn create_upvalue_open(&mut self, stack_index: usize) -> UpvalueId {
        let id = self.object_pool.create_upvalue_open(stack_index);
        self.gc.track_object(GcId::UpvalueId(id), 64);
        id
    }

    /// Create a closed upvalue with a value
    #[inline(always)]
    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> UpvalueId {
        let id = self.object_pool.create_upvalue_closed(value);
        self.gc.track_object(GcId::UpvalueId(id), 64);
        id
    }

    /// Get the inline upvalue for CClosureInline1
    /// This is the ultra-fast path for C closures with a single upvalue
    /// Returns the upvalue value directly without any indirection
    #[inline(always)]
    pub fn get_c_closure_inline_upvalue(&self) -> LuaValue {
        self.c_closure_inline_upvalue
    }

    /// Get a C closure upvalue by index (1-based like Lua's lua_upvalueindex)
    /// This can only be called from within a C closure
    /// Returns None if index is out of bounds or not in a C closure context
    #[inline]
    pub fn get_c_closure_upvalue(&self, index: usize) -> Option<LuaValue> {
        if self.c_closure_upvalues_ptr.is_null()
            || index == 0
            || index > self.c_closure_upvalues_len
        {
            return None;
        }

        // Get the upvalue ID (1-based index)
        let upvalue_id = unsafe { *self.c_closure_upvalues_ptr.add(index - 1) };

        // Resolve the upvalue value
        if let Some(upvalue) = self.object_pool.get_upvalue(upvalue_id) {
            upvalue.get_closed_value()
        } else {
            None
        }
    }

    /// Set a C closure upvalue by index (1-based)
    /// This can only be called from within a C closure
    /// Returns true if successful, false if index out of bounds or not in C closure
    #[inline]
    pub fn set_c_closure_upvalue(&mut self, index: usize, value: LuaValue) -> bool {
        if self.c_closure_upvalues_ptr.is_null()
            || index == 0
            || index > self.c_closure_upvalues_len
        {
            return false;
        }

        // Get the upvalue ID (1-based index)
        let upvalue_id = unsafe { *self.c_closure_upvalues_ptr.add(index - 1) };

        // Set the upvalue value
        if let Some(upvalue) = self.object_pool.get_upvalue_mut(upvalue_id) {
            upvalue.close(value);
            true
        } else {
            false
        }
    }

    /// Get function by LuaValue (resolves ID from object pool)
    pub fn get_function(&self, value: &LuaValue) -> Option<&GcFunction> {
        if let Some(id) = value.as_function_id() {
            self.object_pool.get_function(id)
        } else {
            None
        }
    }

    /// Get mutable function by LuaValue
    pub fn get_function_mut(&mut self, value: &LuaValue) -> Option<&mut GcFunction> {
        if let Some(id) = value.as_function_id() {
            self.object_pool.get_function_mut(id)
        } else {
            None
        }
    }

    //==========================================================================
    // Value conversion helpers (for WASM and external APIs)
    //==========================================================================

    /// Convert a LuaValue to its string representation (without metamethods)
    /// This properly resolves GC objects through the object pool
    pub fn value_to_string_raw(&self, value: &LuaValue) -> String {
        if value.is_nil() {
            "nil".to_string()
        } else if let Some(b) = value.as_bool() {
            b.to_string()
        } else if let Some(i) = value.as_integer() {
            i.to_string()
        } else if let Some(n) = value.as_number() {
            // Format float to match Lua output
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{:.1}", n)
            } else {
                n.to_string()
            }
        } else if let Some(lua_str) = self.get_string(value) {
            lua_str.as_str().to_string()
        } else if value.is_table() {
            if let Some(id) = value.as_table_id() {
                format!("table: 0x{:x}", id.0)
            } else {
                "table".to_string()
            }
        } else if value.is_function() {
            if let Some(id) = value.as_function_id() {
                format!("function: 0x{:x}", id.0)
            } else {
                "function".to_string()
            }
        } else if value.is_cfunction() {
            "function".to_string()
        } else if value.is_thread() {
            if let Some(id) = value.as_thread_id() {
                format!("thread: 0x{:x}", id.0)
            } else {
                "thread".to_string()
            }
        } else if value.is_userdata() {
            if let Some(id) = value.as_userdata_id() {
                format!("userdata: 0x{:x}", id.0)
            } else {
                "userdata".to_string()
            }
        } else {
            format!("{:?}", value)
        }
    }

    /// Get the string content of a LuaValue if it is a string
    /// Returns None if the value is not a string
    pub fn value_as_string(&self, value: &LuaValue) -> Option<String> {
        self.get_string(value).map(|s| s.as_str().to_string())
    }

    /// Get the type name of a LuaValue
    pub fn value_type_name(&self, value: &LuaValue) -> &'static str {
        match value.kind() {
            LuaValueKind::Nil => "nil",
            LuaValueKind::Boolean => "boolean",
            LuaValueKind::Integer | LuaValueKind::Float => "number",
            LuaValueKind::String => "string",
            LuaValueKind::Table => "table",
            LuaValueKind::Function | LuaValueKind::CFunction => "function",
            LuaValueKind::Thread => "thread",
            LuaValueKind::Userdata => "userdata",
        }
    }

    /// Check GC and run a step if needed (like luaC_checkGC in Lua 5.4)
    /// This is called after allocating new objects (strings, tables, functions)
    /// Uses GC debt mechanism like Lua: runs when debt > threshold
    ///
    /// OPTIMIZATION: Fast path is inlined, slow path is separate function
    #[inline(always)]
    fn check_gc(&mut self) {
        // Fast path: check if gc_debt > 0
        // Once debt becomes positive, trigger GC step
        if self.gc.gc_debt <= 0 {
            return;
        }
        // Slow path: actual GC work
        self.check_gc_slow();
    }

    /// Slow path for GC - separate function to keep hot path small
    /// Public version for direct inline checks
    #[cold]
    #[inline(never)]
    pub fn check_gc_slow_pub(&mut self) {
        self.check_gc_slow();
    }

    #[cold]
    #[inline(never)]
    fn check_gc_slow(&mut self) {
        // Collect roots using pre-allocated buffer (avoid allocation)
        self.gc_roots_buffer.clear();

        // 1. Global table
        self.gc_roots_buffer.push(LuaValue::table(self.global));

        // 2. Registry table (persistent objects storage)
        self.gc_roots_buffer.push(LuaValue::table(self.registry));

        // 3. String metatable
        if let Some(mt) = &self.string_metatable {
            self.gc_roots_buffer.push(*mt);
        }

        // 4. ALL frame registers AND function values (not just current frame)
        // This is critical - any register in any active frame must be kept alive
        // Also, the function being executed in each frame must be kept alive!
        for frame in &self.frames[..self.frame_count] {
            // Add the function value for this frame - this is CRITICAL!
            self.gc_roots_buffer.push(frame.as_function_value());

            let base_ptr = frame.base_ptr as usize;
            let top = frame.top as usize;
            for i in 0..top {
                if base_ptr + i < self.register_stack.len() {
                    let value = self.register_stack[base_ptr + i];
                    // Skip nil values - they don't need to be roots
                    if !value.is_nil() {
                        self.gc_roots_buffer.push(value);
                    }
                }
            }
        }

        // 5. All registers beyond the last frame's top (temporary values)
        // NOTE: Only scan up to a reasonable limit to avoid scanning stale registers
        if self.frame_count > 0 {
            let last_frame = &self.frames[self.frame_count - 1];
            let last_frame_end = last_frame.base_ptr as usize + last_frame.top as usize;
            // Limit scan to avoid excessive GC work on large register stacks
            let scan_limit = (last_frame_end + 128).min(self.register_stack.len());
            for i in last_frame_end..scan_limit {
                let value = self.register_stack[i];
                if !value.is_nil() {
                    self.gc_roots_buffer.push(value);
                }
            }
        } else {
            // No frames? Scan limited portion
            let scan_limit = 256.min(self.register_stack.len());
            for i in 0..scan_limit {
                let value = self.register_stack[i];
                if !value.is_nil() {
                    self.gc_roots_buffer.push(value);
                }
            }
        }

        // 6. Return values
        for value in &self.return_values {
            self.gc_roots_buffer.push(*value);
        }

        // 7. Open upvalues - these point to stack locations that must stay alive
        for upval_id in &self.open_upvalues {
            if let Some(uv) = self.object_pool.get_upvalue(*upval_id) {
                if let Some(val) = uv.get_closed_value() {
                    self.gc_roots_buffer.push(val);
                }
            }
        }

        // Perform GC step with complete root set
        self.gc.step(&self.gc_roots_buffer, &mut self.object_pool);
    }

    // ============ GC Management ============

    fn register_chunk_constants(&mut self, chunk: &Chunk) {
        for value in &chunk.constants {
            match value.kind() {
                LuaValueKind::String | LuaValueKind::Table => {
                    // Table IDs are managed by object pool, no direct GC registration needed
                    // The object pool will handle lifetime management
                }
                LuaValueKind::Function => {
                    // Function IDs are managed by object pool, no direct GC registration needed
                    // Recursively register nested function chunks if needed
                    if let Some(func_id) = value.as_function_id() {
                        // Extract child chunk before recursion to avoid borrow conflicts
                        let child_chunk =
                            if let Some(func_ref) = self.object_pool.get_function(func_id) {
                                func_ref.chunk().cloned()
                            } else {
                                None
                            };

                        if let Some(child_chunk) = child_chunk {
                            self.register_chunk_constants(&child_chunk);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Perform garbage collection
    pub fn collect_garbage(&mut self) {
        // Collect all roots
        let mut roots = Vec::new();

        // Add the global table itself as a root
        roots.push(LuaValue::table(self.global));

        // Add registry table as a root (persistent objects)
        roots.push(LuaValue::table(self.registry));

        // Add string metatable if present
        if let Some(mt) = &self.string_metatable {
            roots.push(*mt);
        }

        // Add all frame registers AND function values as roots
        for frame in &self.frames[..self.frame_count] {
            // CRITICAL: Add the function being executed
            roots.push(frame.as_function_value());

            let base_ptr = frame.base_ptr as usize;
            let top = frame.top as usize;
            for i in 0..top {
                if base_ptr + i < self.register_stack.len() {
                    roots.push(self.register_stack[base_ptr + i]);
                }
            }
        }

        // Add return values as roots
        for value in &self.return_values {
            roots.push(value.clone());
        }

        // Add open upvalues as roots (only closed ones that have values)
        for upvalue_id in &self.open_upvalues {
            if let Some(uv) = self.object_pool.get_upvalue(*upvalue_id) {
                if let Some(value) = uv.get_closed_value() {
                    roots.push(value);
                }
            }
        }

        // Run GC with mutable object pool reference
        self.gc.collect(&roots, &mut self.object_pool);
    }

    /// Get GC statistics
    pub fn gc_stats(&self) -> String {
        let stats = self.gc.stats();
        format!(
            "GC Stats:\n\
            - Bytes allocated: {}\n\
            - Threshold: {}\n\
            - Total collections: {}\n\
            - Minor collections: {}\n\
            - Major collections: {}\n\
            - Objects collected: {}\n\
            - Young generation size: {}\n\
            - Old generation size: {}\n\
            - Promoted objects: {}",
            stats.bytes_allocated,
            stats.threshold,
            stats.collection_count,
            stats.minor_collections,
            stats.major_collections,
            stats.objects_collected,
            stats.young_gen_size,
            stats.old_gen_size,
            stats.promoted_objects
        )
    }

    // ===== Lightweight Error Handling API =====

    /// Set runtime error and return lightweight error enum
    /// If an error handler is set (via xpcall), it will be called immediately
    pub fn error(&mut self, message: impl Into<String>) -> LuaError {
        // Simply set the error message - error handling is done by xpcall
        // when the error propagates back through the Rust call stack.
        // At that point, the Lua call stack is still intact.
        self.error_message = message.into();
        LuaError::RuntimeError
    }

    /// Set compile error and return lightweight error enum
    #[inline]
    pub fn compile_error(&mut self, message: impl Into<String>) -> LuaError {
        self.error_message = message.into();
        LuaError::CompileError
    }

    /// Set yield values and return lightweight error enum
    #[inline]
    pub fn do_yield(&mut self, values: Vec<LuaValue>) -> LuaError {
        self.yield_values = values;
        LuaError::Yield
    }

    /// Get the current error message
    #[inline]
    pub fn get_error_message(&self) -> &str {
        &self.error_message
    }

    /// Take the yield values (clears internal storage)
    #[inline]
    pub fn take_yield_values(&mut self) -> Vec<LuaValue> {
        std::mem::take(&mut self.yield_values)
    }

    /// Clear error state
    #[inline]
    pub fn clear_error(&mut self) {
        self.error_message.clear();
        self.yield_values.clear();
    }

    /// Try to get a metamethod from a value
    fn get_metamethod(&mut self, value: &LuaValue, event: &str) -> Option<LuaValue> {
        match value.kind() {
            LuaValueKind::Table => {
                if let Some(table_id) = value.as_table_id() {
                    let metatable = {
                        let table = self.object_pool.get_table(table_id)?;
                        table.get_metatable()
                    };
                    if let Some(metatable) = metatable {
                        let key = self.create_string(event);
                        self.table_get_with_meta(&metatable, &key)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            LuaValueKind::String => {
                let key = self.create_string(event);
                // All strings share a metatable
                if let Some(mt) = &self.string_metatable.clone() {
                    if let Some(mt_ref) = self.get_table(mt) {
                        mt_ref.raw_get(&key)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            // TODO: Support metatables for userdata
            _ => None,
        }
    }

    /// Call a binary metamethod (like __add, __sub, etc.)
    #[allow(dead_code)]
    fn call_binop_metamethod(
        &mut self,
        left: &LuaValue,
        right: &LuaValue,
        event: &str,
        result_reg: usize,
    ) -> LuaResult<bool> {
        // Try left operand's metamethod first
        let metamethod = self
            .get_metamethod(left, event)
            .or_else(|| self.get_metamethod(right, event));

        if let Some(mm) = metamethod {
            self.call_metamethod_with_args(mm, vec![left.clone(), right.clone()], result_reg)
        } else {
            Ok(false)
        }
    }

    /// Call a unary metamethod (like __unm, __bnot, etc.)
    #[allow(dead_code)]
    fn call_unop_metamethod(
        &mut self,
        value: &LuaValue,
        event: &str,
        result_reg: usize,
    ) -> LuaResult<bool> {
        if let Some(mm) = self.get_metamethod(value, event) {
            self.call_metamethod_with_args(mm, vec![value.clone()], result_reg)
        } else {
            Ok(false)
        }
    }

    /// Generic method to call a metamethod with given arguments
    #[allow(dead_code)]
    fn call_metamethod_with_args(
        &mut self,
        metamethod: LuaValue,
        args: Vec<LuaValue>,
        result_reg: usize,
    ) -> LuaResult<bool> {
        match metamethod.kind() {
            LuaValueKind::Function => {
                let Some(func_id) = metamethod.as_function_id() else {
                    return Err(self.error("Invalid function ID".to_string()));
                };
                let Some(func_ref) = self.object_pool.get_function(func_id) else {
                    return Err(self.error("Invalid function".to_string()));
                };
                let chunk = func_ref.lua_chunk();
                let max_stack_size = chunk.max_stack_size;
                let code_ptr = chunk.code.as_ptr();
                let constants_ptr = chunk.constants.as_ptr();
                let upvalues_ptr = func_ref.upvalues.as_ptr();

                // CRITICAL FIX: Calculate new base relative to current frame
                // This prevents register_stack from growing indefinitely
                let new_base = if self.frame_count > 0 {
                    let current_frame = &self.frames[self.frame_count - 1];
                    let caller_base = current_frame.base_ptr as usize;
                    let caller_max_stack =
                        if let Some(caller_func_id) = current_frame.get_function_id() {
                            self.object_pool
                                .get_function(caller_func_id)
                                .and_then(|f| f.chunk().map(|c| c.max_stack_size))
                                .unwrap_or(256)
                        } else {
                            256
                        };
                    caller_base + caller_max_stack
                } else {
                    0
                };
                self.ensure_stack_capacity(new_base + max_stack_size);

                // Copy arguments to new frame's registers
                for (i, arg) in args.iter().enumerate() {
                    if i < max_stack_size {
                        self.register_stack[new_base + i] = *arg;
                    }
                }

                let temp_frame = LuaCallFrame::new_lua_function(
                    func_id,
                    code_ptr,
                    constants_ptr,
                    upvalues_ptr,
                    new_base,
                    max_stack_size, // top
                    result_reg,
                    1, // expect 1 result
                    max_stack_size,
                );

                self.push_frame(temp_frame);

                // Execute the metamethod
                let result = self.run()?;

                // Store result in the target register
                if !self.frames_is_empty() {
                    let frame = self.current_frame();
                    let base_ptr = frame.base_ptr as usize;
                    self.set_register(base_ptr, result_reg, result);
                }

                Ok(true)
            }
            LuaValueKind::CFunction => {
                let cf = metamethod.as_cfunction().unwrap();
                // Create temporary frame for CFunction
                let arg_count = args.len() + 1; // +1 for function itself

                // CRITICAL FIX: Calculate new base relative to current frame
                let new_base = if self.frame_count > 0 {
                    let current_frame = &self.frames[self.frame_count - 1];
                    let caller_base = current_frame.base_ptr as usize;
                    let caller_max_stack =
                        if let Some(caller_func_id) = current_frame.get_function_id() {
                            self.object_pool
                                .get_function(caller_func_id)
                                .and_then(|f| f.chunk().map(|c| c.max_stack_size))
                                .unwrap_or(256)
                        } else {
                            256
                        };
                    caller_base + caller_max_stack
                } else {
                    0
                };
                self.ensure_stack_capacity(new_base + arg_count);

                self.register_stack[new_base] = LuaValue::cfunction(cf);
                for (i, arg) in args.iter().enumerate() {
                    self.register_stack[new_base + i + 1] = *arg;
                }

                let temp_frame = LuaCallFrame::new_c_function(new_base, arg_count);

                self.push_frame(temp_frame);

                // Call the CFunction
                let multi_result = cf(self)?;

                // Pop temporary frame
                self.pop_frame_discard();

                // Store result
                let values = multi_result.all_values();
                let result = values.first().cloned().unwrap_or(LuaValue::nil());
                let frame = self.current_frame();
                let base_ptr = frame.base_ptr as usize;
                self.set_register(base_ptr, result_reg, result);

                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Call __tostring metamethod if it exists, return the string result
    pub fn call_tostring_metamethod(
        &mut self,
        lua_table_value: &LuaValue,
    ) -> LuaResult<Option<LuaValue>> {
        // Check for __tostring metamethod
        if let Some(tostring_func) = self.get_metamethod(lua_table_value, "__tostring") {
            // Call the metamethod with the value as argument
            return self.call_metamethod(&tostring_func, &[lua_table_value.clone()]);
        }

        Ok(None)
    }

    /// Convert a value to string, calling __tostring metamethod if present
    pub fn value_to_string(&mut self, value: &LuaValue) -> LuaResult<String> {
        // Handle string values directly
        if value.is_string() {
            if let Some(s) = self.get_string(value) {
                return Ok(s.as_str().to_string());
            }
        }

        if let Some(s) = self.call_tostring_metamethod(value)? {
            if let Some(str) = self.get_string(&s) {
                Ok(str.as_str().to_string())
            } else {
                Err(self.error("`__tostring` metamethod did not return a string".to_string()))
            }
        } else {
            // Format value without using deprecated method
            Ok(self.format_value(value))
        }
    }

    /// Format a value as a string (for display purposes)
    fn format_value(&self, value: &LuaValue) -> String {
        match value.kind() {
            LuaValueKind::Nil => "nil".to_string(),
            LuaValueKind::Boolean => if value.as_bool().unwrap_or(false) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            LuaValueKind::Integer => value
                .as_integer()
                .map(|i| i.to_string())
                .unwrap_or_default(),
            LuaValueKind::Float => value.as_number().map(|n| n.to_string()).unwrap_or_default(),
            LuaValueKind::String => {
                if let Some(s) = self.get_string(value) {
                    s.as_str().to_string()
                } else {
                    "string".to_string()
                }
            }
            LuaValueKind::Table => {
                if let Some(id) = value.as_table_id() {
                    format!("table: {:p}", id.0 as *const ())
                } else {
                    "table".to_string()
                }
            }
            LuaValueKind::Function => "function".to_string(),
            LuaValueKind::CFunction => "function".to_string(),
            LuaValueKind::Userdata => "userdata".to_string(),
            LuaValueKind::Thread => "thread".to_string(),
        }
    }

    /// Generate a stack traceback string
    pub fn generate_traceback(&self, error_msg: &str) -> String {
        let mut trace = format!("{}\nstack traceback:", error_msg);

        // Iterate through active call frames from innermost to outermost (most recent first)
        // frame_count is the number of active frames
        if self.frame_count == 0 {
            return trace;
        }

        // Traverse from innermost (frame_count - 1) to outermost (0)
        for i in (0..self.frame_count).rev() {
            let frame = &self.frames[i];

            // Get source location info
            let (source, line) = if frame.is_lua() {
                // Get function ID and chunk info
                if let Some(func_id) = frame.get_function_id() {
                    if let Some(func) = self.object_pool.get_function(func_id) {
                        if let Some(chunk) = func.chunk() {
                            let source_str = chunk.source_name.as_deref().unwrap_or("?");

                            // Get line number from pc (pc points to next instruction, so use pc-1)
                            let pc = frame.pc.saturating_sub(1) as usize;
                            let line_str =
                                if !chunk.line_info.is_empty() && pc < chunk.line_info.len() {
                                    chunk.line_info[pc].to_string()
                                } else {
                                    "?".to_string()
                                };

                            (source_str.to_string(), line_str)
                        } else {
                            // C closure with upvalues
                            ("[C closure]".to_string(), "?".to_string())
                        }
                    } else {
                        ("?".to_string(), "?".to_string())
                    }
                } else {
                    ("?".to_string(), "?".to_string())
                }
            } else {
                // C function
                ("[C]".to_string(), "?".to_string())
            };

            trace.push_str(&format!("\n\t{}:{}: in function", source, line));
        }

        trace
    }

    /// Execute a function with protected call (pcall semantics)
    /// Note: Yields are NOT caught by pcall - they propagate through
    pub fn protected_call(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Save current state
        let initial_frame_count = self.frame_count;

        // Try to call the function
        let result = self.call_function_internal(func, args);

        match result {
            Ok(return_values) => {
                // Success: return true and the return values
                Ok((true, return_values))
            }
            Err(LuaError::Yield) => {
                // Yield is not an error - propagate it
                Err(LuaError::Yield)
            }
            Err(_) => {
                // Real error: clean up frames and return false with error message
                // Simply clear all open upvalues to avoid dangling references
                self.open_upvalues.clear();

                // Now pop the frames
                while self.frame_count > initial_frame_count {
                    self.pop_frame_discard();
                }

                // Return error - take the message to avoid allocation
                let msg = std::mem::take(&mut self.error_message);
                let error_str = self.create_string(&msg);

                Ok((false, vec![error_str]))
            }
        }
    }

    /// ULTRA-OPTIMIZED pcall for CFunction calls
    /// Works directly on the stack without any Vec allocations
    /// Args are read from caller's stack and results are written directly to return_values
    /// Returns: (success, result_count) where results are in self.return_values
    #[inline]
    pub fn protected_call_stack_based(
        &mut self,
        func: LuaValue,
        arg_base: usize,  // Where args start in stack (caller's base + 1)
        arg_count: usize, // Number of arguments
    ) -> LuaResult<(bool, usize)> {
        // Save current state
        let initial_frame_count = self.frame_count;

        // Call function directly without Vec allocation
        let result = self.call_function_stack_based(func, arg_base, arg_count);

        match result {
            Ok(result_count) => Ok((true, result_count)),
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(_) => {
                // Error path: clean up and return error message
                self.open_upvalues.clear();
                while self.frame_count > initial_frame_count {
                    self.pop_frame_discard();
                }
                let msg = std::mem::take(&mut self.error_message);
                let error_str = self.create_string(&msg);
                self.return_values.clear();
                self.return_values.push(error_str);
                Ok((false, 1))
            }
        }
    }

    /// Internal helper that calls function using stack-based arguments
    /// Avoids Vec allocation for the common case
    /// Results are placed in self.return_values, returns count
    #[inline]
    fn call_function_stack_based(
        &mut self,
        func: LuaValue,
        arg_base: usize,
        arg_count: usize,
    ) -> LuaResult<usize> {
        match func.kind() {
            LuaValueKind::CFunction => {
                let cfunc = func.as_cfunction().unwrap();

                // Calculate new base for the call frame
                let new_base = if self.frame_count > 0 {
                    let current_frame = &self.frames[self.frame_count - 1];
                    (current_frame.base_ptr as usize) + 256
                } else {
                    0
                };

                let stack_size = arg_count + 1;
                self.ensure_stack_capacity(new_base + stack_size);

                // Copy args from caller's stack to new frame
                unsafe {
                    let src = self.register_stack.as_ptr().add(arg_base);
                    let dst = self.register_stack.as_mut_ptr().add(new_base);
                    *dst = func; // func at slot 0
                    std::ptr::copy_nonoverlapping(src, dst.add(1), arg_count);
                }

                let temp_frame = LuaCallFrame::new_c_function(new_base, stack_size);
                if self.try_push_frame(temp_frame).is_none() {
                    return Err(self.error("C stack overflow".to_string()));
                }

                match cfunc(self) {
                    Ok(r) => {
                        self.pop_frame_discard();
                        self.return_values = r.all_values();
                        Ok(self.return_values.len())
                    }
                    Err(LuaError::Yield) => Err(LuaError::Yield),
                    Err(e) => {
                        self.pop_frame_discard();
                        Err(e)
                    }
                }
            }
            LuaValueKind::Function => {
                let Some(func_id) = func.as_function_id() else {
                    return Err(self.error("Invalid function reference".to_string()));
                };

                let (max_stack_size, code_ptr, constants_ptr, upvalues_ptr) = {
                    let Some(func_ref) = self.object_pool.get_function(func_id) else {
                        return Err(self.error("Invalid function".to_string()));
                    };
                    let chunk = func_ref.lua_chunk();
                    let size = chunk.max_stack_size.max(1);
                    (
                        size,
                        chunk.code.as_ptr(),
                        chunk.constants.as_ptr(),
                        func_ref.upvalues.as_ptr(),
                    )
                };

                let new_base = if self.frame_count > 0 {
                    let current_frame = &self.frames[self.frame_count - 1];
                    (current_frame.base_ptr as usize) + 256
                } else {
                    0
                };

                self.ensure_stack_capacity(new_base + max_stack_size);

                // Copy args and initialize remaining slots
                unsafe {
                    let src = self.register_stack.as_ptr().add(arg_base);
                    let dst = self.register_stack.as_mut_ptr().add(new_base);
                    let copy_count = arg_count.min(max_stack_size);
                    std::ptr::copy_nonoverlapping(src, dst, copy_count);
                    // Initialize remaining with nil
                    let nil_val = LuaValue::nil();
                    for i in copy_count..max_stack_size {
                        *dst.add(i) = nil_val;
                    }
                }

                // Push boundary frame and Lua function frame - check for stack overflow
                let boundary_frame = LuaCallFrame::new_c_function(new_base, 0);
                if self.try_push_frame(boundary_frame).is_none() {
                    return Err(self.error("C stack overflow".to_string()));
                }

                let new_frame = LuaCallFrame::new_lua_function(
                    func_id,
                    code_ptr,
                    constants_ptr,
                    upvalues_ptr,
                    new_base,
                    max_stack_size,
                    0,
                    -1,
                    max_stack_size,
                );
                if self.try_push_frame(new_frame).is_none() {
                    self.pop_frame_discard(); // Pop the boundary frame
                    return Err(self.error("C stack overflow".to_string()));
                }

                let exec_result = execute::luavm_execute(self);

                match exec_result {
                    Ok(_) | Err(LuaError::Exit) => {
                        self.pop_frame_discard();
                        Ok(self.return_values.len())
                    }
                    Err(LuaError::Yield) => Err(LuaError::Yield),
                    Err(e) => {
                        self.pop_frame_discard();
                        Err(e)
                    }
                }
            }
            _ => Err(self.error("attempt to call a non-function value".to_string())),
        }
    }

    /// Protected call with error handler (xpcall semantics)
    /// The error handler is registered and will be called by error() when an error occurs
    /// Note: Yields are NOT caught by xpcall - they propagate through
    pub fn protected_call_with_handler(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
        err_handler: LuaValue,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        let initial_frame_count = self.frame_count;

        // Call the function
        let result = self.call_function_internal(func, args);

        match result {
            Ok(values) => Ok((true, values)),
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(_) => {
                // Error occurred - call the error handler NOW while the stack is still intact
                // This allows debug.traceback() to see the full call stack
                let error_msg = self.error_message.clone();
                let err_value = self.create_string(&error_msg);

                let handled_msg = match self.call_function_internal(err_handler, vec![err_value]) {
                    Ok(handler_results) => {
                        // Error handler succeeded, use its return value as the error message
                        if let Some(result) = handler_results.first() {
                            self.value_to_string_raw(result)
                        } else {
                            error_msg
                        }
                    }
                    Err(_) => {
                        // Error handler itself failed
                        format!("error in error handling: {}", error_msg)
                    }
                };

                // NOW clean up frames created by the failed function call
                while self.frame_count > initial_frame_count {
                    let frame = self.pop_frame().unwrap();
                    // Close upvalues belonging to this frame
                    self.close_upvalues_from(frame.base_ptr as usize);
                }

                // Return the handled error message
                let err_str = self.create_string(&handled_msg);
                Ok((false, vec![err_str]))
            }
        }
    }

    /// Internal helper to call a function (used by pcall/xpcall and coroutines)
    /// Optimized: directly calls luavm_execute instead of duplicating the dispatch loop
    pub(crate) fn call_function_internal(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<Vec<LuaValue>> {
        match func.kind() {
            LuaValueKind::CFunction => {
                let cfunc = func.as_cfunction().unwrap();

                // OPTIMIZED: Use fixed offset instead of expensive get_function lookup
                let new_base = if self.frame_count > 0 {
                    let current_frame = &self.frames[self.frame_count - 1];
                    (current_frame.base_ptr as usize) + 256
                } else {
                    0
                };

                let stack_size = args.len() + 1;
                self.ensure_stack_capacity(new_base + stack_size);

                // Set up arguments: func at base, args starting at base+1
                self.register_stack[new_base] = func;
                for (i, arg) in args.iter().enumerate() {
                    self.register_stack[new_base + i + 1] = *arg;
                }

                // Create C function frame
                let temp_frame = LuaCallFrame::new_c_function(new_base, stack_size);
                if self.try_push_frame(temp_frame).is_none() {
                    return Err(self.error("C stack overflow".to_string()));
                }

                // Call CFunction
                let result = match cfunc(self) {
                    Ok(r) => {
                        self.pop_frame_discard();
                        Ok(r.all_values())
                    }
                    Err(LuaError::Yield) => Err(LuaError::Yield),
                    Err(e) => {
                        self.pop_frame_discard();
                        Err(e)
                    }
                };

                result
            }
            LuaValueKind::Function => {
                let Some(func_id) = func.as_function_id() else {
                    return Err(self.error("Invalid function reference".to_string()));
                };

                // Get function info
                let (max_stack_size, code_ptr, constants_ptr, upvalues_ptr) = {
                    let Some(func_ref) = self.object_pool.get_function(func_id) else {
                        return Err(self.error("Invalid function".to_string()));
                    };
                    let chunk = func_ref.lua_chunk();
                    let size = chunk.max_stack_size.max(1);
                    (
                        size,
                        chunk.code.as_ptr(),
                        chunk.constants.as_ptr(),
                        func_ref.upvalues.as_ptr(),
                    )
                };

                // OPTIMIZED: Use fixed offset instead of expensive get_function lookup
                let new_base = if self.frame_count > 0 {
                    let current_frame = &self.frames[self.frame_count - 1];
                    (current_frame.base_ptr as usize) + 256
                } else {
                    0
                };

                self.ensure_stack_capacity(new_base + max_stack_size);

                // Copy args first, then initialize remaining with nil (only beyond args)
                let arg_count = args.len().min(max_stack_size);
                unsafe {
                    let dst = self.register_stack.as_mut_ptr().add(new_base);
                    // Copy arguments
                    for (i, arg) in args.iter().enumerate() {
                        if i < max_stack_size {
                            *dst.add(i) = *arg;
                        }
                    }
                    // Initialize remaining registers with nil
                    let nil_val = LuaValue::nil();
                    for i in arg_count..max_stack_size {
                        *dst.add(i) = nil_val;
                    }
                }

                // Push C function boundary frame - RETURN will detect this and write to return_values
                let boundary_frame = LuaCallFrame::new_c_function(new_base, 0);
                if self.try_push_frame(boundary_frame).is_none() {
                    return Err(self.error("C stack overflow".to_string()));
                }

                // Push Lua function frame
                let new_frame = LuaCallFrame::new_lua_function(
                    func_id,
                    code_ptr,
                    constants_ptr,
                    upvalues_ptr,
                    new_base,
                    max_stack_size,
                    0,  // result_reg unused
                    -1, // LUA_MULTRET
                    max_stack_size,
                );
                if self.try_push_frame(new_frame).is_none() {
                    self.pop_frame_discard(); // Pop the boundary frame
                    return Err(self.error("C stack overflow".to_string()));
                }

                // Execute using the main dispatcher - no duplicate code!
                let exec_result = execute::luavm_execute(self);

                match exec_result {
                    Ok(_) | Err(LuaError::Exit) => {
                        // Normal return - pop boundary frame and get return values
                        self.pop_frame_discard();
                        let result = std::mem::take(&mut self.return_values);

                        // NOTE: We intentionally don't clear the stack here anymore.
                        // The stack will be overwritten on next call, and GC can handle
                        // any stale references. This gives significant performance improvement.

                        Ok(result)
                    }
                    Err(LuaError::Yield) => {
                        // Yield - frames stay for resume
                        Err(LuaError::Yield)
                    }
                    Err(e) => {
                        // Error - pop boundary frame and clear stack region
                        self.pop_frame_discard();
                        for i in new_base..(new_base + max_stack_size) {
                            if i < self.register_stack.len() {
                                self.register_stack[i] = LuaValue::nil();
                            }
                        }
                        Err(e)
                    }
                }
            }
            _ => Err(self.error("attempt to call a non-function value".to_string())),
        }
    }

    // Async bridge API: Call a registered async function (internal use)
    #[cfg(feature = "async")]
    pub fn async_call(
        &mut self,
        func_name: &str,
        args: Vec<LuaValue>,
        coroutine: LuaValue,
    ) -> LuaResult<u64> {
        let task_id = self
            .async_executor
            .spawn_task(func_name, args, coroutine)
            .map_err(|e| self.error(e))?;
        Ok(task_id)
    }

    // Poll all async tasks and resume completed coroutines
    #[cfg(feature = "async")]
    pub fn poll_async(&mut self) -> LuaResult<()> {
        let completed_tasks = self.async_executor.collect_completed_tasks();

        for (_task_id, coroutine, result) in completed_tasks {
            // Resume the coroutine with the result values
            let values = result?;
            let (_success, _resume_result) = self.resume_thread(coroutine, values)?;
        }

        Ok(())
    }

    // Register an async function callable from Lua
    #[cfg(feature = "async")]
    pub fn register_async_function<F, Fut>(&mut self, name: &str, func: F)
    where
        F: Fn(Vec<LuaValue>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = LuaResult<Vec<LuaValue>>> + Send + 'static,
    {
        self.async_executor
            .register_async_function(name.to_string(), func);
    }

    // Get the number of active async tasks
    #[cfg(feature = "async")]
    pub fn active_async_tasks(&self) -> usize {
        self.async_executor.active_task_count()
    }
}
