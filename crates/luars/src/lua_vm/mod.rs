// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
mod call_info;
mod const_string;
mod execute;
mod lua_error;
mod lua_state;
pub mod opcode;
mod safe_option;

use crate::compiler::{compile_code, compile_code_with_name};
use crate::gc::GC;
use crate::lua_value::{Chunk, LuaUserdata, LuaValue};
pub use crate::lua_vm::call_info::CallInfo;
use crate::lua_vm::const_string::ConstString;
use crate::lua_vm::execute::lua_execute;
pub use crate::lua_vm::lua_error::LuaError;
pub use crate::lua_vm::lua_state::LuaState;
pub use crate::lua_vm::safe_option::SafeOption;
use crate::stdlib::Stdlib;
use crate::{GcKind, ObjectAllocator, ThreadPtr, UpvaluePtr, lib_registry};
pub use execute::TmKind;
pub use execute::{get_metamethod_event, get_metatable};
pub use opcode::{Instruction, OpCode};
use std::rc::Rc;

pub type LuaResult<T> = Result<T, LuaError>;
/// C Function type - Rust function callable from Lua
/// Now takes LuaContext instead of LuaVM for better ergonomics
pub type CFunction = fn(&mut LuaState) -> LuaResult<usize>;

/// Global VM state (equivalent to global_State in Lua C API)
/// Manages global resources shared by all execution threads/coroutines
pub struct LuaVM {
    /// Global environment table (_G and _ENV point to this)
    pub(crate) global: LuaValue,

    /// Registry table (like Lua's LUA_REGISTRYINDEX)
    pub(crate) registry: LuaValue,

    /// Object pool for unified object management
    pub(crate) object_allocator: ObjectAllocator,

    /// Garbage collector state
    pub(crate) gc: GC,

    /// Main thread execution state (embedded)
    pub(crate) main_state: ThreadPtr,

    #[allow(unused)]
    /// String metatable (shared by all strings)
    pub(crate) string_mt: Option<LuaValue>,

    pub(crate) safe_option: SafeOption,

    pub const_strings: ConstString,
}

impl LuaVM {
    pub fn new(option: SafeOption) -> Box<Self> {
        let mut gc = GC::new();
        let mut object_allocator = ObjectAllocator::new(option.clone());
        let cs = ConstString::new(&mut object_allocator, &mut gc);
        let mut vm = Box::new(LuaVM {
            global: LuaValue::nil(),
            registry: LuaValue::nil(),
            object_allocator,
            gc,
            main_state: ThreadPtr::null(), //,
            string_mt: None,
            safe_option: option.clone(),
            const_strings: cs,
        });

        let ptr_vm = vm.as_mut() as *mut LuaVM;
        let current_white = vm.gc.current_white;
        // Set LuaVM pointer in main_state
        let thread_value = vm.object_allocator.create_thread(
            &mut vm.gc,
            LuaState::new(6, ptr_vm, true, option.clone()),
            current_white,
        );

        vm.main_state = thread_value.as_thread_ptr().unwrap();

        // Initialize registry (like Lua's init_registry)
        // Registry is a GC root and protects all values stored in it
        let registry = vm.create_table(2, 8);
        vm.registry = registry;

        // Set _G to point to the global table itself
        let globals_value = vm.create_table(0, 20);
        vm.global = globals_value;
        vm.set_global("_G", globals_value);
        vm.set_global("_ENV", globals_value);

        // Store globals in registry (like Lua's LUA_RIDX_GLOBALS)
        vm.registry_seti(1, globals_value);

        vm
    }

    pub fn main_state(&mut self) -> &mut LuaState {
        &mut self.main_state.as_mut_ref().data
    }

    pub fn main_state_ref(&self) -> &LuaState {
        &self.main_state.as_ref().data
    }

    /// Set a value in the registry by integer key
    pub fn registry_seti(&mut self, key: i64, value: LuaValue) {
        self.raw_seti(&self.registry.clone(), key, value);
    }

    /// Get a value from the registry by integer key
    pub fn registry_geti(&self, key: i64) -> Option<LuaValue> {
        self.raw_geti(&self.registry, key)
    }

    /// Set a value in the registry by string key
    pub fn registry_set(&mut self, key: &str, value: LuaValue) {
        let key_value = self.create_string(key);

        // Use VM table_set so we always run the GC barrier
        let registry = self.registry;
        self.raw_set(&registry, key_value, value);
    }

    /// Get a value from the registry by string key
    pub fn registry_get(&mut self, key: &str) -> Option<LuaValue> {
        let key = self.create_string(key);
        self.raw_get(&self.registry, &key)
    }

    pub fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()> {
        lib_registry::create_standard_registry(lib).load_all(self)?;
        Ok(())
    }

    /// Execute a chunk in the main thread
    pub fn execute(&mut self, chunk: Rc<Chunk>) -> LuaResult<Vec<LuaValue>> {
        // Main chunk needs _ENV upvalue pointing to global table
        // This matches Lua 5.4+ behavior where all chunks have _ENV as upvalue[0]
        let env_upvalue_id = self.create_upvalue_closed(self.global);
        let func = self.create_function(chunk, vec![env_upvalue_id]);
        self.execute_function(func, vec![])
    }

    pub fn execute_string(&mut self, source: &str) -> LuaResult<Vec<LuaValue>> {
        let chunk = self.compile(source)?;
        self.execute(Rc::new(chunk))
    }

    /// Execute a function with arguments
    pub(crate) fn execute_function(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<Vec<LuaValue>> {
        // Save function index - will be at current logical top
        let main_state = self.main_state();
        let func_idx = main_state.get_top();
        let nargs = args.len();

        // Push function onto stack (updates stack_top)
        main_state.push_value(func.clone())?;

        // Push arguments (each updates stack_top)
        for arg in args {
            main_state.push_value(arg)?;
        }

        // Create initial call frame
        // base points to first argument (func_idx + 1), following Lua convention
        let base = func_idx + 1;
        // Top-level call expects multiple return values
        main_state.push_frame(func, base, nargs, -1)?;

        // Run the VM execution loop
        let results = self.run()?;

        // Reset logical stack top for next execution
        self.main_state().set_top(0);

        Ok(results)
    }

    /// Main VM execution loop (equivalent to luaV_execute)
    fn run(&mut self) -> LuaResult<Vec<LuaValue>> {
        lua_execute(self.main_state())?;

        let main_state = self.main_state();
        // Collect all values from logical stack (0 to stack_top) as return values
        let mut results = Vec::new();
        let top = main_state.get_top();
        for i in 0..top {
            if let Some(val) = main_state.stack_get(i) {
                results.push(val);
            }
        }

        // Check GC after VM execution completes (like Lua's luaC_checkGC after returning to caller)
        // At this point, all return values are collected and safe from collection
        main_state.check_gc()?;

        Ok(results)
    }

    /// Compile source code using VM's string pool
    pub fn compile(&mut self, source: &str) -> LuaResult<Chunk> {
        let chunk = match compile_code(source, self) {
            Ok(c) => c,
            Err(e) => return Err(self.compile_error(e)),
        };

        Ok(chunk)
    }

    pub fn compile_with_name(&mut self, source: &str, chunk_name: &str) -> LuaResult<Chunk> {
        let chunk = match compile_code_with_name(source, self, chunk_name) {
            Ok(c) => c,
            Err(e) => return Err(self.compile_error(e)),
        };

        Ok(chunk)
    }

    pub fn get_global(&mut self, name: &str) -> Option<LuaValue> {
        let key = self.create_string(name);
        self.raw_get(&self.global, &key)
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        let key = self.create_string(name);

        // Use VM table_set so we always run the GC barrier
        let global = self.global;
        self.raw_set(&global, key, value);
    }

    /// Set the metatable for all strings
    /// This allows string methods to be called with : syntax (e.g., str:upper())
    pub fn set_string_metatable(&mut self, string_lib_table: LuaValue) {
        // Create a metatable with __index pointing to the string library
        let mt_value = self.create_table(0, 1);

        // Set __index to point to the string library
        let index_key = self.const_strings.tm_index;
        self.raw_set(&mt_value, index_key, string_lib_table);

        // Store in the VM
        self.string_mt = Some(mt_value);
    }

    // ============ Coroutine Support ============

    /// Create a new thread (coroutine) - returns ThreadId-based LuaValue
    /// OPTIMIZED: Minimal initial allocations - grows on demand
    pub fn create_thread(&mut self, func: LuaValue) -> LuaValue {
        // Create a new LuaState for the coroutine
        let mut thread = LuaState::new(1, self as *mut LuaVM, false, self.safe_option.clone());

        // Push the function onto the thread's stack (updates stack_top)
        // It will be used when resume() is first called
        thread
            .push_value(func)
            .expect("Failed to push function onto coroutine stack");

        // Create thread in ObjectPool and return LuaValue
        let current_white = self.gc.current_white;
        let value = self
            .object_allocator
            .create_thread(&mut self.gc, thread, current_white);
        value
    }

    /// Resume a coroutine - DEPRECATED: Use thread_state.resume() instead
    /// This method is kept for backward compatibility but delegates to LuaState
    pub fn resume_thread(
        &mut self,
        thread_val: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Get ThreadId from LuaValue
        let Some(l) = thread_val.as_thread_mut() else {
            return Err(self.error("invalid thread".to_string()));
        };

        if l.is_main_thread() {
            return Err(self.error("cannot resume main thread".to_string()));
        }

        // Borrow mutably and delegate to LuaState::resume
        l.resume(args)
    }

    /// Fast table get - NO metatable support!
    /// Use this for normal field access (GETFIELD, GETTABLE, GETI)
    /// This is the correct behavior for Lua bytecode instructions
    /// Only use table_get_with_meta when you explicitly need __index metamethod
    #[inline(always)]
    pub fn raw_get(&self, table_value: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        let table = table_value.as_table()?;
        table.raw_get(key)
    }

    #[inline]
    pub fn raw_set(&mut self, table_value: &LuaValue, key: LuaValue, value: LuaValue) -> bool {
        let Some(table) = table_value.as_table_mut() else {
            return false;
        };
        let new_key = table.raw_set(&key, value);
        let mut need_barrier = false;
        if new_key && key.is_collectable() {
            // New key inserted - run GC barrier
            need_barrier = true;
        }

        if value.is_collectable() {
            need_barrier = true;
        }

        // GC backward barrier (luaC_barrierback)
        // Tables use backward barrier since they may be modified many times
        if need_barrier {
            self.gc.barrier_back(table_value.as_gc_ptr().unwrap());
        }
        true
    }

    #[inline(always)]
    pub fn raw_geti(&self, table_value: &LuaValue, key: i64) -> Option<LuaValue> {
        let table = table_value.as_table()?;
        table.raw_geti(key)
    }

    pub fn raw_seti(&mut self, table_value: &LuaValue, key: i64, value: LuaValue) -> bool {
        let Some(table) = table_value.as_table_mut() else {
            return false;
        };
        table.raw_seti(key, value.clone());

        // GC backward barrier (luaC_barrierback)
        // Tables use backward barrier since they may be modified many times
        if value.is_collectable() {
            self.gc.barrier_back(table_value.as_gc_ptr().unwrap());
        }
        true
    }

    /// Create a new table and register it with GC
    /// Create a string and register it with GC
    /// For short strings (4 bytes), use interning (global deduplication)
    /// Create a string value with automatic interning for short strings
    /// Returns LuaValue directly with ZERO allocation overhead for interned strings
    ///
    /// Performance characteristics:
    /// - Cache hit (interned): O(1) hash lookup, 0 allocations, 0 atomic ops
    /// - Cache miss (new): 1 Box allocation, GC registration, pool insertion
    /// - Long string: 1 Box allocation, GC registration, no pooling
    pub fn create_string(&mut self, s: &str) -> LuaValue {
        let current_white = self.gc.current_white;
        let value = self
            .object_allocator
            .create_string(&mut self.gc, s, current_white);
        value
    }

    pub fn create_binary(&mut self, data: Vec<u8>) -> LuaValue {
        let current_white = self.gc.current_white;
        let value = self
            .object_allocator
            .create_binary(&mut self.gc, data, current_white);
        value
    }

    /// Create string from owned String (avoids clone for non-interned strings)
    #[inline]
    pub fn create_string_owned(&mut self, s: String) -> LuaValue {
        let current_white = self.gc.current_white;
        let value = self
            .object_allocator
            .create_string_owned(&mut self.gc, s, current_white);
        value
    }

    /// Create substring (optimized for string.sub)
    #[inline]
    pub fn create_substring(&mut self, s_value: LuaValue, start: usize, end: usize) -> LuaValue {
        let current_white = self.gc.current_white;
        let value = self.object_allocator.create_substring(
            &mut self.gc,
            s_value,
            start,
            end,
            current_white,
        );
        value
    }

    // ============ Legacy GC Barrier Methods (deprecated) ============

    /// Create a new table in object pool
    /// GC tracks objects via ObjectPool iteration, no allgc list needed
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> LuaValue {
        let current_white = self.gc.current_white;
        let value =
            self.object_allocator
                .create_table(&mut self.gc, array_size, hash_size, current_white);
        value
    }

    /// Create new userdata in object pool
    pub fn create_userdata(&mut self, data: LuaUserdata) -> LuaValue {
        let current_white = self.gc.current_white;
        let value = self
            .object_allocator
            .create_userdata(&mut self.gc, data, current_white);
        value
    }

    /// Create a function in object pool
    /// Tracks the object in GC's allgc list for efficient sweep
    #[inline(always)]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalue_ids: Vec<UpvaluePtr>) -> LuaValue {
        let current_white = self.gc.current_white;
        let value =
            self.object_allocator
                .create_function(&mut self.gc, chunk, upvalue_ids, current_white);
        value
    }

    /// Create a C closure (native function with upvalues stored as closed upvalues)
    /// The upvalues are automatically created as closed upvalues with the given values
    #[inline]
    pub fn create_c_closure(&mut self, func: CFunction, upvalues: Vec<LuaValue>) -> LuaValue {
        // Create closed upvalues for each value
        let upvalue_ids: Vec<UpvaluePtr> = upvalues
            .into_iter()
            .map(|v| self.create_upvalue_closed(v))
            .collect();

        let current_white = self.gc.current_white;
        let value =
            self.object_allocator
                .create_c_closure(&mut self.gc, func, upvalue_ids, current_white);
        value
    }

    /// Create an open upvalue pointing to a stack index
    #[inline(always)]
    pub fn create_upvalue_open(&mut self, stack_index: usize) -> UpvaluePtr {
        let current_white = self.gc.current_white;
        let ptr =
            self.object_allocator
                .create_upvalue_open(&mut self.gc, stack_index, current_white);
        ptr
    }

    /// Create a closed upvalue with a value
    #[inline(always)]
    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> UpvaluePtr {
        let current_white = self.gc.current_white;
        let ptr = self
            .object_allocator
            .create_upvalue_closed(&mut self.gc, value, current_white);
        ptr
    }

    // Port of Lua 5.5's luaC_condGC macro:
    // #define luaC_condGC(L,pre,pos) \
    //   { if (G(L)->GCdebt <= 0) { pre; luaC_step(L); pos;}; }
    //
    /// Check GC and run a step if needed (like luaC_checkGC in Lua 5.5)
    #[inline(always)]
    fn check_gc(&mut self, l: &mut LuaState) -> bool {
        if self.gc.gc_debt <= 0 {
            self.gc.step(l);
            return true;
        }

        false
    }

    // ============ GC Management ============
    /// Perform a full GC cycle (like luaC_fullgc in Lua 5.5)
    /// This is the internal version that can be called in emergency situations
    fn full_gc(&mut self, l: &mut LuaState, is_emergency: bool) {
        self.gc.gc_emergency = is_emergency;

        // Dispatch based on GC mode (from luaC_fullgc)
        match self.gc.gc_kind {
            GcKind::GenMinor => {
                self.full_gen(l);
            }
            GcKind::Inc => {
                self.full_inc(l);
            }
            GcKind::GenMajor => {
                // Temporarily switch to incremental mode
                self.gc.gc_kind = GcKind::Inc;
                self.full_inc(l);
                self.gc.gc_kind = GcKind::GenMajor;
            }
        }

        self.gc.gc_emergency = false;
    }

    /// Full GC cycle for incremental mode (like fullinc in Lua 5.5)
    fn full_inc(&mut self, l: &mut LuaState) {
        // NOTE: mark_open_upvalues should NOT be called here!
        // In Lua 5.5, remarkupvals is called DURING the atomic phase,
        // after restart_collection. Calling it here would add objects
        // to the gray list, which then gets cleared by restart_collection.

        // If we're keeping invariant (in marking phase), sweep first
        if self.gc.keep_invariant() {
            self.gc.enter_sweep(l);
        }

        // Run until pause state
        self.gc.run_until_state(l, crate::gc::GcState::Pause);
        // Run finalizers
        self.gc.run_until_state(l, crate::gc::GcState::CallFin);
        // Complete the cycle
        self.gc.run_until_state(l, crate::gc::GcState::Pause);

        // Set pause for next cycle
        self.gc.set_pause();
    }

    /// Full GC cycle for generational mode (like fullgen in Lua 5.5)
    fn full_gen(&mut self, l: &mut LuaState) {
        // NOTE: mark_open_upvalues should NOT be called here!
        // In Lua 5.5, remarkupvals is called DURING the atomic phase.
        self.gc.full_generation(l);
    }

    // /// Collect all GC roots (objects that must not be collected)
    // pub fn collect_roots(&self) -> Vec<LuaValue> {
    //     let mut roots = Vec::with_capacity(128);

    //     // 1. Global table
    //     roots.push(self.global);

    //     // 2. Registry table (persistent objects storage)
    //     roots.push(self.registry);

    //     // 3. String metatable
    //     if let Some(mt) = &self.string_mt {
    //         roots.push(*mt);
    //     }

    //     // 4. All values in the logical stack (0..stack_top)
    //     // Lua semantics: only slots below L->top are live. Slots above may contain
    //     // stale temporaries and MUST NOT be treated as roots, otherwise weak-table
    //     // tests will keep dead objects alive.
    //     let top = self.main_state.get_top();
    //     for i in 0..top {
    //         if let Some(value) = self.main_state.stack_get(i) {
    //             if !value.is_nil() {
    //                 roots.push(value);
    //             }
    //         }
    //     }

    //     // 5. All call frames (functions being executed)
    //     for i in 0..self.main_state.call_depth() {
    //         if let Some(frame) = self.main_state.get_frame(i) {
    //             roots.push(frame.func.clone());
    //         }
    //     }

    //     // 6. Open upvalues - handled separately in GC by mark_open_upvalues
    //     // Open upvalue objects themselves are marked directly by GC
    //     // Their values (stack slots) are already marked above in step 4

    //     roots
    // }

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

    // ===== Error Handling =====

    pub fn error(&mut self, message: impl Into<String>) -> LuaError {
        self.main_state().error(message.into());
        LuaError::RuntimeError
    }

    #[inline]
    pub fn compile_error(&mut self, message: impl Into<String>) -> LuaError {
        self.main_state().error(message.into());
        LuaError::CompileError
    }

    #[inline]
    pub fn get_error_message(&self) -> &str {
        self.main_state_ref().error_msg()
    }

    /// Generate a stack traceback string
    pub fn generate_traceback(&self, error_msg: &str) -> String {
        // Delegate to LuaState's generate_traceback
        let traceback = self.main_state_ref().generate_traceback();
        if !traceback.is_empty() {
            format!("{}\nstack traceback:\n{}", error_msg, traceback)
        } else {
            error_msg.to_string()
        }
    }

    // ============ Protected Call (pcall/xpcall) ============

    /// Execute a function with protected call (pcall semantics)
    /// Note: Yields are NOT caught by pcall - they propagate through
    pub fn protected_call(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Delegate to main_state
        self.main_state().pcall(func, args)
    }

    /// ULTRA-OPTIMIZED pcall for CFunction calls
    /// Works directly on the stack without any Vec allocations
    /// Returns: (success, result_count) where results are on stack
    #[inline]
    pub fn protected_call_stack_based(
        &mut self,
        func_idx: usize,
        arg_count: usize,
    ) -> LuaResult<(bool, usize)> {
        // Delegate to main_state
        self.main_state().pcall_stack_based(func_idx, arg_count)
    }

    /// Protected call with error handler (xpcall semantics)
    /// The error handler is called if an error occurs
    /// Note: Yields are NOT caught by xpcall - they propagate through
    pub fn protected_call_with_handler(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
        err_handler: LuaValue,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Delegate to main_state
        self.main_state().xpcall(func, args, err_handler)
    }

    pub fn get_main_thread_ptr(&self) -> ThreadPtr {
        self.main_state
    }
}
