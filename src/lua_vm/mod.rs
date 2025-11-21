// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
mod lua_call_frame;
mod lua_error;
mod opcode;
mod dispatcher;

use crate::gc::GC;
use dispatcher::{dispatch_instruction, DispatchAction};
use crate::lua_value::{
    Chunk, CoroutineStatus, LuaFunction, LuaString, LuaTable, LuaThread, LuaUpvalue, LuaValue,
    LuaValueKind,
};
pub use crate::lua_vm::lua_call_frame::LuaCallFrame;
pub use crate::lua_vm::lua_error::LuaError;
use crate::{ObjectPool, lib_registry};
pub use opcode::{Instruction, OpCode};
use std::cell::RefCell;
use std::rc::Rc;

pub type LuaResult<T> = Result<T, LuaError>;

pub struct LuaVM {
    // Global environment table (_G and _ENV point to this)
    pub(crate) globals: LuaValue,

    // Call stack
    pub frames: Vec<LuaCallFrame>,

    // Global register stack (unified stack architecture, like Lua 5.4)
    pub register_stack: Vec<LuaValue>,

    // Garbage collector
    pub(crate) gc: GC,

    // Multi-return value buffer (temporary storage for function returns)
    pub return_values: Vec<LuaValue>,

    // Open upvalues list (for closing when frames exit)
    pub(crate) open_upvalues: Vec<Rc<LuaUpvalue>>,

    // Next frame ID (for tracking frames)
    pub(crate) next_frame_id: usize,

    // Error handling state
    pub error_handler: Option<LuaValue>, // Current error handler for xpcall

    // FFI state
    pub(crate) ffi_state: crate::ffi::FFIState,

    // Current running thread (for coroutine.running())
    pub current_thread: Option<Rc<RefCell<LuaThread>>>,

    // Current thread as LuaValue (for comparison in coroutine.running())
    pub current_thread_value: Option<LuaValue>,

    // Main thread representation (for coroutine.running() in main thread)
    pub main_thread_value: Option<LuaValue>,

    // String metatable (shared by all strings) - stored as TableId in LuaValue
    pub(crate) string_metatable: Option<LuaValue>,

    // Object pool for unified object management (new architecture)
    pub(crate) object_pool: crate::object_pool::ObjectPool,
}

impl LuaVM {
    pub fn new() -> Self {
        let mut vm = LuaVM {
            globals: LuaValue::nil(),
            frames: Vec::new(),
            register_stack: Vec::with_capacity(1024), // Pre-allocate for initial stack
            gc: GC::new(),
            return_values: Vec::new(),
            open_upvalues: Vec::new(),
            next_frame_id: 0,
            error_handler: None,
            ffi_state: crate::ffi::FFIState::new(),
            current_thread: None,
            current_thread_value: None,
            main_thread_value: None, // Will be initialized lazily
            string_metatable: None,
            object_pool: ObjectPool::new(),
        };

        // Set _G to point to the global table itself
        let globals_ref = vm.create_table();
        vm.globals = globals_ref;
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
    }

    /// Execute a chunk directly (convenience method)
    pub fn execute(&mut self, chunk: Rc<Chunk>) -> LuaResult<LuaValue> {
        // Register all constants in the chunk with GC
        self.register_chunk_constants(&chunk);

        // Create upvalue for _ENV (global table)
        // Main chunks in Lua 5.4 always have _ENV as upvalue[0]
        let env_upvalue = LuaUpvalue::new_closed(self.globals);
        let upvalues = vec![env_upvalue];

        // Create main function in object pool with _ENV upvalue
        let main_func_value = self.create_function(chunk.clone(), upvalues);

        // Create initial call frame using unified stack
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let base_ptr = self.register_stack.len();
        let required_size = base_ptr + chunk.max_stack_size;
        self.ensure_stack_capacity(required_size);

        let frame = LuaCallFrame::new_lua_function(
            frame_id,
            main_func_value,
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

        // Get function pointer
        let func_ptr = func
            .as_function_ptr()
            .ok_or_else(|| LuaError::RuntimeError("Not a function".to_string()))?;

        let func_obj = unsafe { &*func_ptr };
        let func_ref = func_obj.borrow();

        // Register chunk constants
        self.register_chunk_constants(&func_ref.chunk);

        // Setup stack and frame
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let base_ptr = 0; // Start from beginning of cleared stack
        let max_stack = func_ref.chunk.max_stack_size;
        let required_size = max_stack; // Need at least max_stack registers
        
        // Initialize stack with nil values
        self.register_stack.resize(required_size, LuaValue::nil());

        // Copy arguments to registers
        for (i, arg) in args.iter().enumerate() {
            if i < max_stack {
                self.register_stack[base_ptr + i] = *arg;
            }
        }

        drop(func_ref);

        let frame = LuaCallFrame::new_lua_function(frame_id, func, base_ptr, max_stack, 0, 0);

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
        use crate::compiler::Compiler;

        let chunk = match Compiler::compile(self, source) {
            Ok(c) => c,
            Err(e) => return Err(LuaError::CompileError(e)),
        };

        Ok(chunk)
    }

    /// Main execution loop - interprets bytecode instructions
    /// Returns the final return value from the chunk
    fn run(&mut self) -> LuaResult<LuaValue> {
        loop {
            // Check if we have any frames to execute
            if self.frames.is_empty() {
                // Execution finished
                return Ok(self.return_values.first().copied().unwrap_or(LuaValue::nil()));
            }

            // Get current frame and chunk
            let frame = self.current_frame();
            let func_ptr = frame
                .get_function_ptr()
                .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;

            // Safety: func_ptr is valid as long as the function exists in object_pool
            let func = unsafe { &*func_ptr };
            let func_ref = func.borrow();
            let chunk = &func_ref.chunk;

            // Check PC bounds
            let pc = frame.pc;
            if pc >= chunk.code.len() {
                return Err(LuaError::RuntimeError(format!(
                    "PC out of bounds: {} >= {}",
                    pc,
                    chunk.code.len()
                )));
            }

            // Fetch instruction
            let instr = chunk.code[pc];

            // Drop borrows before executing instruction
            drop(func_ref);

            // Increment PC (some instructions will modify it)
            self.current_frame_mut().pc += 1;

            // Dispatch instruction using the dispatcher module
            let action = match dispatch_instruction(self, instr) {
                Ok(action) => action,
                Err(LuaError::Yield(_)) => {
                    // Coroutine yielded via CFunction (e.g., coroutine.yield)
                    // Convert to Yield action
                    DispatchAction::Yield
                }
                Err(e) => return Err(e),
            };

            // Handle dispatch action
            match action {
                DispatchAction::Continue => {
                    // Continue to next instruction
                }
                DispatchAction::Skip(n) => {
                    // Skip N additional instructions (PC already incremented by 1)
                    self.current_frame_mut().pc += n;
                }
                DispatchAction::Return => {
                    // Function returned, check if execution is done
                    if self.frames.is_empty() {
                        return Ok(self
                            .return_values
                            .first()
                            .copied()
                            .unwrap_or(LuaValue::nil()));
                    }
                }
                DispatchAction::Yield => {
                    // Coroutine yielded - return control to resume_thread
                    // yield_values should already be set in the current thread
                    return Ok(LuaValue::nil());
                }
                DispatchAction::Call => {
                    // TODO: Handle function call (CALL instruction will set this)
                    // For now, just continue
                }
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
        if let Some(global_id) = self.globals.as_table_id() {
            let global = self.object_pool.get_table(global_id).unwrap();
            global.borrow().raw_get(&key)
        } else {
            None
        }
    }

    pub fn get_global_by_lua_value(&self, key: &LuaValue) -> Option<LuaValue> {
        if let Some(global_id) = self.globals.as_table_id() {
            let global = self.object_pool.get_table(global_id).unwrap();
            global.borrow().raw_get(key)
        } else {
            None
        }
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        let key = self.create_string(name);

        if let Some(global_id) = self.globals.as_table_id() {
            let global = self.object_pool.get_table(global_id).unwrap();
            global.borrow_mut().raw_set(key, value);
        }
    }

    pub fn set_global_by_lua_value(&self, key: &LuaValue, value: LuaValue) {
        if let Some(global_id) = self.globals.as_table_id() {
            let global = self.object_pool.get_table(global_id).unwrap();
            global.borrow_mut().raw_set(key.clone(), value);
        }
    }

    /// Set the metatable for all strings
    /// In Lua, all strings share a metatable with __index pointing to the string library
    pub fn set_string_metatable(&mut self, string_lib: LuaValue) {
        // Create the metatable
        let metatable = self.create_table();

        // Create the __index key before any borrowing
        let index_key = self.create_string("__index");

        // Get the table reference to set __index
        if let Some(mt_ref) = self.get_table(&metatable) {
            // Set __index to the string library table
            mt_ref.borrow_mut().raw_set(index_key, string_lib);
        }

        // Store the metatable as LuaValue (contains TableId)
        self.string_metatable = Some(metatable);
    }

    /// Get the shared string metatable
    pub fn get_string_metatable(&self) -> Option<LuaValue> {
        self.string_metatable.clone()
    }

    /// Get FFI state (immutable)
    pub fn get_ffi_state(&self) -> &crate::ffi::FFIState {
        &self.ffi_state
    }

    /// Get FFI state (mutable)
    pub fn get_ffi_state_mut(&mut self) -> &mut crate::ffi::FFIState {
        &mut self.ffi_state
    }

    // ============ Coroutine Support ============

    /// Create a new thread (coroutine)
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

    /// Resume a coroutine
    pub fn resume_thread(
        &mut self,
        thread_val: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Extract Rc from LuaValue
        let thread_rc = unsafe {
            let ptr = thread_val
                .as_thread_ptr()
                .ok_or(LuaError::RuntimeError("invalid thread".to_string()))?;
            if ptr.is_null() {
                return Err(LuaError::RuntimeError("invalid thread".to_string()));
            }
            let rc = Rc::from_raw(ptr);
            let cloned = rc.clone();
            std::mem::forget(rc); // Don't drop
            cloned
        };

        let status = thread_rc.borrow().status;

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

        let is_first_resume = {
            let thread = thread_rc.borrow();
            thread.frames.is_empty()
        };

        // Load thread state
        {
            let mut thread = thread_rc.borrow_mut();
            thread.status = CoroutineStatus::Running;
            self.frames = std::mem::take(&mut thread.frames);
            self.register_stack = std::mem::take(&mut thread.register_stack);
            self.return_values = std::mem::take(&mut thread.return_values);
            self.open_upvalues = std::mem::take(&mut thread.open_upvalues);
            self.next_frame_id = thread.next_frame_id;
        }

        self.current_thread = Some(thread_rc.clone());
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
                Err(LuaError::Yield(values)) => {
                    // Function yielded - this is expected
                    Ok(values)
                }
                Err(e) => Err(e),
            }
        } else {
            // Resumed from yield:
            // Use saved CALL instruction info to properly store return values
            let (call_reg, call_nret) = {
                let thread = thread_rc.borrow();
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
                } // Fill remaining expected registers with nil
                for i in num_returns..num_expected.min(top - a) {
                    if base_ptr + a + i < self.register_stack.len() {
                        self.register_stack[base_ptr + a + i] = LuaValue::nil();
                    }
                }

                // Clear the saved info
                thread_rc.borrow_mut().yield_call_reg = None;
                thread_rc.borrow_mut().yield_call_nret = None;
            }

            self.return_values = args;

            // Continue execution from where it yielded
            self.run().map(|v| vec![v])
        };

        // Check if thread yielded by examining thread's yield_values
        let did_yield = {
            let thread = thread_rc.borrow();
            !thread.yield_values.is_empty()
        };

        // Save thread state back
        let final_result = if did_yield {
            // Thread yielded - save state and return yield values
            let mut thread = thread_rc.borrow_mut();
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
            let mut thread = thread_rc.borrow_mut();
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
                Err(e) => {
                    thread.status = CoroutineStatus::Dead;
                    Ok((false, vec![self.create_string(&format!("{}", e))]))
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
        self.current_thread_value = None; // Clear after resume completes

        final_result
    }

    /// Yield from current coroutine
    /// Returns Err(LuaError::Yield) which will be caught by run() loop
    pub fn yield_thread(&mut self, values: Vec<LuaValue>) -> LuaResult<()> {
        if let Some(thread_rc) = &self.current_thread {
            // Store yield values in the thread
            thread_rc.borrow_mut().yield_values = values.clone();
            thread_rc.borrow_mut().status = CoroutineStatus::Suspended;
            // Return Yield "error" to unwind the call stack
            Err(LuaError::Yield(values))
        } else {
            Err(LuaError::RuntimeError(
                "attempt to yield from outside a coroutine".to_string(),
            ))
        }
    }

    /// Get value from table with metatable support (__index metamethod)
    /// Use this for GETTABLE, GETFIELD, GETI instructions
    /// For raw access without metamethods, use table_get_raw() instead
    pub fn table_get_with_meta(&mut self, lua_table_value: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        // Fast path: use cached pointer if available (ZERO ObjectPool lookup!)
        if let Some(ptr) = lua_table_value.as_table_ptr() {
            // SAFETY: Pointer is valid as long as table exists in ObjectPool
            let lua_table = unsafe { &*ptr };

            // First try raw get
            let value = {
                let table = lua_table.borrow();
                table.raw_get(key).unwrap_or(LuaValue::nil())
            };

            if !value.is_nil() {
                return Some(value);
            }

            // Check for __index metamethod
            let meta_value = {
                let table = lua_table.borrow();
                table.get_metatable()
            };

            if let Some(mt) = meta_value
                && let Some(meta_id) = mt.as_table_id()
            {
                let index_key = self.create_string("__index");

                // Try cached pointer for metatable too
                if let Some(mt_ptr) = mt.as_table_ptr() {
                    let metatable = unsafe { &*mt_ptr };
                    let index_value = {
                        let mt_borrowed = metatable.borrow();
                        mt_borrowed.raw_get(&index_key)
                    };

                    if let Some(index_val) = index_value {
                        match index_val.kind() {
                            LuaValueKind::Table => return self.table_get_with_meta(&index_val, key),
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
                } else {
                    // Fallback to ObjectPool lookup
                    let metatable = self.object_pool.get_table(meta_id)?;
                    let index_value = {
                        let mt_borrowed = metatable.borrow();
                        mt_borrowed.raw_get(&index_key)
                    };

                    if let Some(index_val) = index_value {
                        match index_val.kind() {
                            LuaValueKind::Table => return self.table_get_with_meta(&index_val, key),
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
            }

            return None;
        }

        // Slow path: no cached pointer, use ObjectPool lookup
        let Some(table_id) = lua_table_value.as_table_id() else {
            return None;
        };

        let lua_table = self.object_pool.get_table(table_id)?;

        // First try raw get
        let value = {
            let table = lua_table.borrow();
            table.raw_get(key).unwrap_or(LuaValue::nil())
        };

        if !value.is_nil() {
            return Some(value);
        }

        // If not found, check for __index metamethod
        let meta_value = {
            let table = lua_table.borrow();
            table.get_metatable()
        };

        if let Some(mt) = meta_value
            && let Some(meta_id) = mt.as_table_id()
        {
            let index_key = self.create_string("__index");
            let metatable = self.object_pool.get_table(meta_id)?;

            let index_value = {
                let mt_borrowed = metatable.borrow();
                mt_borrowed.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table
                    LuaValueKind::Table => return self.table_get_with_meta(&index_val, key),

                    // __index is a function - call it with (table, key)
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

        let userdata = self.object_pool.get_userdata(userdata_id)?;
        // Check for __index metamethod
        let metatable = userdata.borrow().get_metatable();

        if let Some(mt_id) = metatable.as_table_id() {
            let index_key = self.create_string("__index");

            let index_value = {
                let mt_borrowed = self.object_pool.get_table(mt_id)?.borrow();
                mt_borrowed.raw_get(&index_key)
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
        if let Some(mt) = &self.string_metatable {
            let index_value = if let Some(mt_ref) = self.get_table(mt) {
                mt_ref.borrow().raw_get(&index_key)
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
        // Fast path: use cached pointer if available (ZERO ObjectPool lookup!)
        if let Some(ptr) = lua_table_val.as_table_ptr() {
            // SAFETY: Pointer is valid as long as table exists in ObjectPool
            let lua_table = unsafe { &*ptr };

            // Check if key already exists
            let has_key = {
                let table = lua_table.borrow();
                table.raw_get(&key).map(|v| !v.is_nil()).unwrap_or(false)
            };

            if has_key {
                // Key exists, use raw set
                lua_table.borrow_mut().raw_set(key, value);
                return Ok(());
            }

            // Key doesn't exist, check for __newindex metamethod
            let meta_value = {
                let table = lua_table.borrow();
                table.get_metatable()
            };

            if let Some(mt) = meta_value
                && let Some(table_id) = mt.as_table_id()
            {
                let newindex_key = self.create_string("__newindex");

                // Try to use cached metatable pointer
                let newindex_value = if let Some(mt_ptr) = mt.as_table_ptr() {
                    let metatable = unsafe { &*mt_ptr };
                    let mt_borrowed = metatable.borrow();
                    mt_borrowed.raw_get(&newindex_key)
                } else {
                    // Fallback to ObjectPool lookup
                    let metatable = self
                        .object_pool
                        .get_table(table_id)
                        .ok_or(LuaError::RuntimeError("missing metatable".to_string()))?;
                    let mt_borrowed = metatable.borrow();
                    mt_borrowed.raw_get(&newindex_key)
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
            lua_table.borrow_mut().raw_set(key, value);
            return Ok(());
        }

        // Slow path: no cached pointer, use ObjectPool lookup
        let Some(table_id) = lua_table_val.as_table_id() else {
            return Err(LuaError::RuntimeError("table_set: not a table".to_string()));
        };

        let lua_table = self
            .object_pool
            .get_table(table_id)
            .ok_or(LuaError::RuntimeError("invalid table".to_string()))?;

        // Check if key already exists
        let has_key = {
            let table = lua_table.borrow();
            table.raw_get(&key).map(|v| !v.is_nil()).unwrap_or(false)
        };

        if has_key {
            // Key exists, use raw set
            lua_table.borrow_mut().raw_set(key, value);
            return Ok(());
        }

        // Key doesn't exist, check for __newindex metamethod
        let meta_value = {
            let table = lua_table.borrow();
            table.get_metatable()
        };

        if let Some(mt) = meta_value
            && let Some(table_id) = mt.as_table_id()
        {
            let newindex_key = self.create_string("__newindex");
            let metatable = self
                .object_pool
                .get_table(table_id)
                .ok_or(LuaError::RuntimeError("missing metatable".to_string()))?;

            let newindex_value = {
                let mt_borrowed = metatable.borrow();
                mt_borrowed.raw_get(&newindex_key)
            };

            if let Some(newindex_val) = newindex_value {
                match newindex_val.kind() {
                    // __newindex is a table - set in that table
                    LuaValueKind::Table => {
                        return self.table_set_with_meta(newindex_val, key, value);
                    }
                    // __newindex is a function - call it with (table, key, value)
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

        let lua_table = self
            .object_pool
            .get_table(table_id)
            .ok_or(LuaError::RuntimeError("invalid table".to_string()))?;
        // No metamethod or key doesn't exist, use raw set
        lua_table.borrow_mut().raw_set(key, value);
        Ok(())
    }

    /// Call a Lua value (function or CFunction) with the given arguments
    /// Returns the first return value, or None if the call fails
    pub fn call_metamethod(
        &mut self,
        func: &LuaValue,
        args: &[LuaValue],
    ) -> LuaResult<Option<LuaValue>> {
        match func.kind() {
            LuaValueKind::CFunction => {
                let cfunc = func.as_cfunction().unwrap();
                // Create a temporary frame for the call
                let mut registers = vec![func.clone()];
                registers.extend_from_slice(args);
                registers.resize(16, LuaValue::nil());

                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // We need a dummy function for the frame - use an empty one
                let dummy_func = LuaFunction {
                    chunk: Rc::new(Chunk {
                        code: Vec::new(),
                        constants: Vec::new(),
                        locals: Vec::new(),
                        upvalue_count: 0,
                        param_count: 0,
                        is_vararg: false,
                        max_stack_size: 16,
                        child_protos: Vec::new(),
                        upvalue_descs: Vec::new(),
                        source_name: Some("[C]".to_string()),
                        line_info: Vec::new(),
                    }),
                    upvalues: Vec::new(),
                };

                let dummy_func_id = self.object_pool.create_function(dummy_func);
                let dummy_func_value = LuaValue::function_id(dummy_func_id);

                let base_ptr = self.register_stack.len();
                let num_args = registers.len();
                self.ensure_stack_capacity(base_ptr + num_args);
                for (i, val) in registers.into_iter().enumerate() {
                    self.register_stack[base_ptr + i] = val;
                }

                let temp_frame =
                    LuaCallFrame::new_c_function(frame_id, dummy_func_value, 0, base_ptr, num_args);

                self.frames.push(temp_frame);

                // Call the CFunction
                let result = cfunc(self);

                // Pop the temporary frame
                self.frames.pop();

                match result {
                    Ok(multi_val) => {
                        let values = multi_val.all_values();
                        Ok(values.get(0).cloned())
                    }
                    Err(e) => Err(e),
                }
            }
            LuaValueKind::Function => {
                let lua_func_id = func.as_function_id().unwrap();

                // Get max_stack_size before mutable operations
                let max_stack_size = {
                    let lua_func_ref = self
                        .object_pool
                        .get_function(lua_func_id)
                        .ok_or(LuaError::RuntimeError("invalid function".to_string()))?;
                    lua_func_ref.borrow().chunk.max_stack_size
                };

                // Call Lua function
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // Create a new call frame
                let base_ptr = self.register_stack.len();
                self.ensure_stack_capacity(base_ptr + max_stack_size);

                // Initialize with nil
                for i in 0..max_stack_size {
                    self.register_stack[base_ptr + i] = LuaValue::nil();
                }

                // Copy arguments to registers (starting from register 0)
                for (i, arg) in args.iter().enumerate() {
                    if i < max_stack_size {
                        self.register_stack[base_ptr + i] = arg.clone();
                    }
                }

                let new_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    func.clone(), // Clone the LuaValue to pass ownership
                    base_ptr,
                    max_stack_size,
                    0,
                    0, // Don't write back to caller's registers
                );

                let initial_frame_count = self.frames.len();
                self.frames.push(new_frame);

                // Execute instructions in this frame until it returns
                let exec_result = loop {
                    if self.frames.len() <= initial_frame_count {
                        // Frame has been popped (function returned)
                        break Ok(());
                    }

                    let frame_idx = self.frames.len() - 1;
                    let pc = self.frames[frame_idx].pc;
                    let function_value = self.frames[frame_idx].function_value;

                    // Dynamically resolve chunk
                    let chunk = if let Some(func_ref) = self.get_function(&function_value) {
                        func_ref.borrow().chunk.clone()
                    } else {
                        break Err(LuaError::RuntimeError(
                            "Invalid function in frame".to_string(),
                        ));
                    };

                    if pc >= chunk.code.len() {
                        // End of code
                        self.frames.pop();
                        break Ok(());
                    }

                    let _instr = chunk.code[pc];
                    self.frames[frame_idx].pc += 1;

                    // Execute through dispatcher
                    // TODO: This needs to be integrated with the main run() loop
                    // For now, call_metamethod should use call_function_internal
                    todo!("call_metamethod needs refactoring to use dispatcher");
                };

                match exec_result {
                    Ok(_) => {
                        // Get the return value from return_values buffer
                        let result = if !self.return_values.is_empty() {
                            Some(self.return_values[0].clone())
                        } else {
                            None
                        };
                        // Clear return values
                        self.return_values.clear();
                        Ok(result)
                    }
                    Err(e) => Err(e),
                }
            }
            _ => Err(LuaError::RuntimeError(
                "Attempt to call a non-function value".to_string(),
            )),
        }
    }

    // Integer division

    /// Close all open upvalues for a specific frame
    /// Called when a frame exits to move values from stack to heap
    #[allow(dead_code)]
    fn close_upvalues(&mut self, frame_id: usize) {
        // Find all open upvalues pointing to this frame
        let upvalues_to_close: Vec<Rc<LuaUpvalue>> = self
            .open_upvalues
            .iter()
            .filter(|uv| {
                if let Some(frame) = self.frames.iter().find(|f| f.frame_id == frame_id) {
                    // Check if any open upvalue points to this frame
                    for reg_idx in 0..frame.top {
                        if uv.points_to(frame_id, reg_idx) {
                            return true;
                        }
                    }
                }
                false
            })
            .cloned()
            .collect();

        // Close each upvalue
        for upvalue in upvalues_to_close.iter() {
            // Get the value from the stack before closing
            let value = upvalue.get_value(&self.frames, &self.register_stack);
            upvalue.close(value);
        }

        // Remove closed upvalues from the open list
        self.open_upvalues.retain(|uv| uv.is_open());
    }
    
    /// Close all open upvalues at or above the given stack position
    /// Used by RETURN (k bit) and CLOSE instructions
    pub fn close_upvalues_from(&mut self, stack_pos: usize) {
        let upvalues_to_close: Vec<Rc<LuaUpvalue>> = self
            .open_upvalues
            .iter()
            .filter(|uv| {
                // Check if this upvalue points to stack_pos or higher
                for frame in self.frames.iter() {
                    for reg_idx in 0..frame.top {
                        let absolute_pos = frame.base_ptr + reg_idx;
                        if absolute_pos >= stack_pos && uv.points_to(frame.frame_id, reg_idx) {
                            return true;
                        }
                    }
                }
                false
            })
            .cloned()
            .collect();

        // Close each upvalue
        for upvalue in upvalues_to_close.iter() {
            let value = upvalue.get_value(&self.frames, &self.register_stack);
            upvalue.close(value);
        }

        // Remove closed upvalues from the open list
        self.open_upvalues.retain(|uv| uv.is_open());
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
        // Get pointer from object pool for direct access
        let ptr = self
            .object_pool
            .get_string(id)
            .map(|s| Rc::as_ptr(s) as *const LuaString)
            .unwrap_or(std::ptr::null());
        LuaValue::string_id_ptr(id, ptr)
    }

    /// Get string by LuaValue (resolves ID from object pool)
    pub fn get_string(&self, value: &LuaValue) -> Option<&LuaString> {
        if let Some(id) = value.as_string_id() {
            self.object_pool.get_string(id).map(|rc| &**rc)
        } else {
            None
        }
    }

    /// Create a new table in object pool
    pub fn create_table(&mut self) -> LuaValue {
        // TODO: Auto GC causes severe performance regression
        // Need to optimize GC algorithm before enabling
        // if self.gc.should_collect() {
        //     self.collect_garbage();
        // }

        let id = self.object_pool.create_table();

        // Register with GC for manual collection
        self.gc
            .register_object(id.0, crate::gc::GcObjectType::Table);

        // Get pointer from object pool for direct access
        let ptr = self
            .object_pool
            .get_table(id)
            .map(|t| Rc::as_ptr(t) as *const std::cell::RefCell<LuaTable>)
            .unwrap_or(std::ptr::null());
        LuaValue::table_id_ptr(id, ptr)
    }

    /// Get table by LuaValue (resolves ID from object pool)
    pub fn get_table(&self, value: &LuaValue) -> Option<&std::cell::RefCell<LuaTable>> {
        if let Some(id) = value.as_table_id() {
            self.object_pool.get_table(id).map(|rc| &**rc)
        } else {
            None
        }
    }

    /// Helper: Set table field via raw_set
    pub fn table_set_raw(&mut self, table: &LuaValue, key: LuaValue, value: LuaValue) {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.borrow_mut().raw_set(key, value);
        }
    }

    /// Helper: Get table field via raw_get
    pub fn table_get_raw(&self, table: &LuaValue, key: &LuaValue) -> LuaValue {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.borrow().raw_get(key).unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        }
    }

    /// Helper: Set table metatable
    pub fn table_set_metatable(&mut self, table: &LuaValue, metatable: Option<LuaValue>) {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.borrow_mut().set_metatable(metatable);
        }
    }

    /// Helper: Get table metatable
    pub fn table_get_metatable(&self, table: &LuaValue) -> Option<LuaValue> {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.borrow().get_metatable()
        } else {
            None
        }
    }

    /// Create new userdata in object pool
    pub fn create_userdata(&mut self, data: crate::lua_value::LuaUserdata) -> LuaValue {
        let id = self.object_pool.create_userdata(data);
        // Get pointer from object pool for direct access
        let ptr = self
            .object_pool
            .get_userdata(id)
            .map(|u| Rc::as_ptr(u) as *const std::cell::RefCell<crate::lua_value::LuaUserdata>)
            .unwrap_or(std::ptr::null());
        LuaValue::userdata_id_ptr(id, ptr)
    }

    /// Get userdata by LuaValue (resolves ID from object pool)
    pub fn get_userdata(
        &self,
        value: &LuaValue,
    ) -> Option<&std::cell::RefCell<crate::lua_value::LuaUserdata>> {
        if let Some(id) = value.as_userdata_id() {
            self.object_pool.get_userdata(id).map(|rc| &**rc)
        } else {
            None
        }
    }

    /// Create a function in object pool
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalues: Vec<Rc<LuaUpvalue>>) -> LuaValue {
        let func = LuaFunction { chunk, upvalues };
        let id = self.object_pool.create_function(func);
        // Get Rc pointer from object pool - \u73b0\u5728\u6307\u9488\u7a33\u5b9a\u4e86!
        // Rc \u7684\u5185\u90e8\u6570\u636e\u4e0d\u4f1a\u56e0\u4e3a HashMap rehash \u800c\u79fb\u52a8
        let ptr = self
            .object_pool
            .get_function(id)
            .map(|rc| rc.as_ref() as *const RefCell<LuaFunction>)
            .unwrap_or(std::ptr::null());
        LuaValue::function_id_ptr(id, ptr)
    }

    /// Get function by LuaValue (resolves ID from object pool)
    pub fn get_function(&self, value: &LuaValue) -> Option<&std::rc::Rc<RefCell<LuaFunction>>> {
        if let Some(id) = value.as_function_id() {
            self.object_pool.get_function(id)
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
            Ok(func_ref.borrow().chunk.clone())
        } else {
            Err("Invalid function in current frame".to_string())
        }
    }

    /// Helper: Get upvalue from current frame's function
    #[inline]
    #[allow(dead_code)]
    fn get_current_upvalue(&self, index: usize) -> Result<std::rc::Rc<LuaUpvalue>, String> {
        let frame = self.current_frame();
        if let Some(func_ref) = self.get_function(&frame.function_value) {
            let func = func_ref.borrow();
            if index < func.upvalues.len() {
                Ok(func.upvalues[index].clone())
            } else {
                Err(format!("Invalid upvalue index: {}", index))
            }
        } else {
            Err("Invalid function in current frame".to_string())
        }
    }

    /// Check if GC should run and collect garbage if needed
    #[allow(unused)]
    fn maybe_collect_garbage(&mut self) {
        if self.gc.should_collect() {
            self.collect_garbage();
        }
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
                                Some(func_ref.borrow().chunk.clone())
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
        roots.push(self.globals);

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
        for upvalue in &self.open_upvalues {
            if let Some(value) = upvalue.get_closed_value() {
                roots.push(value);
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

    /// Try to get a metamethod from a value
    fn get_metamethod(&mut self, value: &LuaValue, event: &str) -> Option<LuaValue> {
        match value.kind() {
            LuaValueKind::Table => {
                if let Some(table_id) = value.as_table_id() {
                    let metatable = {
                        let table = self.object_pool.get_table(table_id).expect("invalid table");
                        table.borrow().get_metatable()
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
                if let Some(mt) = &self.string_metatable {
                    if let Some(mt_ref) = self.get_table(mt) {
                        mt_ref.borrow().raw_get(&key)
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
                let func_id = metamethod
                    .as_function_id()
                    .ok_or(LuaError::RuntimeError("Invalid function ID".to_string()))?;
                let max_stack_size =
                    {
                        let func_ref = self.object_pool.get_function(func_id).ok_or(
                            LuaError::RuntimeError("Invalid function reference".to_string()),
                        )?;
                        func_ref.borrow().chunk.max_stack_size
                    };

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
                Err(LuaError::RuntimeError(
                    "`__tostring` metamethod did not return a string".to_string(),
                ))
            }
        } else {
            Ok(value.to_string_repr())
        }
    }

    /// Generate a stack traceback string
    pub fn generate_traceback(&self, error_msg: &str) -> String {
        let mut trace = format!("{}\nstack traceback:", error_msg);

        // Iterate through call frames from top to bottom (most recent first)
        for frame in self.frames.iter().rev() {
            // Dynamically resolve chunk for debug info
            let (source, line) = if let Some(func_ref) = self.get_function(&frame.function_value) {
                let func = func_ref.borrow();
                let chunk = &func.chunk;

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
    pub fn protected_call(&mut self, func: LuaValue, args: Vec<LuaValue>) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Save current state
        let initial_frame_count = self.frames.len();

        // Try to call the function
        let result = self.call_function_internal(func, args);

        match result {
            Ok(return_values) => {
                // Success: return true and the return values
                Ok((true, return_values))
            }
            Err(LuaError::Yield(values)) => {
                // Yield is not an error - propagate it
                Err(LuaError::Yield(values))
            }
            Err(error_msg) => {
                // Real error: clean up frames and return false with error message
                // Simply clear all open upvalues to avoid dangling references
                self.open_upvalues.clear();

                // Now pop the frames
                while self.frames.len() > initial_frame_count {
                    self.frames.pop();
                }

                // Return error without traceback for now (can add later)
                let error_str = self.create_string(&format!("{}", error_msg));

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
        eprintln!("[xpcall] protected_call_with_handler called");
        eprintln!("[xpcall] err_handler type: {:?}", err_handler.kind());
        
        let old_handler = self.error_handler.clone();
        self.error_handler = Some(err_handler.clone());

        let initial_frame_count = self.frames.len();
        eprintln!("[xpcall] initial_frame_count: {}", initial_frame_count);

        let result = self.call_function_internal(func, args);
        eprintln!("[xpcall] call_function_internal result: {:?}", result.is_ok());

        self.error_handler = old_handler;

        match result {
            Ok(values) => {
                eprintln!("[xpcall] Success, returning {} values", values.len());
                Ok((true, values))
            }
            Err(LuaError::Yield(values)) => {
                eprintln!("[xpcall] Yield encountered");
                // Yield is not an error - propagate it
                Err(LuaError::Yield(values))
            }
            Err(err_msg) => {
                eprintln!("[xpcall] Error encountered: {:?}", err_msg);
                
                // Clean up frames created by the failed function call
                while self.frames.len() > initial_frame_count {
                    let frame = self.frames.pop().unwrap();
                    // Close upvalues belonging to this frame
                    self.close_upvalues_from(frame.base_ptr);
                }
                
                eprintln!("[xpcall] Calling error handler");
                let err_str = self.create_string(&format!("{}", err_msg));
                let handler_result = self.call_function_internal(err_handler, vec![err_str]);
                eprintln!("[xpcall] Handler result: {:?}", handler_result.is_ok());

                match handler_result {
                    Ok(handler_values) => Ok((false, handler_values)),
                    Err(LuaError::Yield(values)) => {
                        // Yield from error handler - propagate it
                        Err(LuaError::Yield(values))
                    }
                    Err(_) => {
                        let err_str =
                            self.create_string(&format!("Error in error handler: {}", err_msg));
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

                // Allocate registers in global stack
                let new_base = self.register_stack.len();
                let stack_size = 16; // enough for most cfunc calls
                self.ensure_stack_capacity(new_base + stack_size);

                self.register_stack[new_base] = func;
                for (i, arg) in args.iter().enumerate() {
                    if i + 1 < stack_size {
                        self.register_stack[new_base + i + 1] = arg.clone();
                    }
                }

                // Create dummy function and add to object pool
                let dummy_func = LuaFunction {
                    chunk: Rc::new(Chunk {
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
                    }),
                    upvalues: Vec::new(),
                };
                let dummy_func_id = self.object_pool.create_function(dummy_func);
                let dummy_func_value = LuaValue::function_id(dummy_func_id);

                let temp_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    dummy_func_value,
                    new_base,
                    stack_size,
                    0,
                    0,
                );

                self.frames.push(temp_frame);

                // Call CFunction - ensure frame is always popped even on error
                let result = match cfunc(self) {
                    Ok(r) => Ok(r),
                    Err(LuaError::Yield(values)) => {
                        // CFunction yielded - this is valid for coroutine.yield
                        // Don't pop frame, just return the yield
                        return Err(LuaError::Yield(values));
                    }
                    Err(e) => {
                        self.frames.pop();
                        return Err(e);
                    }
                };

                self.frames.pop();

                Ok(result?.all_values())
            }
            LuaValueKind::Function => {
                let lua_func_id = func.as_function_id().unwrap();

                // Get max_stack_size before entering the execution loop
                let max_stack_size = {
                    let lua_func_ref = self.object_pool.get_function(lua_func_id).ok_or(
                        LuaError::RuntimeError("Invalid function reference".to_string()),
                    )?;
                    let size = lua_func_ref.borrow().chunk.max_stack_size;
                    // Ensure at least 1 register for function body
                    if size == 0 { 1 } else { size }
                };

                // For Lua function, use similar logic to call_metamethod
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let new_base = self.register_stack.len();
                self.ensure_stack_capacity(new_base + max_stack_size);

                for (i, arg) in args.iter().enumerate() {
                    if i < max_stack_size {
                        self.register_stack[new_base + i] = arg.clone();
                    }
                }

                let new_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    func,
                    new_base,
                    max_stack_size,
                    0,
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
                        func_ref.borrow().chunk.clone()
                    } else {
                        break Err(LuaError::RuntimeError(
                            "Invalid function in frame".to_string(),
                        ));
                    };

                    if pc >= chunk.code.len() {
                        // End of code
                        self.frames.pop();
                        break Ok(());
                    }

                    let instr = chunk.code[pc];
                    self.frames[frame_idx].pc += 1;

                    // Dispatch instruction
                    match crate::lua_vm::dispatcher::dispatch_instruction(self, instr) {
                        Ok(action) => {
                            use crate::lua_vm::dispatcher::DispatchAction;
                            match action {
                                DispatchAction::Continue => {
                                    // Continue to next instruction
                                },
                                DispatchAction::Skip(n) => {
                                    // Skip N additional instructions
                                    self.frames[frame_idx].pc += n;
                                },
                                DispatchAction::Return => {
                                    // Frame will be popped, loop will exit
                                },
                                DispatchAction::Yield => {
                                    // Yield detected - propagate it up
                                    break Err(LuaError::Yield(self.return_values.clone()));
                                },
                                DispatchAction::Call => {
                                    // CALL instruction already set up the frame
                                    // Continue execution in the new frame
                                },
                            }
                        },
                        Err(e) => {
                            // Real error occurred
                            eprintln!("[call_function_internal] Error during execution: {:?}", e);
                            break Err(e);
                        }
                    }
                };

                match exec_result {
                    Ok(_) => {
                        // Get return values
                        let result = self.return_values.clone();
                        self.return_values.clear();
                        eprintln!("[call_function_internal] Lua function returned {} values", result.len());
                        Ok(result)
                    }
                    Err(e) => {
                        eprintln!("[call_function_internal] Returning error: {:?}", e);
                        Err(e)
                    },
                }
            }
            _ => Err(LuaError::RuntimeError(
                "attempt to call a non-function value".to_string(),
            )),
        }
    }
}
