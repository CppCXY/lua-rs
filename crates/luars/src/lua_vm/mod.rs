// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
mod execute;
mod lua_call_frame;
mod lua_error;
mod opcode;

use crate::compiler::{compile_code, compile_code_with_name};
use crate::gc::{GC, GcFunction, GcId, TableId, UpvalueId};
use crate::lua_value::{
    CFunction, Chunk, CoroutineStatus, LuaString, LuaTable, LuaThread, LuaUserdata, LuaValue,
    LuaValueKind,
};
pub use crate::lua_vm::lua_call_frame::LuaCallFrame;
pub use crate::lua_vm::lua_error::LuaError;
use crate::{ObjectPool, lib_registry};
pub use opcode::{Instruction, OpCode};
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

    // Object pool for unified object management (new architecture)
    // Placed near top for cache locality with hot operations
    pub(crate) object_pool: ObjectPool,

    // Garbage collector (cold path - only accessed during actual GC)
    pub(crate) gc: GC,

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
            global: TableId(0),   // Will be initialized below
            registry: TableId(1), // Will be initialized below
            // gc_roots_buffer: Vec::with_capacity(512), // Pre-allocate roots buffer
            // frames,
            // frame_count: 0,
            // register_stack: Vec::with_capacity(256), // Pre-allocate for initial stack
            // object_pool: ObjectPool::new(),
            // gc: GC::new(),
            // return_values: Vec::with_capacity(16),
            // open_upvalues: Vec::new(),
            // to_be_closed: Vec::new(),
            // next_frame_id: 0,
            // error_handler: None,
            // #[cfg(feature = "loadlib")]
            // ffi_state: crate::ffi::FFIState::new(),
            // current_thread: None,
            // current_thread_id: None,
            // current_thread_value: None,
            // main_thread_value: None, // Will be initialized lazily
            // string_metatable: None,
            // c_closure_upvalues_ptr: std::ptr::null(),
            // c_closure_upvalues_len: 0,
            // c_closure_inline_upvalue: LuaValue::nil(),
            // #[cfg(feature = "async")]
            // async_executor: AsyncExecutor::new(),
            // Initialize error storage
            error_message: String::new(),
            yield_values: Vec::new(),
            object_pool: ObjectPool::new(),
            gc: GC::new(),
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
        todo!()
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
        // TODO: The full execute module is commented out
        // For now, return a placeholder to avoid panic
        todo!()
    }

    pub fn get_global(&mut self, name: &str) -> Option<LuaValue> {
        let key = self.create_string(name);

        let global = self.object_pool.get_table(self.global)?;
        global.raw_get(&key)
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        let key = self.create_string(name);

        if let Some(global) = self.object_pool.get_table_mut(self.global) {
            global.raw_set(key.clone(), value.clone());
        }
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

    /// Resume a coroutine using ThreadId-based LuaValue
    /// ULTRA-OPTIMIZED: Minimized object_pool lookups using raw pointers
    pub fn resume_thread(
        &mut self,
        thread_val: LuaValue,
        _args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Get ThreadId from LuaValue
        let Some(thread_id) = thread_val.as_thread_id() else {
            return Err(self.error("invalid thread".to_string()));
        };

        todo!()
    }

    /// Yield from current coroutine
    /// Returns Err(LuaError::Yield) which will be caught by run() loop
    pub fn yield_thread(&mut self, values: Vec<LuaValue>) -> LuaResult<()> {
        todo!()
    }

    /// Fast table get - NO metatable support!
    /// Use this for normal field access (GETFIELD, GETTABLE, GETI)
    /// This is the correct behavior for Lua bytecode instructions
    /// Only use table_get_with_meta when you explicitly need __index metamethod
    #[inline(always)]
    pub fn table_get(&self, _table_value: &LuaValue, _key: &LuaValue) -> Option<LuaValue> {
        None
    }

    /// Get value from table with metatable support (__index metamethod)
    /// Use this for GETTABLE, GETFIELD, GETI instructions
    /// For raw access without metamethods, use table_get_raw() instead
    pub fn table_get_with_meta(
        &mut self,
        _table_value: &LuaValue,
        _key: &LuaValue,
    ) -> Option<LuaValue> {
        None
    }

    /// Get value from userdata with metatable support
    /// Handles __index metamethod
    pub fn userdata_get(&mut self, userdata_value: &LuaValue, _key: &LuaValue) -> Option<LuaValue> {
        let Some(userdata_id) = userdata_value.as_userdata_id() else {
            return None;
        };

        // Get metatable from userdata
        let _metatable = {
            let userdata = self.object_pool.get_userdata(userdata_id)?;
            userdata.get_metatable()
        };

        None
    }

    /// Get value from string with metatable support
    /// Handles __index metamethod for strings
    pub fn string_get(&mut self, _string_val: &LuaValue, _key: &LuaValue) -> Option<LuaValue> {
        None
    }

    /// Set value in table with metatable support (__newindex metamethod)
    /// Use this for SETTABLE, SETFIELD, SETI instructions
    /// For raw set without metamethods, use table_set_raw() instead
    pub fn table_set_with_meta(
        &mut self,
        lua_table_val: LuaValue,
        _key: LuaValue,
        _value: LuaValue,
    ) -> LuaResult<()> {
        // Use ObjectPool lookup
        let Some(table_id) = lua_table_val.as_table_id() else {
            return Err(self.error("table_set: not a table".to_string()));
        };

        Ok(())
    }

    /// Call a Lua value (function or CFunction) with the given arguments
    /// Returns the first return value, or None if the call fails
    pub fn call_metamethod(
        &mut self,
        _func: &LuaValue,
        _args: &[LuaValue],
    ) -> LuaResult<Option<LuaValue>> {
        todo!()
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

    pub fn table_set_raw(&mut self, table: &LuaValue, key: LuaValue, value: LuaValue) {
        if let Some(table_ref) = self.get_table_mut(table) {
            table_ref.raw_set(key, value);
        }
    }

    pub fn table_get_raw(&self, table: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.raw_get(key)
        } else {
            None
        }
    }

    pub fn table_set_metatable(&mut self, table: &LuaValue, metatable: Option<LuaValue>) {
        if let Some(table_ref) = self.get_table_mut(table) {
            table_ref.set_metatable(metatable);
        }
    }

    /// Create new userdata in object pool
    pub fn create_userdata(&mut self, data: LuaUserdata) -> LuaValue {
        let id = self.object_pool.create_userdata(data);
        LuaValue::userdata(id)
    }

    /// Get userdata by LuaValue (resolves ID from object pool)
    pub fn get_userdata(&self, value: &LuaValue) -> Option<&LuaUserdata> {
        if let Some(id) = value.as_userdata_id() {
            self.object_pool.get_userdata(id)
        } else {
            None
        }
    }

    /// Get mutable userdata by LuaValue
    pub fn get_userdata_mut(&mut self, value: &LuaValue) -> Option<&mut LuaUserdata> {
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
    pub fn create_c_closure(&mut self, func: CFunction, upvalues: Vec<LuaValue>) -> LuaValue {
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
        // // Collect roots using pre-allocated buffer (avoid allocation)
        // self.gc_roots_buffer.clear();

        // // 1. Global table
        // self.gc_roots_buffer.push(LuaValue::table(self.global));

        // // 2. Registry table (persistent objects storage)
        // self.gc_roots_buffer.push(LuaValue::table(self.registry));

        // // 3. String metatable
        // if let Some(mt) = &self.string_metatable {
        //     self.gc_roots_buffer.push(*mt);
        // }

        // // 4. ALL frame registers AND function values (not just current frame)
        // // This is critical - any register in any active frame must be kept alive
        // // Also, the function being executed in each frame must be kept alive!
        // for frame in &self.frames[..self.frame_count] {
        //     // Add the function value for this frame - this is CRITICAL!
        //     self.gc_roots_buffer.push(frame.as_function_value());

        //     let base_ptr = frame.base_ptr as usize;
        //     let top = frame.top as usize;
        //     for i in 0..top {
        //         if base_ptr + i < self.register_stack.len() {
        //             let value = self.register_stack[base_ptr + i];
        //             // Skip nil values - they don't need to be roots
        //             if !value.is_nil() {
        //                 self.gc_roots_buffer.push(value);
        //             }
        //         }
        //     }
        // }

        // // 5. All registers beyond the last frame's top (temporary values)
        // // NOTE: Only scan up to a reasonable limit to avoid scanning stale registers
        // if self.frame_count > 0 {
        //     let last_frame = &self.frames[self.frame_count - 1];
        //     let last_frame_end = last_frame.base_ptr as usize + last_frame.top as usize;
        //     // Limit scan to avoid excessive GC work on large register stacks
        //     let scan_limit = (last_frame_end + 128).min(self.register_stack.len());
        //     for i in last_frame_end..scan_limit {
        //         let value = self.register_stack[i];
        //         if !value.is_nil() {
        //             self.gc_roots_buffer.push(value);
        //         }
        //     }
        // } else {
        //     // No frames? Scan limited portion
        //     let scan_limit = 256.min(self.register_stack.len());
        //     for i in 0..scan_limit {
        //         let value = self.register_stack[i];
        //         if !value.is_nil() {
        //             self.gc_roots_buffer.push(value);
        //         }
        //     }
        // }

        // // 6. Return values
        // for value in &self.return_values {
        //     self.gc_roots_buffer.push(*value);
        // }

        // // 7. Open upvalues - these point to stack locations that must stay alive
        // for upval_id in &self.open_upvalues {
        //     if let Some(uv) = self.object_pool.get_upvalue(*upval_id) {
        //         if let Some(val) = uv.get_closed_value() {
        //             self.gc_roots_buffer.push(val);
        //         }
        //     }
        // }

        // // Perform GC step with complete root set
        // self.gc.step(&self.gc_roots_buffer, &mut self.object_pool);
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
        // // Collect all roots
        // let mut roots = Vec::new();

        // // Add the global table itself as a root
        // roots.push(LuaValue::table(self.global));

        // // Add registry table as a root (persistent objects)
        // roots.push(LuaValue::table(self.registry));

        // // Add string metatable if present
        // if let Some(mt) = &self.string_metatable {
        //     roots.push(*mt);
        // }

        // // Add all frame registers AND function values as roots
        // for frame in &self.frames[..self.frame_count] {
        //     // CRITICAL: Add the function being executed
        //     roots.push(frame.as_function_value());

        //     let base_ptr = frame.base_ptr as usize;
        //     let top = frame.top as usize;
        //     for i in 0..top {
        //         if base_ptr + i < self.register_stack.len() {
        //             roots.push(self.register_stack[base_ptr + i]);
        //         }
        //     }
        // }

        // // Add return values as roots
        // for value in &self.return_values {
        //     roots.push(value.clone());
        // }

        // // Add open upvalues as roots (only closed ones that have values)
        // for upvalue_id in &self.open_upvalues {
        //     if let Some(uv) = self.object_pool.get_upvalue(*upvalue_id) {
        //         if let Some(value) = uv.get_closed_value() {
        //             roots.push(value);
        //         }
        //     }
        // }

        // // Run GC with mutable object pool reference
        // self.gc.collect(&roots, &mut self.object_pool);
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
            LuaValueKind::String => None,
            // TODO: Support metatables for userdata
            _ => None,
        }
    }

    /// Generate a stack traceback string
    pub fn generate_traceback(&self, error_msg: &str) -> String {
        // let mut trace = format!("{}\nstack traceback:", error_msg);

        // // Iterate through active call frames from innermost to outermost (most recent first)
        // // frame_count is the number of active frames
        // if self.frame_count == 0 {
        //     return trace;
        // }

        // // Traverse from innermost (frame_count - 1) to outermost (0)
        // for i in (0..self.frame_count).rev() {
        //     let frame = &self.frames[i];

        //     // Get source location info
        //     let (source, line) = if frame.is_lua() {
        //         // Get function ID and chunk info
        //         if let Some(func_id) = frame.get_function_id() {
        //             if let Some(func) = self.object_pool.get_function(func_id) {
        //                 if let Some(chunk) = func.chunk() {
        //                     let source_str = chunk.source_name.as_deref().unwrap_or("?");

        //                     // Get line number from pc (pc points to next instruction, so use pc-1)
        //                     let pc = frame.pc.saturating_sub(1) as usize;
        //                     let line_str =
        //                         if !chunk.line_info.is_empty() && pc < chunk.line_info.len() {
        //                             chunk.line_info[pc].to_string()
        //                         } else {
        //                             "?".to_string()
        //                         };

        //                     (source_str.to_string(), line_str)
        //                 } else {
        //                     // C closure with upvalues
        //                     ("[C closure]".to_string(), "?".to_string())
        //                 }
        //             } else {
        //                 ("?".to_string(), "?".to_string())
        //             }
        //         } else {
        //             ("?".to_string(), "?".to_string())
        //         }
        //     } else {
        //         // C function
        //         ("[C]".to_string(), "?".to_string())
        //     };

        //     trace.push_str(&format!("\n\t{}:{}: in function", source, line));
        // }

        // trace
        todo!()
    }

    /// Execute a function with protected call (pcall semantics)
    /// Note: Yields are NOT caught by pcall - they propagate through
    pub fn protected_call(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // // Save current state
        // let initial_frame_count = self.frame_count;

        // // Try to call the function
        // let result = self.call_function_internal(func, args);

        // match result {
        //     Ok(return_values) => {
        //         // Success: return true and the return values
        //         Ok((true, return_values))
        //     }
        //     Err(LuaError::Yield) => {
        //         // Yield is not an error - propagate it
        //         Err(LuaError::Yield)
        //     }
        //     Err(_) => {
        //         // Real error: clean up frames and return false with error message
        //         // Simply clear all open upvalues to avoid dangling references
        //         self.open_upvalues.clear();

        //         // Now pop the frames
        //         while self.frame_count > initial_frame_count {
        //             self.pop_frame_discard();
        //         }

        //         // Return error - take the message to avoid allocation
        //         let msg = std::mem::take(&mut self.error_message);
        //         let error_str = self.create_string(&msg);

        //         Ok((false, vec![error_str]))
        //     }
        // }
        todo!()
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
        // // Save current state
        // let initial_frame_count = self.frame_count;

        // // Call function directly without Vec allocation
        // let result = self.call_function_stack_based(func, arg_base, arg_count);

        // match result {
        //     Ok(result_count) => Ok((true, result_count)),
        //     Err(LuaError::Yield) => Err(LuaError::Yield),
        //     Err(_) => {
        //         // Error path: clean up and return error message
        //         self.open_upvalues.clear();
        //         while self.frame_count > initial_frame_count {
        //             self.pop_frame_discard();
        //         }
        //         let msg = std::mem::take(&mut self.error_message);
        //         let error_str = self.create_string(&msg);
        //         self.return_values.clear();
        //         self.return_values.push(error_str);
        //         Ok((false, 1))
        //     }
        // }
        todo!()
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
        // let initial_frame_count = self.frame_count;

        // // Call the function
        // let result = self.call_function_internal(func, args);

        // match result {
        //     Ok(values) => Ok((true, values)),
        //     Err(LuaError::Yield) => Err(LuaError::Yield),
        //     Err(_) => {
        //         // Error occurred - call the error handler NOW while the stack is still intact
        //         // This allows debug.traceback() to see the full call stack
        //         let error_msg = self.error_message.clone();
        //         let err_value = self.create_string(&error_msg);

        //         let handled_msg = match self.call_function_internal(err_handler, vec![err_value]) {
        //             Ok(handler_results) => {
        //                 // Error handler succeeded, use its return value as the error message
        //                 if let Some(result) = handler_results.first() {
        //                     self.value_to_string_raw(result)
        //                 } else {
        //                     error_msg
        //                 }
        //             }
        //             Err(_) => {
        //                 // Error handler itself failed
        //                 format!("error in error handling: {}", error_msg)
        //             }
        //         };

        //         // NOW clean up frames created by the failed function call
        //         while self.frame_count > initial_frame_count {
        //             let frame = self.pop_frame().unwrap();
        //             // Close upvalues belonging to this frame
        //             self.close_upvalues_from(frame.base_ptr as usize);
        //         }

        //         // Return the handled error message
        //         let err_str = self.create_string(&handled_msg);
        //         Ok((false, vec![err_str]))
        //     }
        // }
        todo!()
    }
}
