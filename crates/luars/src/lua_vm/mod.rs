// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
pub mod async_thread;
pub mod call_info;
mod const_string;
mod execute;
mod lua_error;
mod lua_ref;
mod lua_state;
pub mod opcode;
mod safe_option;

use crate::compiler::{LuaLanguageLevel, compile_code, compile_code_with_name};
use crate::gc::GC;
use crate::lua_value::{
    Chunk, LuaUpvalue, LuaUserdata, LuaValue, LuaValueKind, LuaValuePtr, UpvalueStore,
};
pub use crate::lua_vm::call_info::CallInfo;
use crate::lua_vm::const_string::ConstString;
use crate::lua_vm::execute::lua_execute;
pub use crate::lua_vm::lua_error::LuaError;
use crate::lua_vm::lua_ref::RefManager;
pub use crate::lua_vm::lua_ref::{LUA_NOREF, LUA_REFNIL, LuaRefValue, RefId};
pub use crate::lua_vm::lua_state::LuaState;
pub use crate::lua_vm::safe_option::SafeOption;
use crate::stdlib::Stdlib;
use crate::{CreateResult, GcKind, ObjectAllocator, ThreadPtr, UpvaluePtr, lib_registry};
pub use execute::TmKind;
pub use execute::{get_metamethod_event, get_metatable};
pub use opcode::{Instruction, OpCode};
use std::future::Future;
use std::rc::Rc;
use std::time::Instant;

/// xoshiro256** RNG matching C Lua's implementation exactly
#[derive(Debug, Clone)]
pub(crate) struct LuaRng {
    pub state: [u64; 4],
}

impl LuaRng {
    /// Seed from two integers, matching C Lua's setseed
    pub fn from_seed(n1: i64, n2: i64) -> Self {
        let mut rng = LuaRng {
            state: [n1 as u64, 0xff, n2 as u64, 0],
        };
        // Warm up: discard 16 values to spread the seed
        for _ in 0..16 {
            rng.next_rand();
        }
        rng
    }

    /// Seed from a time value (for default initialization)
    pub fn from_seed_time(time: u64) -> Self {
        Self::from_seed(time as i64, 0)
    }

    /// Generate next random u64 using xoshiro256**
    pub fn next_rand(&mut self) -> u64 {
        let s = &mut self.state;
        let s0 = s[0];
        let s1 = s[1];
        let s2 = s[2] ^ s0;
        let s3 = s[3] ^ s1;
        // result = s1 * 5, rotate left 7, then * 9
        let res = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        s[0] = s0 ^ s3;
        s[1] = s1 ^ s2;
        s[2] = s2 ^ (s1 << 17);
        s[3] = s3.rotate_left(45);
        res
    }

    /// Convert random u64 to float in [0, 1)
    /// Takes the top 53 bits (DBL_MANT_DIG) and scales to [0,1)
    pub fn next_float(&mut self) -> f64 {
        let rv = self.next_rand();
        // Take top 53 bits
        let mantissa = rv >> (64 - 53); // = rv >> 11
        (mantissa as f64) * f64::from_bits(0x3CA0000000000000) // 2^-53
    }
}

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

    /// Reference manager for luaL_ref/luaL_unref mechanism
    pub(crate) ref_manager: RefManager,

    /// Object pool for unified object management
    pub(crate) object_allocator: ObjectAllocator,

    /// Garbage collector state
    pub(crate) gc: GC,

    /// Main thread execution state (embedded)
    pub(crate) main_state: ThreadPtr,

    /// String metatable (shared by all strings)
    pub(crate) string_mt: Option<LuaValue>,

    /// Number metatable (shared by all numbers: integers and floats)
    pub(crate) number_mt: Option<LuaValue>,

    /// Boolean metatable (shared by all booleans)
    pub(crate) bool_mt: Option<LuaValue>,

    /// Nil metatable
    pub(crate) nil_mt: Option<LuaValue>,

    pub(crate) safe_option: SafeOption,

    /// Shared C call depth counter — tracks real Rust stack depth across all
    /// coroutines.  Incremented on every entry to `lua_execute` and on every
    /// C-function frame push; decremented on the corresponding exits.
    /// Replaces the old per-LuaState `c_call_depth`.
    pub(crate) n_ccalls: usize,

    pub(crate) version: LuaLanguageLevel,

    /// Random number generator — xoshiro256** matching C Lua exactly
    pub(crate) rng: LuaRng,

    /// Start time for os.clock() measurements
    pub(crate) start_time: Instant,

    pub const_strings: ConstString,

    /// Cached default I/O file handles for fast access (avoids registry lookup per io.write/read)
    pub(crate) io_default_output: Option<LuaValue>,
    pub(crate) io_default_input: Option<LuaValue>,
}

impl LuaVM {
    pub fn new(option: SafeOption) -> Box<Self> {
        let mut gc = GC::new(option.clone());
        gc.set_temporary_memory_limit(isize::MAX / 2);
        let mut object_allocator = ObjectAllocator::new();
        let cs = ConstString::new(&mut object_allocator, &mut gc);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        let mut vm = Box::new(LuaVM {
            global: LuaValue::nil(),
            registry: LuaValue::nil(),
            ref_manager: RefManager::new(),
            object_allocator,
            gc,
            main_state: ThreadPtr::null(), //,
            string_mt: None,
            number_mt: None,
            bool_mt: None,
            nil_mt: None,
            safe_option: option.clone(),
            n_ccalls: 0,
            version: LuaLanguageLevel::Lua55,
            // Initialize RNG with a deterministic seed for reproducibility
            rng: LuaRng::from_seed_time(time),
            // Record start time for os.clock()
            start_time: Instant::now(),
            const_strings: cs,
            io_default_output: None,
            io_default_input: None,
        });

        let ptr_vm = vm.as_mut() as *mut LuaVM;
        // Set LuaVM pointer in main_state
        let thread_value = vm
            .object_allocator
            .create_thread(&mut vm.gc, LuaState::new(6, ptr_vm, true, option.clone()))
            .unwrap();

        vm.main_state = thread_value.as_thread_ptr().unwrap();

        // Initialize registry (like Lua's init_registry)
        // Registry is a GC root and protects all values stored in it
        let registry = vm.create_table(2, 8).unwrap();
        vm.registry = registry;

        // Set _G to point to the global table itself
        let globals_value = vm.create_table(0, 20).unwrap();
        vm.global = globals_value;
        vm.set_global("_G", globals_value).unwrap();
        vm.set_global("_ENV", globals_value).unwrap();

        // Store globals in registry (like Lua's LUA_RIDX_GLOBALS)
        vm.registry_seti(1, globals_value);
        vm.gc.clear_temporary_memory_limit();
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
    pub fn registry_set(&mut self, key: &str, value: LuaValue) -> LuaResult<()> {
        let key_value = self.create_string(key)?;

        // Use VM table_set so we always run the GC barrier
        let registry = self.registry;
        self.raw_set(&registry, key_value, value);
        Ok(())
    }

    /// Get a value from the registry by string key
    pub fn registry_get(&mut self, key: &str) -> LuaResult<Option<LuaValue>> {
        let key = self.create_string(key)?;
        Ok(self.raw_get(&self.registry, &key))
    }

    /// Create a reference to a Lua value (like luaL_ref in C API)
    ///
    /// This stores the value in the registry and returns a LuaRefValue.
    /// - For nil: returns LUA_REFNIL (no storage)
    /// - For GC objects: stores in registry, returns ref ID
    /// - For simple values: stores directly in LuaRefValue
    ///
    /// You must call release_ref() when done to free registry entries.
    pub fn create_ref(&mut self, value: LuaValue) -> LuaRefValue {
        // Nil gets special treatment (no storage)
        if value.is_nil() {
            return LuaRefValue::new_direct(LuaValue::nil());
        }

        // For GC objects (tables, functions, strings, userdata, etc.)
        // store in registry to keep them alive
        if value.is_collectable() {
            let ref_id = self.ref_manager.alloc_ref_id();
            self.registry_seti(ref_id as i64, value);
            LuaRefValue::new_registry(ref_id)
        } else {
            // For simple values (numbers, booleans), store directly
            LuaRefValue::new_direct(value)
        }
    }

    /// Get the value from a reference
    pub fn get_ref_value(&self, lua_ref: &LuaRefValue) -> LuaValue {
        lua_ref.get(self)
    }

    /// Release a reference created by create_ref (like luaL_unref in C API)
    ///
    /// This frees the registry entry and allows the value to be garbage collected.
    /// After calling this, the LuaRefValue should not be used.
    pub fn release_ref(&mut self, lua_ref: LuaRefValue) {
        if let Some(ref_id) = lua_ref.ref_id() {
            // Remove from registry
            self.registry_seti(ref_id as i64, LuaValue::nil());
            // Return ref_id to free list
            self.ref_manager.free_ref_id(ref_id);
        }
        // Direct references don't need cleanup
    }

    /// Release a reference by raw ID (for C API compatibility)
    pub fn release_ref_id(&mut self, ref_id: RefId) {
        if ref_id > 0 {
            self.registry_seti(ref_id as i64, LuaValue::nil());
            self.ref_manager.free_ref_id(ref_id);
        }
    }

    /// Get value from registry by raw ref ID (for C API compatibility)
    pub fn get_ref_value_by_id(&self, ref_id: RefId) -> LuaValue {
        if ref_id == LUA_REFNIL {
            return LuaValue::nil();
        }
        if ref_id <= 0 {
            return LuaValue::nil();
        }
        self.registry_geti(ref_id as i64).unwrap_or(LuaValue::nil())
    }

    pub fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()> {
        lib_registry::create_standard_registry(lib).load_all(self)?;
        Ok(())
    }

    /// Serialize a Lua value to JSON (requires 'serde' feature)
    #[cfg(feature = "serde")]
    pub fn serialize_to_json(&self, value: &LuaValue) -> Result<serde_json::Value, String> {
        crate::serde::lua_to_json(value)
    }

    /// Serialize a Lua value to a JSON string (requires 'serde' feature)
    #[cfg(feature = "serde")]
    pub fn serialize_to_json_string(
        &self,
        value: &LuaValue,
        pretty: bool,
    ) -> Result<String, String> {
        crate::serde::lua_to_json_string(value, pretty)
    }

    /// Deserialize a JSON value to Lua (requires 'serde' feature)
    #[cfg(feature = "serde")]
    pub fn deserialize_from_json(&mut self, json: &serde_json::Value) -> Result<LuaValue, String> {
        crate::serde::json_to_lua(json, self)
    }

    /// Deserialize a JSON string to Lua (requires 'serde' feature)
    #[cfg(feature = "serde")]
    pub fn deserialize_from_json_string(&mut self, json_str: &str) -> Result<LuaValue, String> {
        crate::serde::json_string_to_lua(json_str, self)
    }

    /// Execute a chunk in the main thread
    pub fn execute(&mut self, chunk: Rc<Chunk>) -> LuaResult<Vec<LuaValue>> {
        // Main chunk needs _ENV upvalue pointing to global table
        // This matches Lua 5.4+ behavior where all chunks have _ENV as upvalue[0]
        let env_upval = self.create_upvalue_closed(self.global)?;
        let func = self.create_function(
            chunk,
            crate::lua_value::UpvalueStore::from_vec(vec![env_upval]),
        )?;
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
        main_state.push_frame(&func, base, nargs, -1)?;

        // Run the VM execution loop
        let results = self.run()?;

        // Reset logical stack top for next execution
        self.main_state().set_top(0)?;

        Ok(results)
    }

    /// Main VM execution loop (equivalent to luaV_execute)
    fn run(&mut self) -> LuaResult<Vec<LuaValue>> {
        // Initial entry: track n_ccalls like all other call sites
        self.main_state().inc_n_ccalls()?;
        let exec_result = lua_execute(self.main_state(), 0);
        self.main_state().dec_n_ccalls();
        exec_result?;

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
        self.gc.disable_memory_check();
        let chunk = match compile_code(source, self) {
            Ok(c) => c,
            Err(e) => {
                self.gc.enable_memory_check();
                return Err(self.compile_error(e));
            }
        };

        self.gc.enable_memory_check();
        self.gc.check_memory()?;
        Ok(chunk)
    }

    pub fn compile_with_name(&mut self, source: &str, chunk_name: &str) -> LuaResult<Chunk> {
        self.gc.disable_memory_check();
        let chunk = match compile_code_with_name(source, self, chunk_name) {
            Ok(c) => c,
            Err(e) => {
                self.gc.enable_memory_check();
                return Err(self.compile_error(e));
            }
        };

        self.gc.enable_memory_check();
        self.gc.check_memory()?;
        Ok(chunk)
    }

    pub fn get_global(&mut self, name: &str) -> LuaResult<Option<LuaValue>> {
        let key = self.create_string(name)?;
        Ok(self.raw_get(&self.global, &key))
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) -> LuaResult<()> {
        let key = self.create_string(name)?;

        // Use VM table_set so we always run the GC barrier
        let global = self.global;
        self.raw_set(&global, key, value);

        Ok(())
    }

    /// Set the metatable for all strings
    /// This allows string methods to be called with : syntax (e.g., str:upper())
    pub fn set_string_metatable(&mut self, string_lib_table: LuaValue) -> LuaResult<()> {
        // Create a metatable with __index pointing to the string library
        let mt_value = self.create_table(0, 1)?;

        // Set __index to point to the string library
        let index_key = self.const_strings.tm_index;
        self.raw_set(&mt_value, index_key, string_lib_table);

        // Store in the VM
        self.string_mt = Some(mt_value);

        Ok(())
    }

    // ============ Coroutine Support ============

    /// Create a new thread (coroutine) - returns ThreadId-based LuaValue
    /// OPTIMIZED: Minimal initial allocations - grows on demand
    pub fn create_thread(&mut self, func: LuaValue) -> CreateResult {
        // Create a new LuaState for the coroutine
        let mut thread = LuaState::new(1, self as *mut LuaVM, false, self.safe_option.clone());

        // Push the function onto the thread's stack (updates stack_top)
        // It will be used when resume() is first called
        thread
            .push_value(func)
            .expect("Failed to push function onto coroutine stack");

        // Create thread in ObjectPool and return LuaValue
        self.object_allocator.create_thread(&mut self.gc, thread)
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

    // ============ Async Support ============

    /// Register an async function as a Lua global.
    ///
    /// The async function factory `f` receives the Lua arguments as `Vec<LuaValue>`
    /// and returns a `Future` that produces `LuaResult<Vec<LuaValue>>`.
    ///
    /// From Lua code, the function looks and behaves like a normal synchronous
    /// function. The async yield/resume is driven transparently by `AsyncThread`.
    ///
    /// **Important**: The function MUST be called from within an `AsyncThread`
    /// (i.e., the coroutine must be yieldable). Use `create_async_thread()` or
    /// `execute_string_async()` to run Lua code that calls async functions.
    ///
    /// # Example
    ///
    /// ```ignore
    /// vm.register_async("sleep", |args| async move {
    ///     let secs = args[0].as_number().unwrap_or(1.0);
    ///     tokio::time::sleep(Duration::from_secs_f64(secs)).await;
    ///     Ok(vec![LuaValue::boolean(true)])
    /// })?;
    /// ```
    pub fn register_async<F, Fut>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: Fn(Vec<LuaValue>) -> Fut + 'static,
        Fut: Future<Output = LuaResult<Vec<async_thread::AsyncReturnValue>>> + 'static,
    {
        let wrapper = async_thread::wrap_async_function(f);
        let closure_val = self.create_closure(wrapper)?;
        self.set_global(name, closure_val)?;
        Ok(())
    }

    /// Create an `AsyncThread` from a pre-compiled chunk.
    ///
    /// The chunk is loaded into a new coroutine, and the returned `AsyncThread`
    /// can be `.await`ed to drive execution.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let chunk = vm.compile("return async_fn()")?;
    /// let thread = vm.create_async_thread(chunk, vec![])?;
    /// let results = thread.await?;
    /// ```
    pub fn create_async_thread(
        &mut self,
        chunk: crate::lua_value::Chunk,
        args: Vec<LuaValue>,
    ) -> LuaResult<async_thread::AsyncThread> {
        // Main chunk needs _ENV upvalue pointing to global table
        let env_upval = self.create_upvalue_closed(self.global)?;
        let func_val = self.create_function(
            Rc::new(chunk),
            crate::lua_value::UpvalueStore::from_vec(vec![env_upval]),
        )?;
        let thread_val = self.create_thread(func_val)?;
        let vm_ptr = self as *mut LuaVM;
        Ok(async_thread::AsyncThread::new(thread_val, vm_ptr, args))
    }

    /// Compile and execute Lua source code asynchronously.
    ///
    /// This is the simplest way to run Lua code that may call async functions.
    /// Internally creates a coroutine and drives it with `AsyncThread`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// vm.register_async("fetch", |args| async move { ... })?;
    /// let results = vm.execute_string_async("return fetch('https://...')").await?;
    /// ```
    pub async fn execute_string_async(
        &mut self,
        source: &str,
    ) -> LuaResult<Vec<LuaValue>> {
        let chunk = self.compile(source)?;
        let async_thread = self.create_async_thread(chunk, vec![])?;
        async_thread.await
    }

    #[inline(always)]
    pub fn raw_set(&mut self, table_value: &LuaValue, key: LuaValue, value: LuaValue) -> bool {
        let Some(table) = table_value.as_table_mut() else {
            return false;
        };
        let new_key = table.raw_set(&key, value);

        // GC backward barrier (luaC_barrierback)
        // Only needed when:
        // 1. New key is inserted AND key is collectable, OR
        // 2. Value is collectable
        // Use branchless style for better performance
        let need_barrier = (new_key && key.iscollectable()) || value.iscollectable();
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

    /// Create a string and register it with GC
    /// For short strings (4 bytes), use interning (global deduplication)
    /// Create a string value with automatic interning for short strings
    /// Returns LuaValue directly with ZERO allocation overhead for interned strings
    ///
    /// Performance characteristics:
    /// - Cache hit (interned): O(1) hash lookup, 0 allocations, 0 atomic ops
    /// - Cache miss (new): 1 Box allocation, GC registration, pool insertion
    /// - Long string: 1 Box allocation, GC registration, no pooling
    #[inline]
    pub fn create_string(&mut self, s: &str) -> CreateResult {
        self.object_allocator.create_string(&mut self.gc, s)
    }

    #[inline]
    pub fn create_binary(&mut self, data: Vec<u8>) -> CreateResult {
        self.object_allocator.create_binary(&mut self.gc, data)
    }

    /// Create string from owned String (avoids clone for non-interned strings)
    #[inline]
    pub fn create_string_owned(&mut self, s: String) -> CreateResult {
        self.object_allocator.create_string_owned(&mut self.gc, s)
    }

    /// Create substring (optimized for string.sub)
    #[inline]
    pub fn create_substring(
        &mut self,
        s_value: LuaValue,
        start: usize,
        end: usize,
    ) -> CreateResult {
        self.object_allocator
            .create_substring(&mut self.gc, s_value, start, end)
    }

    /// Create a new table
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> CreateResult {
        self.object_allocator
            .create_table(&mut self.gc, array_size, hash_size)
    }

    /// Create new userdata
    pub fn create_userdata(&mut self, data: LuaUserdata) -> CreateResult {
        self.object_allocator.create_userdata(&mut self.gc, data)
    }

    /// Create a function in object pool
    #[inline(always)]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalues: UpvalueStore) -> CreateResult {
        self.object_allocator
            .create_function(&mut self.gc, chunk, upvalues)
    }

    /// Create a C closure (native function with upvalues stored as closed upvalues)
    /// The upvalues are automatically created as closed upvalues with the given values
    #[inline]
    pub fn create_c_closure(&mut self, func: CFunction, upvalues: Vec<LuaValue>) -> CreateResult {
        self.object_allocator
            .create_c_closure(&mut self.gc, func, upvalues)
    }

    /// Create an RClosure from a Rust closure (Box<dyn Fn>).
    /// Unlike CFunction (bare fn pointer), this can capture arbitrary Rust state.
    #[inline]
    pub fn create_rclosure(
        &mut self,
        func: crate::lua_value::RustCallback,
        upvalues: Vec<LuaValue>,
    ) -> CreateResult {
        self.object_allocator
            .create_rclosure(&mut self.gc, func, upvalues)
    }

    /// Convenience: create an RClosure from any `Fn(&mut LuaState) -> LuaResult<usize> + 'static`.
    /// Boxes the closure automatically.
    #[inline]
    pub fn create_closure<F>(&mut self, func: F) -> CreateResult
    where
        F: Fn(&mut LuaState) -> LuaResult<usize> + 'static,
    {
        self.create_rclosure(Box::new(func), Vec::new())
    }

    /// Convenience: create an RClosure with upvalues from any
    /// `Fn(&mut LuaState) -> LuaResult<usize> + 'static`.
    #[inline]
    pub fn create_closure_with_upvalues<F>(
        &mut self,
        func: F,
        upvalues: Vec<LuaValue>,
    ) -> CreateResult
    where
        F: Fn(&mut LuaState) -> LuaResult<usize> + 'static,
    {
        self.create_rclosure(Box::new(func), upvalues)
    }

    /// Create an open upvalue pointing to a stack index
    #[inline(always)]
    pub fn create_upvalue_open(
        &mut self,
        stack_index: usize,
        ptr: LuaValuePtr,
    ) -> LuaResult<UpvaluePtr> {
        let upval = LuaUpvalue::new_open(stack_index, ptr);
        self.object_allocator.create_upvalue(&mut self.gc, upval)
    }

    /// Create a closed upvalue with a value
    #[inline(always)]
    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> LuaResult<UpvaluePtr> {
        let upval = LuaUpvalue::new_closed(value);
        self.object_allocator.create_upvalue(&mut self.gc, upval)
    }

    // Port of Lua 5.5's luaC_condGC macro:
    // #define luaC_condGC(L,pre,pos) \
    //   { if (G(L)->GCdebt <= 0) { pre; luaC_step(L); pos;}; }
    //
    /// Check GC and run a step if needed (like luaC_checkGC in Lua 5.5)
    ///
    ///  Must check gc_stopped and gc_stopem before running GC!
    /// - gc_stopped: User explicitly stopped GC (collectgarbage("stop"))
    /// - gc_stopem: GC is already running (prevents recursive GC during allocation)
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
    ///
    /// Port of Lua 5.5 lgc.c:
    /// ```c
    /// static void fullgen (lua_State *L, global_State *g) {
    ///   minor2inc(L, g, KGC_INC);
    ///   entergen(L, g);
    /// }
    /// ```
    fn full_gen(&mut self, l: &mut LuaState) {
        self.gc.change_to_incremental_mode(l);
        self.gc.enter_gen(l);
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
    pub fn get_error_message(&mut self, e: LuaError) -> String {
        self.main_state().get_error_msg(e)
    }

    /// Generate a stack traceback string
    pub fn generate_traceback(&mut self, error_msg: &str) -> String {
        // Try to use debug.traceback if available
        // We attempt to call debug.traceback(message, 1)
        let result = (|| -> LuaResult<String> {
            // Get debug table
            let debug_table = match self.get_global("debug")? {
                Some(v) if v.is_table() => v,
                _ => return Ok(String::new()), // debug not available
            };

            // Get debug.traceback function
            let traceback_func = {
                let state = self.main_state();
                let traceback_key = state.create_string("traceback")?;
                match state.raw_get(&debug_table, &traceback_key) {
                    Some(v) if v.is_function() => v,
                    _ => return Ok(String::new()), // debug.traceback not available
                }
            };

            // Create arguments: message and level
            // Use level=0 to include the full traceback from error point
            let state = self.main_state();
            let msg_val = state.create_string(error_msg)?;
            let level_val = LuaValue::integer(0);

            // Call debug.traceback using protected_call
            let (success, results) =
                self.protected_call(traceback_func, vec![msg_val, level_val])?;

            if success {
                if let Some(result) = results.first() {
                    if let Some(s) = result.as_str() {
                        return Ok(s.to_string());
                    }
                }
            }

            Ok(String::new())
        })();

        match result {
            Ok(s) if !s.is_empty() => s,
            _ => self.fallback_traceback(error_msg),
        }
    }

    /// Fallback traceback using Rust implementation
    fn fallback_traceback(&self, error_msg: &str) -> String {
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

    pub fn get_basic_metatable(&self, kind: LuaValueKind) -> Option<LuaValue> {
        match kind {
            LuaValueKind::String | LuaValueKind::Binary => self.string_mt,
            LuaValueKind::Integer | LuaValueKind::Float => self.number_mt,
            LuaValueKind::Boolean => self.bool_mt,
            LuaValueKind::Nil => self.nil_mt,
            _ => None,
        }
    }

    pub fn set_basic_metatable(&mut self, kind: LuaValueKind, mt: Option<LuaValue>) {
        match kind {
            LuaValueKind::String | LuaValueKind::Binary => self.string_mt = mt,
            LuaValueKind::Integer | LuaValueKind::Float => self.number_mt = mt,
            LuaValueKind::Boolean => self.bool_mt = mt,
            LuaValueKind::Nil => self.nil_mt = mt,
            _ => {}
        }
    }

    pub fn get_basic_metatables(&self) -> Vec<LuaValue> {
        let mut mts = Vec::new();
        if let Some(mt) = &self.string_mt {
            mts.push(mt.clone());
        }
        if let Some(mt) = &self.number_mt {
            mts.push(mt.clone());
        }
        if let Some(mt) = &self.bool_mt {
            mts.push(mt.clone());
        }
        if let Some(mt) = &self.nil_mt {
            mts.push(mt.clone());
        }
        mts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lua_ref_mechanism() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Create some test values
        let table = vm.create_table(0, 2).unwrap();
        let num_key = vm.create_string("num").unwrap();
        let str_key = vm.create_string("str").unwrap();
        let str_val = vm.create_string("hello").unwrap();
        vm.raw_set(&table, num_key, LuaValue::number(42.0));
        vm.raw_set(&table, str_key, str_val);

        let number = LuaValue::number(123.456);
        let nil_val = LuaValue::nil();

        // Test 1: Create references
        let table_ref = vm.create_ref(table.clone());
        let number_ref = vm.create_ref(number.clone());
        let nil_ref = vm.create_ref(nil_val.clone());

        // Verify reference types
        assert!(table_ref.is_registry_ref(), "Table should use registry");
        assert!(!number_ref.is_registry_ref(), "Number should be direct");
        assert!(!nil_ref.is_registry_ref(), "Nil should be direct");

        // Test 2: Retrieve values through references
        let retrieved_table = vm.get_ref_value(&table_ref);
        assert!(retrieved_table.is_table(), "Should retrieve table");

        let retrieved_num = vm.get_ref_value(&number_ref);
        assert_eq!(
            retrieved_num.as_number(),
            Some(123.456),
            "Should retrieve number"
        );

        let retrieved_nil = vm.get_ref_value(&nil_ref);
        assert!(retrieved_nil.is_nil(), "Should retrieve nil");

        // Test 3: Verify table contents
        let num_key2 = vm.create_string("num").unwrap();
        let val = vm.raw_get(&retrieved_table, &num_key2);
        assert_eq!(
            val.and_then(|v| v.as_number()),
            Some(42.0),
            "Table content should be preserved"
        );

        // Test 4: Get ref IDs
        let table_ref_id = table_ref.ref_id();
        assert!(table_ref_id.is_some(), "Table ref should have ID");
        assert!(table_ref_id.unwrap() > 0, "Ref ID should be positive");

        let number_ref_id = number_ref.ref_id();
        assert!(number_ref_id.is_none(), "Number ref should not have ID");

        // Test 5: Release references
        vm.release_ref(table_ref);
        vm.release_ref(number_ref);
        vm.release_ref(nil_ref);

        // Test 6: After release, ref should return nil
        let after_release = vm.get_ref_value_by_id(table_ref_id.unwrap());
        assert!(after_release.is_nil(), "Released ref should return nil");

        println!("✓ Lua ref mechanism test passed");
    }

    #[test]
    fn test_ref_id_reuse() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Create and release multiple refs to test ID reuse
        let t1 = vm.create_table(0, 0).unwrap();
        let ref1 = vm.create_ref(t1);
        let id1 = ref1.ref_id().unwrap();

        vm.release_ref(ref1);

        // Create another ref - should reuse the ID
        let t2 = vm.create_table(0, 0).unwrap();
        let ref2 = vm.create_ref(t2);
        let id2 = ref2.ref_id().unwrap();

        assert_eq!(id1, id2, "Ref IDs should be reused");

        vm.release_ref(ref2);

        println!("✓ Ref ID reuse test passed");
    }

    #[test]
    fn test_multiple_refs() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Create multiple refs and verify they don't interfere
        let mut refs = Vec::new();
        for i in 0..10 {
            let table = vm.create_table(0, 1).unwrap();
            let key = vm.create_string("value").unwrap();
            let num_val = LuaValue::number(i as f64);
            vm.raw_set(&table, key, num_val);
            refs.push(vm.create_ref(table));
        }

        // Verify all refs are still valid
        for (i, lua_ref) in refs.iter().enumerate() {
            let table = vm.get_ref_value(lua_ref);
            let key = vm.create_string("value").unwrap();
            let val = vm.raw_get(&table, &key);
            assert_eq!(
                val.and_then(|v| v.as_number()),
                Some(i as f64),
                "Ref {} should have correct value",
                i
            );
        }

        // Release all refs
        for lua_ref in refs {
            vm.release_ref(lua_ref);
        }

        println!("✓ Multiple refs test passed");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_json_serialization() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Test 1: Simple values
        let num = LuaValue::number(42.5);
        let json = vm.serialize_to_json(&num).unwrap();
        assert_eq!(json, serde_json::json!(42.5));

        let bool_val = LuaValue::boolean(true);
        let json = vm.serialize_to_json(&bool_val).unwrap();
        assert_eq!(json, serde_json::json!(true));

        let nil = LuaValue::nil();
        let json = vm.serialize_to_json(&nil).unwrap();
        assert_eq!(json, serde_json::json!(null));

        // Test 2: String
        let str_val = vm.create_string("hello world").unwrap();
        let json = vm.serialize_to_json(&str_val).unwrap();
        assert_eq!(json, serde_json::json!("hello world"));

        // Test 3: Array-like table
        let arr = vm.create_table(3, 0).unwrap();
        vm.raw_set(&arr, LuaValue::number(1.0), LuaValue::number(10.0));
        vm.raw_set(&arr, LuaValue::number(2.0), LuaValue::number(20.0));
        vm.raw_set(&arr, LuaValue::number(3.0), LuaValue::number(30.0));

        let json = vm.serialize_to_json(&arr).unwrap();
        assert_eq!(json, serde_json::json!([10, 20, 30]));

        // Test 4: Object-like table
        let obj = vm.create_table(0, 2).unwrap();
        let key1 = vm.create_string("name").unwrap();
        let key2 = vm.create_string("age").unwrap();
        let val1 = vm.create_string("Alice").unwrap();
        vm.raw_set(&obj, key1, val1);
        vm.raw_set(&obj, key2, LuaValue::number(30.0));

        let json = vm.serialize_to_json(&obj).unwrap();
        let expected = serde_json::json!({"name": "Alice", "age": 30});
        assert_eq!(json, expected);

        // Test 5: Nested structure
        let root = vm.create_table(0, 2).unwrap();
        let inner = vm.create_table(2, 0).unwrap();
        vm.raw_set(&inner, LuaValue::number(1.0), LuaValue::number(1.0));
        vm.raw_set(&inner, LuaValue::number(2.0), LuaValue::number(2.0));

        let key = vm.create_string("data").unwrap();
        vm.raw_set(&root, key, inner);
        let key2 = vm.create_string("count").unwrap();
        vm.raw_set(&root, key2, LuaValue::number(100.0));

        let json = vm.serialize_to_json(&root).unwrap();
        let expected = serde_json::json!({"data": [1, 2], "count": 100});
        assert_eq!(json, expected);

        println!("✓ JSON serialization test passed");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_json_deserialization() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Test 1: Simple values
        let json = serde_json::json!(42);
        let lua_val = vm.deserialize_from_json(&json).unwrap();
        assert_eq!(lua_val.as_number(), Some(42.0));

        let json = serde_json::json!(true);
        let lua_val = vm.deserialize_from_json(&json).unwrap();
        assert_eq!(lua_val.as_bool(), Some(true));

        let json = serde_json::json!(null);
        let lua_val = vm.deserialize_from_json(&json).unwrap();
        assert!(lua_val.is_nil());

        // Test 2: String
        let json = serde_json::json!("hello");
        let lua_val = vm.deserialize_from_json(&json).unwrap();
        assert_eq!(lua_val.as_str(), Some("hello"));

        // Test 3: Array
        let json = serde_json::json!([1, 2, 3]);
        let lua_val = vm.deserialize_from_json(&json).unwrap();
        assert!(lua_val.is_table());

        let key1 = vm.create_string("1").unwrap();
        let val1 = vm.raw_get(&lua_val, &LuaValue::number(1.0)).unwrap();
        assert_eq!(val1.as_number(), Some(1.0));

        // Test 4: Object
        let json = serde_json::json!({"name": "Bob", "age": 25});
        let lua_val = vm.deserialize_from_json(&json).unwrap();
        assert!(lua_val.is_table());

        let key = vm.create_string("name").unwrap();
        let name = vm.raw_get(&lua_val, &key).unwrap();
        assert_eq!(name.as_str(), Some("Bob"));

        println!("✓ JSON deserialization test passed");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_json_roundtrip() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Create a complex Lua structure
        let root = vm.create_table(0, 3).unwrap();

        let key1 = vm.create_string("name").unwrap();
        let val1 = vm.create_string("Test").unwrap();
        vm.raw_set(&root, key1, val1);

        let key2 = vm.create_string("count").unwrap();
        vm.raw_set(&root, key2, LuaValue::number(42.0));

        let key3 = vm.create_string("items").unwrap();
        let items = vm.create_table(3, 0).unwrap();
        vm.raw_set(&items, LuaValue::number(1.0), LuaValue::number(10.0));
        vm.raw_set(&items, LuaValue::number(2.0), LuaValue::number(20.0));
        vm.raw_set(&items, LuaValue::number(3.0), LuaValue::number(30.0));
        vm.raw_set(&root, key3, items);

        // Serialize to JSON
        let json = vm.serialize_to_json(&root).unwrap();

        // Deserialize back to Lua
        let reconstructed = vm.deserialize_from_json(&json).unwrap();

        // Verify structure
        assert!(reconstructed.is_table());

        let key = vm.create_string("name").unwrap();
        let name = vm.raw_get(&reconstructed, &key).unwrap();
        assert_eq!(name.as_str(), Some("Test"));

        let key = vm.create_string("count").unwrap();
        let count = vm.raw_get(&reconstructed, &key).unwrap();
        assert_eq!(count.as_number(), Some(42.0));

        println!("✓ JSON roundtrip test passed");
    }
}
