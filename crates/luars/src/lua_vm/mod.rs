// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
mod dispatcher;
mod lua_call_frame;
mod lua_error;
mod opcode;

use crate::gc::{GC, GcFunction, ThreadId, UpvalueId};
#[cfg(feature = "async")]
use crate::lua_async::AsyncExecutor;
use crate::lua_value::{
    Chunk, CoroutineStatus, LuaString, LuaTable, LuaThread, LuaValue, LuaValueKind,
};
pub use crate::lua_vm::lua_call_frame::LuaCallFrame;
pub use crate::lua_vm::lua_error::LuaError;
use crate::{lib_registry, Compiler, ObjectPool};
use dispatcher::dispatch_instruction;
pub use opcode::{Instruction, OpCode};
use std::cell::RefCell;
use std::rc::Rc;

pub type LuaResult<T> = Result<T, LuaError>;

pub struct LuaVM {
    // Global environment table (_G and _ENV point to this)
    pub(crate) global_value: LuaValue,

    // Call stack
    pub frames: Vec<LuaCallFrame>,

    // Global register stack (unified stack architecture, like Lua 5.4)
    pub register_stack: Vec<LuaValue>,

    // Garbage collector
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

    // Object pool for unified object management (new architecture)
    pub(crate) object_pool: ObjectPool,

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
        let mut vm = LuaVM {
            global_value: LuaValue::nil(),
            frames: Vec::with_capacity(64), // Pre-allocate call stack for typical recursion depth
            register_stack: Vec::with_capacity(256), // Pre-allocate for initial stack
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
            object_pool: ObjectPool::new(),
            #[cfg(feature = "async")]
            async_executor: AsyncExecutor::new(),
            // Initialize error storage
            error_message: String::new(),
            yield_values: Vec::new(),
        };

        // Set _G to point to the global table itself
        let globals_ref = vm.create_table(0, 20);
        vm.global_value = globals_ref;
        vm.set_global("_G", globals_ref);
        vm.set_global("_ENV", globals_ref);

        vm
    }

    // Register access helpers for unified stack architecture
    #[inline(always)]
    #[allow(dead_code)]
    fn get_register(&self, base_ptr: usize, reg: usize) -> LuaValue {
        self.register_stack[base_ptr + reg]
    }

    #[inline(always)]
    #[allow(dead_code)]
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
    }

    /// Execute a chunk directly (convenience method)
    pub fn execute(&mut self, chunk: Rc<Chunk>) -> LuaResult<LuaValue> {
        // Register all constants in the chunk with GC
        self.register_chunk_constants(&chunk);

        // Create upvalue for _ENV (global table)
        // Main chunks in Lua 5.4 always have _ENV as upvalue[0]
        let env_upvalue_id = self.object_pool.create_upvalue_closed(self.global_value);
        let upvalues = vec![env_upvalue_id];

        // Create main function in object pool with _ENV upvalue
        let main_func_value = self.create_function(chunk.clone(), upvalues);

        // Create initial call frame using unified stack
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let base_ptr = self.register_stack.len();
        let required_size = base_ptr + chunk.max_stack_size;
        self.ensure_stack_capacity(required_size);

        // Get code pointer from chunk
        let code_ptr = chunk.code.as_ptr();

        let frame = LuaCallFrame::new_lua_function(
            frame_id,
            main_func_value,
            code_ptr,
            base_ptr,
            chunk.max_stack_size,
            0,
            0,
        );

        self.frames.push(frame);

        // Execute
        let result = self.run()?;

        // Clean up - clear stack used by this execution
        self.register_stack.clear();
        self.open_upvalues.clear();

        Ok(result)
    }

    /// Call a function value (for testing and runtime calls)
    pub fn call_function(&mut self, func: LuaValue, args: Vec<LuaValue>) -> LuaResult<LuaValue> {
        // Clear previous state
        self.register_stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();

        // Get function from object pool
        let Some(func_id) = func.as_function_id() else {
            return Err(self.error("Not a function".to_string()));
        };

        // Clone chunk and get info before borrowing self mutably
        let (chunk, max_stack, code_ptr) = {
            let Some(func_ref) = self.object_pool.get_function(func_id) else {
                return Err(self.error("Invalid function ID".to_string()));
            };
            let chunk = func_ref.chunk.clone();
            let max_stack = chunk.max_stack_size;
            let code_ptr = chunk.code.as_ptr();
            (chunk, max_stack, code_ptr)
        };

        // Register chunk constants
        self.register_chunk_constants(&chunk);

        // Setup stack and frame
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

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

        let frame =
            LuaCallFrame::new_lua_function(frame_id, func, code_ptr, base_ptr, max_stack, 0, 0);

        self.frames.push(frame);

        // Execute
        let result = self.run()?;

        Ok(result)
    }

    pub fn execute_string(&mut self, source: &str) -> LuaResult<LuaValue> {
        let chunk = self.compile(source)?;
        self.execute(Rc::new(chunk))
    }

    /// Compile source code using VM's string pool
    pub fn compile(&mut self, source: &str) -> LuaResult<Chunk> {
        let chunk = match Compiler::compile(self, source) {
            Ok(c) => c,
            Err(e) => return Err(self.compile_error(e)),
        };

        Ok(chunk)
    }

    /// Main execution loop - interprets bytecode instructions
    /// Main VM execution loop - MAXIMUM PERFORMANCE
    fn run(&mut self) -> LuaResult<LuaValue> {
        // CRITICAL OPTIMIZATION: Check frames.is_empty() OUTSIDE the loop
        // RETURN instruction will pop frames and check if empty
        // This removes 100M+ unnecessary checks for tight loops
        loop {
            // ULTRA-FAST PATH: Direct unsafe access, no bounds checks
            // Get frame_ptr ONCE and pass to dispatch to avoid repeated Vec lookups
            let frame_ptr = unsafe { self.frames.last_mut().unwrap_unchecked() as *mut LuaCallFrame };
            let instr = unsafe { (*frame_ptr).code_ptr.add((*frame_ptr).pc).read() };
            unsafe { (*frame_ptr).pc += 1; }

            // Use match with explicit Ok branch for better branch prediction hints
            match dispatch_instruction(self, instr, frame_ptr) {
                Ok(()) => {}
                Err(LuaError::Exit) => return Ok(LuaValue::nil()),
                Err(LuaError::Yield) => return Ok(LuaValue::nil()),
                Err(e) => return Err(e),
            }
        }
    }

    // Helper methods
    #[inline(always)]
    pub(crate) fn current_frame(&self) -> &LuaCallFrame {
        unsafe { self.frames.last().unwrap_unchecked() }
    }

    #[inline(always)]
    pub(crate) fn current_frame_mut(&mut self) -> &mut LuaCallFrame {
        unsafe { self.frames.last_mut().unwrap_unchecked() }
    }

    pub fn get_global(&mut self, name: &str) -> Option<LuaValue> {
        let key = self.create_string(name);
        if let Some(global_id) = self.global_value.as_table_id() {
            let global = self.object_pool.get_table(global_id)?;
            global.raw_get(&key)
        } else {
            None
        }
    }

    pub fn get_global_by_lua_value(&self, key: &LuaValue) -> Option<LuaValue> {
        if let Some(global_id) = self.global_value.as_table_id() {
            let global = self.object_pool.get_table(global_id)?;
            global.raw_get(key)
        } else {
            None
        }
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        let key = self.create_string(name);

        if let Some(global_id) = self.global_value.as_table_id() {
            if let Some(global) = self.object_pool.get_table_mut(global_id) {
                global.raw_set(key.clone(), value.clone());
            }

            // Write barrier: global table (old) may now reference new object
            self.gc
                .barrier_forward(crate::gc::GcObjectType::Table, global_id.0);
            self.gc.barrier_back(&value);
        }
    }

    pub fn set_global_by_lua_value(&mut self, key: &LuaValue, value: LuaValue) {
        if let Some(global_id) = self.global_value.as_table_id() {
            if let Some(global) = self.object_pool.get_table_mut(global_id) {
                global.raw_set(key.clone(), value.clone());
            }

            // Write barrier
            self.gc
                .barrier_forward(crate::gc::GcObjectType::Table, global_id.0);
            self.gc.barrier_back(&value);
        }
    }

    /// Set the metatable for all strings
    /// In Lua, all strings share a metatable with __index pointing to the string library
    pub fn set_string_metatable(&mut self, string_lib: LuaValue) {
        // Create the metatable
        let metatable = self.create_table(0, 1);

        // Create the __index key before any borrowing
        let index_key = self.create_string("__index");

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

    /// Create a new thread (coroutine) - returns ThreadId-based LuaValue
    pub fn create_thread_value(&mut self, func: LuaValue) -> LuaValue {
        let mut thread = LuaThread {
            status: CoroutineStatus::Suspended,
            frames: Vec::new(),
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

        // Store the function in the thread's first register
        thread.register_stack.push(func);

        // Create thread in ObjectPool and return LuaValue
        let thread_id = self.object_pool.create_thread(thread);
        LuaValue::thread(thread_id)
    }

    /// Create a new thread (coroutine) - legacy version returning Rc<RefCell<>>
    /// This is still needed for internal VM state tracking (current_thread)
    pub fn create_thread(&mut self, func: LuaValue) -> Rc<RefCell<LuaThread>> {
        let thread = LuaThread {
            status: CoroutineStatus::Suspended,
            frames: Vec::new(),
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
    pub fn resume_thread(
        &mut self,
        thread_val: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Get ThreadId from LuaValue
        let Some(thread_id) = thread_val.as_thread_id() else {
            return Err(self.error("invalid thread".to_string()));
        };

        // Check thread status first
        let status = {
            let Some(thread) = self.object_pool.get_thread(thread_id) else {
                return Err(self.error("invalid thread".to_string()));
            };
            thread.status
        };

        match status {
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

        // Save current VM state
        let saved_frames = std::mem::take(&mut self.frames);
        let saved_stack = std::mem::take(&mut self.register_stack);
        let saved_returns = std::mem::take(&mut self.return_values);
        let saved_upvalues = std::mem::take(&mut self.open_upvalues);
        let saved_frame_id = self.next_frame_id;
        let saved_thread = self.current_thread.take();
        let saved_thread_id = self.current_thread_id.take();

        // Get thread state and check if first resume
        let is_first_resume = {
            let Some(thread) = self.object_pool.get_thread(thread_id) else {
                return Err(self.error("invalid thread".to_string()));
            };
            thread.frames.is_empty()
        };

        // Load thread state into VM
        {
            let Some(thread) = self.object_pool.get_thread_mut(thread_id) else {
                return Err(self.error("invalid thread".to_string()));
            };
            thread.status = CoroutineStatus::Running;
            self.frames = std::mem::take(&mut thread.frames);
            self.register_stack = std::mem::take(&mut thread.register_stack);
            self.return_values = std::mem::take(&mut thread.return_values);
            self.open_upvalues = std::mem::take(&mut thread.open_upvalues);
            self.next_frame_id = thread.next_frame_id;
        }

        self.current_thread_id = Some(thread_id);
        self.current_thread_value = Some(thread_val.clone());

        // Execute
        let result = if is_first_resume {
            // First resume: call the function
            let func = self
                .register_stack
                .get(0)
                .cloned()
                .unwrap_or(LuaValue::nil());
            match self.call_function_internal(func, args) {
                Ok(values) => Ok(values),
                Err(LuaError::Yield) => {
                    // Function yielded - this is expected
                    let values = self.take_yield_values();
                    Ok(values)
                }
                Err(e) => Err(e),
            }
        } else {
            // Resumed from yield:
            // Use saved CALL instruction info to properly store return values
            let (call_reg, call_nret) = {
                let Some(thread) = self.object_pool.get_thread(thread_id) else {
                    return Err(self.error("invalid thread".to_string()));
                };
                (thread.yield_call_reg, thread.yield_call_nret)
            };

            if let (Some(a), Some(num_expected)) = (call_reg, call_nret) {
                let frame = &self.frames[self.frames.len() - 1];
                let base_ptr = frame.base_ptr;
                let top = frame.top;

                // Store resume args as return values of the yield call
                let num_returns = args.len();
                let n = if num_expected == usize::MAX {
                    num_returns
                } else {
                    num_expected.min(num_returns)
                };

                for (i, value) in args.iter().take(n).enumerate() {
                    if base_ptr + a + i < self.register_stack.len() && a + i < top {
                        self.register_stack[base_ptr + a + i] = value.clone();
                    }
                }
                // Fill remaining expected registers with nil
                for i in num_returns..num_expected.min(top - a) {
                    if base_ptr + a + i < self.register_stack.len() {
                        self.register_stack[base_ptr + a + i] = LuaValue::nil();
                    }
                }

                // Clear the saved info
                if let Some(thread) = self.object_pool.get_thread_mut(thread_id) {
                    thread.yield_call_reg = None;
                    thread.yield_call_nret = None;
                }
            }

            self.return_values = args;

            // Continue execution from where it yielded
            self.run().map(|_| self.return_values.clone())
        };

        // Check if thread yielded by examining the result
        let did_yield = match &result {
            Ok(_) if !self.frames.is_empty() => {
                // If frames are not empty after execution, it means we yielded
                true
            }
            _ => false,
        };

        // Save thread state back
        let final_result = if did_yield {
            // Thread yielded - save state and return yield values
            let Some(thread) = self.object_pool.get_thread_mut(thread_id) else {
                return Err(self.error("invalid thread".to_string()));
            };
            thread.frames = std::mem::take(&mut self.frames);
            thread.register_stack = std::mem::take(&mut self.register_stack);
            thread.return_values = std::mem::take(&mut self.return_values);
            thread.open_upvalues = std::mem::take(&mut self.open_upvalues);
            thread.next_frame_id = self.next_frame_id;
            thread.status = CoroutineStatus::Suspended;

            let values = thread.yield_values.clone();
            thread.yield_values.clear();

            Ok((true, values))
        } else {
            // Thread completed or error
            let Some(thread) = self.object_pool.get_thread_mut(thread_id) else {
                return Err(self.error("invalid thread".to_string()));
            };
            thread.frames = std::mem::take(&mut self.frames);
            thread.register_stack = std::mem::take(&mut self.register_stack);
            thread.return_values = std::mem::take(&mut self.return_values);
            thread.open_upvalues = std::mem::take(&mut self.open_upvalues);
            thread.next_frame_id = self.next_frame_id;

            match result {
                Ok(values) => {
                    thread.status = CoroutineStatus::Dead;
                    Ok((true, values))
                }
                Err(LuaError::Exit) => {
                    // Normal exit - coroutine finished successfully
                    thread.status = CoroutineStatus::Dead;
                    Ok((true, thread.return_values.clone()))
                }
                Err(_) => {
                    thread.status = CoroutineStatus::Dead;
                    let error_msg = self.get_error_message().to_string();
                    Ok((false, vec![self.create_string(&error_msg)]))
                }
            }
        };

        // Restore VM state
        self.frames = saved_frames;
        self.register_stack = saved_stack;
        self.return_values = saved_returns;
        self.open_upvalues = saved_upvalues;
        self.next_frame_id = saved_frame_id;
        self.current_thread = saved_thread;
        self.current_thread_id = saved_thread_id;
        self.current_thread_value = None; // Clear after resume completes

        final_result
    }

    /// Yield from current coroutine
    /// Returns Err(LuaError::Yield) which will be caught by run() loop
    pub fn yield_thread(&mut self, values: Vec<LuaValue>) -> LuaResult<()> {
        if let Some(thread_id) = self.current_thread_id {
            // Store yield values in the thread
            if let Some(thread) = self.object_pool.get_thread_mut(thread_id) {
                thread.yield_values = values.clone();
                thread.status = CoroutineStatus::Suspended;
            }
            // Return Yield "error" to unwind the call stack
            Err(self.do_yield(values))
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
                let index_key = self.create_string("__index");

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
            let index_key = self.create_string("__index");
            
            let index_value = {
                let metatable = self.object_pool.get_table(meta_id)?;
                metatable.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    LuaValueKind::Table => {
                        return self.table_get_with_meta(&index_val, key);
                    }
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let args = vec![lua_table_value.clone(), key.clone()];
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
            let index_key = self.create_string("__index");

            let index_value = {
                let mt = self.object_pool.get_table(mt_id)?;
                mt.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table
                    LuaValueKind::Table => return self.table_get_with_meta(&index_val, key),

                    // __index is a function - call it with (userdata, key)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let args = vec![lua_userdata_value.clone(), key.clone()];
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
        let index_key = self.create_string("__index");
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
                    // __index is a function - call it with (string, key)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let args = vec![string_val.clone(), key.clone()];
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
            self.gc.barrier_forward(crate::gc::GcObjectType::Table, table_id.0);
            self.gc.barrier_back(&value);
            return Ok(());
        }

        // Key doesn't exist, check for __newindex metamethod
        if let Some(mt) = meta_value
            && let Some(mt_id) = mt.as_table_id()
        {
            let newindex_key = self.create_string("__newindex");
            
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
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let args = vec![lua_table_val, key, value];
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
        self.gc.barrier_forward(crate::gc::GcObjectType::Table, table_id.0);
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
        // Use call_function_internal for both C functions and Lua functions
        let result = self.call_function_internal(func.clone(), args.to_vec())?;
        Ok(result.get(0).cloned())
    }

    // Integer division

    /// Close all open upvalues for a specific frame
    /// Called when a frame exits to move values from stack to heap
    #[allow(dead_code)]
    fn close_upvalues(&mut self, frame_id: usize) {
        // Find all open upvalues pointing to this frame
        let upvalues_to_close: Vec<UpvalueId> = self
            .open_upvalues
            .iter()
            .filter(|uv_id| {
                if let Some(frame) = self.frames.iter().find(|f| f.frame_id == frame_id) {
                    if let Some(uv) = self.object_pool.get_upvalue(**uv_id) {
                        // Check if any open upvalue points to this frame
                        for reg_idx in 0..frame.top {
                            if uv.points_to(frame_id, reg_idx) {
                                return true;
                            }
                        }
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
                if let Some((fid, reg)) = uv.get_open_location() {
                    let value = self.get_upvalue_value(fid, reg);
                    if let Some(uv_mut) = self.object_pool.get_upvalue_mut(*uv_id) {
                        uv_mut.close(value);
                    }
                }
            }
        }

        // Remove closed upvalues from the open list
        self.open_upvalues.retain(|uv_id| {
            self.object_pool.get_upvalue(*uv_id)
                .map(|uv| uv.is_open())
                .unwrap_or(false)
        });
    }

    /// Helper: Get value from stack for an open upvalue
    fn get_upvalue_value(&self, frame_id: usize, register: usize) -> LuaValue {
        if let Some(frame) = self.frames.iter().find(|f| f.frame_id == frame_id) {
            let abs_idx = frame.base_ptr + register;
            if abs_idx < self.register_stack.len() {
                return self.register_stack[abs_idx].clone();
            }
        }
        LuaValue::nil()
    }

    /// Get upvalue value by UpvalueId
    /// For open upvalues, reads from register stack
    /// For closed upvalues, returns the stored value
    pub fn read_upvalue(&self, uv_id: UpvalueId) -> LuaValue {
        if let Some(uv) = self.object_pool.get_upvalue(uv_id) {
            if let Some((frame_id, register)) = uv.get_open_location() {
                // Open upvalue - read from stack
                if let Some(frame) = self.frames.iter().find(|f| f.frame_id == frame_id) {
                    let abs_idx = frame.base_ptr + register;
                    if abs_idx < self.register_stack.len() {
                        return self.register_stack[abs_idx];
                    }
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

    /// Set upvalue value by UpvalueId
    /// For open upvalues, writes to register stack
    /// For closed upvalues, updates the stored value
    pub fn write_upvalue(&mut self, uv_id: UpvalueId, value: LuaValue) {
        if let Some(uv) = self.object_pool.get_upvalue(uv_id) {
            if let Some((frame_id, register)) = uv.get_open_location() {
                // Open upvalue - write to stack
                if let Some(frame) = self.frames.iter().find(|f| f.frame_id == frame_id) {
                    let abs_idx = frame.base_ptr + register;
                    if abs_idx < self.register_stack.len() {
                        self.register_stack[abs_idx] = value;
                    }
                }
            } else {
                // Closed upvalue - update stored value
                if let Some(uv_mut) = self.object_pool.get_upvalue_mut(uv_id) {
                    uv_mut.close(value);
                }
            }
        }
    }

    /// Close all open upvalues at or above the given stack position
    /// Used by RETURN (k bit) and CLOSE instructions
    pub fn close_upvalues_from(&mut self, stack_pos: usize) {
        // Collect frame info first to avoid borrowing issues
        let frame_info: Vec<(usize, usize, usize)> = self.frames.iter().map(|frame| {
            let max_regs = if let Some(func_id) = frame.function_value.as_function_id() {
                self.object_pool.get_function(func_id)
                    .map(|f| f.chunk.max_stack_size)
                    .unwrap_or(0)
            } else {
                0
            };
            (frame.frame_id, frame.base_ptr, max_regs)
        }).collect();

        let upvalues_to_close: Vec<UpvalueId> = self
            .open_upvalues
            .iter()
            .filter(|uv_id| {
                if let Some(uv) = self.object_pool.get_upvalue(**uv_id) {
                    // Check if this upvalue points to stack_pos or higher
                    for (frame_id, base_ptr, max_regs) in &frame_info {
                        for reg_idx in 0..*max_regs {
                            if uv.points_to(*frame_id, reg_idx) {
                                let absolute_pos = base_ptr + reg_idx;
                                if absolute_pos >= stack_pos {
                                    return true;
                                }
                            }
                        }
                    }
                }
                false
            })
            .cloned()
            .collect();

        // Close each upvalue
        for uv_id in upvalues_to_close.iter() {
            if let Some(uv) = self.object_pool.get_upvalue(*uv_id) {
                if let Some((fid, reg)) = uv.get_open_location() {
                    let value = self.get_upvalue_value(fid, reg);
                    if let Some(uv_mut) = self.object_pool.get_upvalue_mut(*uv_id) {
                        uv_mut.close(value);
                    }
                }
            }
        }

        // Remove closed upvalues from the open list
        self.open_upvalues.retain(|uv_id| {
            self.object_pool.get_upvalue(*uv_id)
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

            // Try to get __close metamethod
            let close_key = self.create_string("__close");
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
    /// For short strings (≤64 bytes), use interning (global deduplication)
    /// Create a string value with automatic interning for short strings
    /// Returns LuaValue directly with ZERO allocation overhead for interned strings
    ///
    /// Performance characteristics:
    /// - Cache hit (interned): O(1) hash lookup, 0 allocations, 0 atomic ops
    /// - Cache miss (new): 1 Box allocation, GC registration, pool insertion
    /// - Long string: 1 Box allocation, GC registration, no pooling
    pub fn create_string(&mut self, s: &str) -> LuaValue {
        let id = self.object_pool.create_string(s);

        // Estimate memory cost: string data + LuaString struct overhead
        // LuaString: ~32 bytes base + string length
        let estimated_bytes = 32 + s.len();
        self.gc.record_allocation(estimated_bytes);

        // GC check MUST NOT happen here - object not yet protected!
        // Caller must call check_gc() AFTER storing value in register

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

    /// Create a new table in object pool
    #[inline(always)]
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> LuaValue {
        let id = self.object_pool.create_table(array_size, hash_size);

        // Register with GC - ultra-lightweight, just update debt
        self.gc
            .register_object(id.0, crate::gc::GcObjectType::Table);

        // GC check MUST NOT happen here - object not yet protected!
        // Caller must call check_gc() AFTER storing value in register

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
    pub fn get_userdata(
        &self,
        value: &LuaValue,
    ) -> Option<&crate::lua_value::LuaUserdata> {
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
    #[inline(always)]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalue_ids: Vec<UpvalueId>) -> LuaValue {
        let id = self.object_pool.create_function(chunk, upvalue_ids);

        // Register with GC - ultra-lightweight
        self.gc.register_object(id.0, crate::gc::GcObjectType::Function);

        LuaValue::function(id)
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

    /// Helper: Get chunk from current frame's function (for hot path)
    #[inline]
    #[allow(dead_code)]
    fn get_current_chunk(&self) -> Result<std::rc::Rc<Chunk>, String> {
        let frame = self.current_frame();
        if let Some(func_ref) = self.get_function(&frame.function_value) {
            Ok(func_ref.chunk.clone())
        } else {
            Err("Invalid function in current frame".to_string())
        }
    }

    /// Get constant from current frame's function
    /// This is a hot-path helper for instructions that need to load constants
    #[inline]
    pub fn get_frame_constant(&self, frame: &LuaCallFrame, index: usize) -> Option<LuaValue> {
        let func_id = frame.function_value.as_function_id()?;
        let func_ref = self.object_pool.get_function(func_id)?;
        func_ref.chunk.constants.get(index).copied()
    }

    /// Get instruction from current frame's function code
    /// This is needed for MMBIN/MMBINI/MMBINK which need to read the previous instruction
    #[inline]
    pub fn get_frame_instruction(&self, frame: &LuaCallFrame, index: usize) -> Option<u32> {
        let func_id = frame.function_value.as_function_id()?;
        let func_ref = self.object_pool.get_function(func_id)?;
        func_ref.chunk.code.get(index).copied()
    }

    /// Helper: Get upvalue from current frame's function
    #[inline]
    #[allow(dead_code)]
    fn get_current_upvalue_id(&self, index: usize) -> Result<UpvalueId, String> {
        let frame = self.current_frame();
        if let Some(func_ref) = self.get_function(&frame.function_value) {
            if index < func_ref.upvalues.len() {
                Ok(func_ref.upvalues[index])
            } else {
                Err(format!("Invalid upvalue index: {}", index))
            }
        } else {
            Err("Invalid function in current frame".to_string())
        }
    }

    /// Check GC and run a step if needed (like luaC_checkGC in Lua 5.4)
    /// This is called after allocating new objects (strings, tables, functions)
    /// Uses GC debt mechanism: runs when debt > 0
    ///
    /// OPTIMIZATION: Use incremental collection with work budget
    fn check_gc(&mut self) {
        // Fast path: check debt without collecting roots
        if !self.gc.should_collect() {
            return;
        }

        // Incremental GC: only collect every N checks to reduce overhead
        self.gc.increment_check_counter();
        if !self.gc.should_run_collection() {
            return;
        }

        // Collect roots: all reachable objects from VM state
        let mut roots = Vec::new();

        // 1. Global table
        roots.push(self.global_value);

        // 2. String metatable
        if let Some(mt) = &self.string_metatable {
            roots.push(*mt);
        }

        // 3. ALL frame registers (not just current frame)
        // This is critical - any register in any active frame must be kept alive
        for frame in &self.frames {
            let base_ptr = frame.base_ptr;
            let top = frame.top;
            for i in 0..top {
                if base_ptr + i < self.register_stack.len() {
                    roots.push(self.register_stack[base_ptr + i]);
                }
            }
        }

        // 4. All registers beyond the frames (temporary values)
        if let Some(last_frame) = self.frames.last() {
            let last_frame_end = last_frame.base_ptr + last_frame.top;
            for i in last_frame_end..self.register_stack.len() {
                roots.push(self.register_stack[i]);
            }
        } else {
            // No frames? Collect all registers
            for reg in &self.register_stack {
                roots.push(*reg);
            }
        }

        // 5. Return values
        for value in &self.return_values {
            roots.push(*value);
        }

        // 6. Open upvalues - these point to stack locations that must stay alive
        for upval_id in &self.open_upvalues {
            if let Some(uv) = self.object_pool.get_upvalue(*upval_id) {
                if let Some(val) = uv.get_closed_value() {
                    roots.push(val);
                }
            }
        }

        // Perform GC step with complete root set
        self.gc.step(&roots, &mut self.object_pool);
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
                                Some(func_ref.chunk.clone())
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
        roots.push(self.global_value);

        // Add all frame registers as roots
        for frame in &self.frames {
            let base_ptr = frame.base_ptr;
            let top = frame.top;
            for i in 0..top {
                roots.push(self.register_stack[base_ptr + i]);
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
    #[inline]
    pub fn error(&mut self, message: impl Into<String>) -> LuaError {
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
                let max_stack_size = func_ref.chunk.max_stack_size;
                let code_ptr = func_ref.chunk.code.as_ptr();

                // Save current state
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // Allocate registers in global stack
                let new_base = self.register_stack.len();
                self.ensure_stack_capacity(new_base + max_stack_size);

                // Copy arguments to new frame's registers
                for (i, arg) in args.iter().enumerate() {
                    if i < max_stack_size {
                        self.register_stack[new_base + i] = *arg;
                    }
                }

                let temp_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    metamethod,
                    code_ptr,
                    new_base,
                    max_stack_size,
                    result_reg,
                    1, // expect 1 result
                );

                self.frames.push(temp_frame);

                // Execute the metamethod
                let result = self.run()?;

                // Store result in the target register
                if !self.frames.is_empty() {
                    let frame = self.current_frame();
                    let base_ptr = frame.base_ptr;
                    self.set_register(base_ptr, result_reg, result);
                }

                Ok(true)
            }
            LuaValueKind::CFunction => {
                let cf = metamethod.as_cfunction().unwrap();
                // Create temporary frame for CFunction
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let arg_count = args.len() + 1; // +1 for function itself
                let new_base = self.register_stack.len();
                self.ensure_stack_capacity(new_base + arg_count);

                self.register_stack[new_base] = LuaValue::cfunction(cf);
                for (i, arg) in args.iter().enumerate() {
                    self.register_stack[new_base + i + 1] = *arg;
                }

                let parent_pc = self.current_frame().pc;
                let temp_frame = LuaCallFrame::new_c_function(
                    frame_id, metamethod, parent_pc, new_base, arg_count,
                );

                self.frames.push(temp_frame);

                // Call the CFunction
                let multi_result = cf(self)?;

                // Pop temporary frame
                self.frames.pop();

                // Store result
                let values = multi_result.all_values();
                let result = values.first().cloned().unwrap_or(LuaValue::nil());
                let frame = self.current_frame();
                let base_ptr = frame.base_ptr;
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
            LuaValueKind::Boolean => if value.as_bool().unwrap_or(false) { "true" } else { "false" }.to_string(),
            LuaValueKind::Integer => value.as_integer().map(|i| i.to_string()).unwrap_or_default(),
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

        // Iterate through call frames from top to bottom (most recent first)
        for frame in self.frames.iter().rev() {
            // Dynamically resolve chunk for debug info
            let (source, line) = if let Some(func_ref) = self.get_function(&frame.function_value) {
                let chunk = &func_ref.chunk;

                let source_str = chunk.source_name.as_deref().unwrap_or("[?]");

                let pc = frame.pc.saturating_sub(1);
                let line_str = if !chunk.line_info.is_empty() && pc < chunk.line_info.len() {
                    chunk.line_info[pc].to_string()
                } else {
                    "?".to_string()
                };

                (source_str.to_string(), line_str)
            } else {
                ("[?]".to_string(), "?".to_string())
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
        let initial_frame_count = self.frames.len();

        // Try to call the function
        let result = self.call_function_internal(func, args);

        match result {
            Ok(return_values) => {
                // Success: return true and the return values
                Ok((true, return_values))
            }
            Err(LuaError::Yield) => {
                // Yield is not an error - propagate it
                let values = self.take_yield_values();
                Err(self.do_yield(values))
            }
            Err(_) => {
                // Real error: clean up frames and return false with error message
                // Simply clear all open upvalues to avoid dangling references
                self.open_upvalues.clear();

                // Now pop the frames
                while self.frames.len() > initial_frame_count {
                    self.frames.pop();
                }

                // Return error - the actual message is stored in vm.error_message
                let msg = self.error_message.clone();
                let error_str = self.create_string(&msg);

                Ok((false, vec![error_str]))
            }
        }
    }

    /// Protected call with error handler
    /// Note: Yields are NOT caught by xpcall - they propagate through
    pub fn protected_call_with_handler(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
        err_handler: LuaValue,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        let old_handler = self.error_handler.clone();
        self.error_handler = Some(err_handler.clone());

        let initial_frame_count = self.frames.len();

        let result = self.call_function_internal(func, args);

        self.error_handler = old_handler;

        match result {
            Ok(values) => Ok((true, values)),
            Err(LuaError::Yield) => Err(LuaError::Yield),
            Err(_) => {
                // Clean up frames created by the failed function call
                while self.frames.len() > initial_frame_count {
                    let frame = self.frames.pop().unwrap();
                    // Close upvalues belonging to this frame
                    self.close_upvalues_from(frame.base_ptr);
                }

                eprintln!("[xpcall] Calling error handler");
                // Get the actual error message
                let msg = self.error_message.clone();
                let err_value = self.create_string(&msg);
                let err_display = format!("Runtime Error: {}", msg);
                
                let handler_result = self.call_function_internal(err_handler, vec![err_value]);

                match handler_result {
                    Ok(handler_values) => Ok((false, handler_values)),
                    Err(LuaError::Yield) => {
                        // Yield from error handler - propagate it
                        let values = self.take_yield_values();
                        Err(self.do_yield(values))
                    }
                    Err(_) => {
                        let err_str =
                            self.create_string(&format!("Error in error handler: {}", err_display));
                        Ok((false, vec![err_str]))
                    }
                }
            }
        }
    }

    /// Internal helper to call a function (used by pcall/xpcall and coroutines)
    /// For regular function calls, the CALL instruction in dispatcher should be used
    pub(crate) fn call_function_internal(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<Vec<LuaValue>> {
        match func.kind() {
            LuaValueKind::CFunction => {
                let cfunc = func.as_cfunction().unwrap();
                // For CFunction, create a temporary frame
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // CRITICAL: Allocate metamethod frame AFTER caller's max_stack
                // to prevent overwriting ANY caller registers (including those beyond top)
                let new_base = if let Some(current_frame) = self.frames.last() {
                    let caller_base = current_frame.base_ptr;
                    let caller_max_stack =
                        if let Some(func_id) = current_frame.function_value.as_function_id() {
                            self.object_pool.get_function(func_id)
                                .map(|f| f.chunk.max_stack_size)
                                .unwrap_or(256)
                        } else {
                            256
                        };
                    caller_base + caller_max_stack
                } else {
                    0
                };
                let stack_size = 16; // enough for most cfunc calls
                self.ensure_stack_capacity(new_base + stack_size);

                self.register_stack[new_base] = func;
                for (i, arg) in args.iter().enumerate() {
                    if i + 1 < stack_size {
                        self.register_stack[new_base + i + 1] = arg.clone();
                    }
                }

                // Create dummy function and add to object pool
                let dummy_chunk = Rc::new(Chunk {
                    code: Vec::new(),
                    constants: Vec::new(),
                    locals: Vec::new(),
                    upvalue_count: 0,
                    param_count: 0,
                    is_vararg: false,
                    max_stack_size: stack_size,
                    child_protos: Vec::new(),
                    upvalue_descs: Vec::new(),
                    source_name: Some("[direct_call]".to_string()),
                    line_info: Vec::new(),
                });
                let dummy_func_id = self.object_pool.create_function(dummy_chunk.clone(), Vec::new());
                let dummy_func_value = LuaValue::function_id(dummy_func_id);

                // Use new_base as result_reg (same as caller_base + caller_max_stack)
                let safe_result_reg = new_base;

                // Create empty code for dummy frame
                let code_ptr = dummy_chunk.code.as_ptr();

                let temp_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    dummy_func_value,
                    code_ptr,
                    new_base,
                    stack_size,
                    safe_result_reg,
                    usize::MAX,
                );

                self.frames.push(temp_frame);

                // Call CFunction - ensure frame is always popped even on error
                let result = match cfunc(self) {
                    Ok(r) => r,
                    Err(LuaError::Yield) => {
                        return Err(LuaError::Yield);
                    }
                    Err(e) => {
                        self.frames.pop();
                        return Err(e);
                    }
                };

                self.frames.pop();

                Ok(result.all_values())
            }
            LuaValueKind::Function => {
                let Some(func_id) = func.as_function_id() else {
                    return Err(self.error("Invalid function reference".to_string()));
                };

                // Get function info from object pool
                let (max_stack_size, code_ptr) = {
                    let Some(func_ref) = self.object_pool.get_function(func_id) else {
                        return Err(self.error("Invalid function".to_string()));
                    };
                    let size = if func_ref.chunk.max_stack_size == 0 { 1 } else { func_ref.chunk.max_stack_size };
                    (size, func_ref.chunk.code.as_ptr())
                };

                // For Lua function, use similar logic to call_metamethod
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // CRITICAL: Allocate metamethod frame AFTER caller's max_stack
                let new_base = if let Some(current_frame) = self.frames.last() {
                    let caller_base = current_frame.base_ptr;
                    let caller_max_stack =
                        if let Some(caller_func_id) = current_frame.function_value.as_function_id() {
                            self.object_pool.get_function(caller_func_id)
                                .map(|f| f.chunk.max_stack_size)
                                .unwrap_or(256)
                        } else {
                            256
                        };
                    caller_base + caller_max_stack
                } else {
                    0
                };
                self.ensure_stack_capacity(new_base + max_stack_size);

                // Initialize all registers with nil
                for i in new_base..(new_base + max_stack_size) {
                    self.register_stack[i] = LuaValue::nil();
                }

                // Copy arguments to registers
                for (i, arg) in args.iter().enumerate() {
                    if i < max_stack_size {
                        self.register_stack[new_base + i] = arg.clone();
                    }
                }

                // Use caller's max_stack as result_reg (safe write position)
                let safe_result_reg = new_base; // Same as caller_base + caller_max_stack

                let new_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    func,
                    code_ptr,
                    new_base,
                    max_stack_size,
                    safe_result_reg,
                    usize::MAX, // Want all return values
                );

                let initial_frame_count = self.frames.len();
                self.frames.push(new_frame);

                // Execute instructions until frame returns
                let exec_result: LuaResult<()> = loop {
                    if self.frames.len() <= initial_frame_count {
                        // Frame has been popped (function returned)
                        break Ok(());
                    }

                    let frame_idx = self.frames.len() - 1;
                    let pc = self.frames[frame_idx].pc;
                    let function_value = self.frames[frame_idx].function_value;

                    // Dynamically resolve chunk
                    let chunk = if let Some(func_ref) = self.get_function(&function_value) {
                        func_ref.chunk.clone()
                    } else {
                        break Err(self.error("Invalid function in frame".to_string()));
                    };

                    if pc >= chunk.code.len() {
                        // End of code
                        self.frames.pop();
                        break Ok(());
                    }

                    let instr = chunk.code[pc];
                    self.frames[frame_idx].pc += 1;

                    // Dispatch instruction (zero-return-value design)
                    // Get frame_ptr for optimization
                    let frame_ptr = &mut self.frames[frame_idx] as *mut LuaCallFrame;
                    if let Err(e) = dispatch_instruction(self, instr, frame_ptr) {
                        break Err(e);
                    }
                };

                match exec_result {
                    Ok(_) => {
                        // Get return values
                        let result = self.return_values.clone();
                        self.return_values.clear();
                        Ok(result)
                    }
                    Err(e) => Err(e),
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
        let task_id = self.async_executor.spawn_task(func_name, args, coroutine)?;
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
