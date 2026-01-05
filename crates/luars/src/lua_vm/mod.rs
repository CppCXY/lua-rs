// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
mod call_info;
mod execute;
mod lua_error;
mod lua_state;
pub mod opcode;
mod safe_option;

use crate::compiler::{compile_code, compile_code_with_name};
use crate::gc::{GC, GcFunction, GcId, TableId, UpvalueId};
use crate::lua_value::{Chunk, LuaString, LuaTable, LuaUserdata, LuaValue, LuaValueKind};
pub use crate::lua_vm::call_info::CallInfo;
use crate::lua_vm::execute::lua_execute;
pub use crate::lua_vm::lua_error::LuaError;
pub use crate::lua_vm::lua_state::LuaState;
pub use crate::lua_vm::safe_option::SafeOption;
use crate::stdlib::Stdlib;
use crate::{ObjectPool, lib_registry};
pub use execute::TmKind;
pub use opcode::{Instruction, OpCode};
use std::ptr::null_mut;
use std::rc::Rc;

pub type LuaResult<T> = Result<T, LuaError>;
/// C Function type - Rust function callable from Lua
/// Now takes LuaContext instead of LuaVM for better ergonomics
pub type CFunction = fn(&mut LuaState) -> LuaResult<usize>;

/// Maximum call stack depth (similar to LUAI_MAXCCALLS in Lua)
pub const MAX_CALL_DEPTH: usize = 200;

/// Global VM state (equivalent to global_State in Lua C API)
/// Manages global resources shared by all execution threads/coroutines
pub struct LuaVM {
    /// Global environment table (_G and _ENV point to this)
    pub(crate) global: TableId,

    /// Registry table (like Lua's LUA_REGISTRYINDEX)
    pub(crate) registry: TableId,

    /// Object pool for unified object management
    pub(crate) object_pool: ObjectPool,

    /// Garbage collector state
    pub(crate) gc: GC,

    /// Main thread execution state (embedded)
    pub(crate) main_state: LuaState,

    #[allow(unused)]
    /// String metatable (shared by all strings)
    pub(crate) string_mt: Option<LuaValue>,

    pub(crate) safe_option: SafeOption,
}

impl LuaVM {
    pub fn new(option: SafeOption) -> Box<Self> {
        let mut vm = Box::new(LuaVM {
            global: TableId(0),
            registry: TableId(1),
            object_pool: ObjectPool::new(),
            gc: GC::new(),
            main_state: LuaState::new(6, null_mut(), option.clone()),
            string_mt: None,
            safe_option: option,
        });

        let ptr_vm = vm.as_mut() as *mut LuaVM;
        // Set LuaVM pointer in main_state
        vm.main_state.set_vm(ptr_vm);

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

    pub fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()> {
        lib_registry::create_standard_registry(lib).load_all(self)?;

        // Reset GC state after loading standard libraries
        // Like Lua's initial full GC after loading base libs
        self.gc.gc_debt = -(8 * 1024);

        Ok(())
    }

    /// Execute a chunk in the main thread
    pub fn execute(&mut self, chunk: Rc<Chunk>) -> LuaResult<Vec<LuaValue>> {
        // Main chunk needs _ENV upvalue pointing to global table
        // This matches Lua 5.4+ behavior where all chunks have _ENV as upvalue[0]
        let env_upvalue_id = self.create_upvalue_closed(LuaValue::table(self.global));
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
        let func_idx = self.main_state.get_top();
        let nargs = args.len();

        // Push function onto stack (updates stack_top)
        self.main_state.push_value(func.clone())?;

        // Push arguments (each updates stack_top)
        for arg in args {
            self.main_state.push_value(arg)?;
        }

        // Create initial call frame
        // base points to first argument (func_idx + 1), following Lua convention
        let base = func_idx + 1;
        // Top-level call expects multiple return values
        self.main_state.push_frame(func, base, nargs, -1)?;

        // Run the VM execution loop
        let results = self.run()?;

        // Reset logical stack top for next execution
        self.main_state.set_top(0);

        Ok(results)
    }

    /// Main VM execution loop (equivalent to luaV_execute)
    fn run(&mut self) -> LuaResult<Vec<LuaValue>> {
        lua_execute(&mut self.main_state)?;

        // Collect all values from logical stack (0 to stack_top) as return values
        let mut results = Vec::new();
        let top = self.main_state.get_top();
        for i in 0..top {
            if let Some(val) = self.main_state.stack_get(i) {
                results.push(val);
            }
        }

        // Check GC after VM execution completes (like Lua's luaC_checkGC after returning to caller)
        // At this point, all return values are collected and safe from collection
        self.check_gc();

        Ok(results)
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

    /// Set the metatable for all strings
    /// This allows string methods to be called with : syntax (e.g., str:upper())
    pub fn set_string_metatable(&mut self, string_lib_table: LuaValue) {
        // Create a metatable with __index pointing to the string library
        let mt_id = self.object_pool.create_table(0, 1);
        let mt_value = LuaValue::table(mt_id);

        // Set __index to point to the string library
        let index_key = self.create_string("__index");
        if let Some(mt) = self.object_pool.get_table_mut(mt_id) {
            mt.raw_set(index_key, string_lib_table);
        }

        // Store in the VM
        self.string_mt = Some(mt_value);
    }

    // ============ Coroutine Support ============

    /// Create a new thread (coroutine) - returns ThreadId-based LuaValue
    /// OPTIMIZED: Minimal initial allocations - grows on demand
    pub fn create_thread(&mut self, func: LuaValue) -> LuaValue {
        // Create a new LuaState for the coroutine
        let mut thread = LuaState::new(1, self as *mut LuaVM, self.safe_option.clone());

        // Push the function onto the thread's stack (updates stack_top)
        // It will be used when resume() is first called
        thread
            .push_value(func)
            .expect("Failed to push function onto coroutine stack");

        // Create thread in ObjectPool and return LuaValue
        let thread_id = self.object_pool.create_thread(thread);
        LuaValue::thread(thread_id)
    }

    /// Resume a coroutine - DEPRECATED: Use thread_state.resume() instead
    /// This method is kept for backward compatibility but delegates to LuaState
    pub fn resume_thread(
        &mut self,
        thread_val: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Get ThreadId from LuaValue
        let Some(thread_id) = thread_val.as_thread_id() else {
            return Err(self.error("invalid thread".to_string()));
        };

        if thread_id.is_main() {
            return Err(self.error("cannot resume main thread".to_string()));
        }

        // Get thread's Rc<RefCell<LuaState>>
        let Some(thread_rc) = self.object_pool.get_thread(thread_id) else {
            return Err(self.error("thread not found".to_string()));
        };

        // Borrow mutably and delegate to LuaState::resume
        thread_rc.borrow_mut().resume(args)
    }

    /// Fast table get - NO metatable support!
    /// Use this for normal field access (GETFIELD, GETTABLE, GETI)
    /// This is the correct behavior for Lua bytecode instructions
    /// Only use table_get_with_meta when you explicitly need __index metamethod
    #[inline(always)]
    pub fn table_get(&self, table_value: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        let table_id = table_value.as_table_id()?;
        let table = self.object_pool.get_table(table_id)?;
        table.raw_get(key)
    }

    #[inline(always)]
    pub fn table_set(&mut self, lua_table_val: &LuaValue, key: LuaValue, value: LuaValue) -> bool {
        let Some(table_id) = lua_table_val.as_table_id() else {
            return false;
        };
        let Some(table) = self.object_pool.get_table_mut(table_id) else {
            return false;
        };
        table.raw_set(key, value.clone());
        // Write barrier for GC
        self.gc_barrier_back_table(table_id, &value);
        true
    }

    /// Get value from table with metatable support (__index metamethod)
    /// Use this for GETTABLE, GETFIELD, GETI instructions
    /// For raw access without metamethods, use table_get_raw() instead
    pub fn table_get_with_meta(
        &mut self,
        table_value: &LuaValue,
        key: &LuaValue,
    ) -> Option<LuaValue> {
        // First try raw get
        let table_id = table_value.as_table_id()?;

        // Try to get value directly from table
        if let Some(value) = self.table_get_raw(table_value, key) {
            return Some(value);
        }

        // Value not found, check for __index metamethod
        let metatable = {
            let table = self.object_pool.get_table(table_id)?;
            table.get_metatable()
        };

        if let Some(mt) = metatable {
            // Try to get __index metamethod
            let index_key = self.create_string("__index");
            if let Some(index_mm) = self.table_get_raw(&mt, &index_key) {
                if index_mm.is_table() {
                    // __index is a table, do lookup in it
                    return self.table_get_with_meta(&index_mm, key);
                } else if index_mm.is_function() || index_mm.is_cfunction() {
                    // __index is a function, call it
                    // For now, we'll skip function call to avoid complexity
                    // TODO: Implement function call for __index
                    return None;
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

        // Check if key already exists in table
        let key_exists = if let Some(table) = self.object_pool.get_table(table_id) {
            table.raw_get(&key).is_some()
        } else {
            return Err(self.error("invalid table".to_string()));
        };

        if key_exists {
            // Key exists, just set it directly
            self.table_set_raw(&lua_table_val, key, value);
            return Ok(());
        }

        // Key doesn't exist, check for __newindex metamethod
        let metatable = if let Some(table) = self.object_pool.get_table(table_id) {
            table.get_metatable()
        } else {
            return Err(self.error("invalid table".to_string()));
        };

        if let Some(mt) = metatable {
            let newindex_key = self.create_string("__newindex");
            if let Some(newindex_mm) = self.table_get_raw(&mt, &newindex_key) {
                if newindex_mm.is_table() {
                    // __newindex is a table, set in that table
                    return self.table_set_with_meta(newindex_mm, key, value);
                } else if newindex_mm.is_function() || newindex_mm.is_cfunction() {
                    // __newindex is a function, call it
                    // For now, we'll skip function call to avoid complexity
                    // TODO: Implement function call for __newindex
                    return Ok(());
                }
            }
        }

        // No metamethod or metamethod didn't handle it, do raw set
        self.table_set_raw(&lua_table_val, key, value);
        Ok(())
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
        if self.gc.gc_kind != crate::gc::GcKind::GenMinor {
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
        if self.gc.gc_kind != crate::gc::GcKind::GenMinor {
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
            let uv_gc_id = GcId::UpvalueId(upvalue_id);

            // Get value's GcId for forward barrier
            if let Some(value_gc_id) = match value.kind() {
                LuaValueKind::Table => value.as_table_id().map(GcId::TableId),
                LuaValueKind::Function => value.as_function_id().map(GcId::FunctionId),
                LuaValueKind::Thread => value.as_thread_id().map(GcId::ThreadId),
                LuaValueKind::String => value.as_string_id().map(GcId::StringId),
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
    pub fn create_c_closure_inline1(&mut self, func: CFunction, upvalue: LuaValue) -> LuaValue {
        let id = self.object_pool.create_c_closure_inline1(func, upvalue);
        self.check_gc();
        self.gc.track_object(GcId::FunctionId(id), 128);
        LuaValue::function(id)
    }

    /// Create an open upvalue pointing to a stack index
    #[inline(always)]
    pub fn create_upvalue_open(&mut self, stack_index: usize) -> UpvalueId {
        let id = self.object_pool.create_upvalue_open(stack_index);
        self.check_gc();
        self.gc.track_object(GcId::UpvalueId(id), 64);
        id
    }

    /// Create a closed upvalue with a value
    #[inline(always)]
    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> UpvalueId {
        self.check_gc();
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
        // Collect roots for GC (like luaC_step in Lua 5.5)
        let roots = self.collect_roots();

        // Perform GC step with complete root set
        self.gc.step(&roots, &mut self.object_pool);
    }

    // ============ GC Management ============

    /// Perform garbage collection (like luaC_fullgc in Lua 5.5)
    /// Performs a complete GC cycle, running until pause state is reached
    /// If isemergency is true, avoids operations that might change interpreter state
    pub fn collect_garbage(&mut self) {
        self.full_gc(false);
    }

    /// Perform a full GC cycle (like luaC_fullgc in Lua 5.5)
    /// This is the internal version that can be called in emergency situations
    fn full_gc(&mut self, is_emergency: bool) {
        let old_emergency = self.gc.gc_emergency;
        self.gc.gc_emergency = is_emergency;

        // Dispatch based on GC mode (from luaC_fullgc)
        match self.gc.gc_kind {
            crate::gc::GcKind::GenMinor => {
                self.full_gen();
            }
            crate::gc::GcKind::Inc => {
                self.full_inc();
            }
            crate::gc::GcKind::GenMajor => {
                // Temporarily switch to incremental mode
                self.gc.gc_kind = crate::gc::GcKind::Inc;
                self.full_inc();
                self.gc.gc_kind = crate::gc::GcKind::GenMajor;
            }
        }

        self.gc.gc_emergency = old_emergency;
    }

    /// Full GC cycle for incremental mode (like fullinc in Lua 5.5)
    fn full_inc(&mut self) {
        // Collect roots
        let roots = self.collect_roots();

        // If we're keeping invariant (in marking phase), sweep first
        if self.gc.keep_invariant() {
            self.gc.enter_sweep(&mut self.object_pool);
        }

        // Run until pause state
        self.gc
            .run_until_state(crate::gc::GcState::Pause, &roots, &mut self.object_pool);
        // Run finalizers
        self.gc
            .run_until_state(crate::gc::GcState::CallFin, &roots, &mut self.object_pool);
        // Complete the cycle
        self.gc
            .run_until_state(crate::gc::GcState::Pause, &roots, &mut self.object_pool);

        // Set pause for next cycle
        self.gc.set_pause();
    }

    /// Full GC cycle for generational mode (like fullgen in Lua 5.5)
    fn full_gen(&mut self) {
        let roots = self.collect_roots();
        self.gc.full_generation(&roots, &mut self.object_pool);
    }

    /// Collect all GC roots (objects that must not be collected)
    fn collect_roots(&self) -> Vec<LuaValue> {
        let mut roots = Vec::with_capacity(128);

        // 1. Global table
        roots.push(LuaValue::table(self.global));

        // 2. Registry table (persistent objects storage)
        roots.push(LuaValue::table(self.registry));

        // 3. String metatable
        if let Some(mt) = &self.string_mt {
            roots.push(*mt);
        }

        // 4. All values in the logical stack (0..stack_top)
        let stack_top = self.main_state.get_top();
        for i in 0..stack_top {
            if let Some(value) = self.main_state.stack_get(i) {
                if !value.is_nil() {
                    roots.push(value);
                }
            }
        }

        // 5. All call frames (functions being executed)
        for i in 0..self.main_state.call_depth() {
            if let Some(frame) = self.main_state.get_frame(i) {
                roots.push(frame.func.clone());
            }
        }

        // 6. Open upvalues
        for upval_id in self.main_state.get_open_upvalues() {
            if let Some(uv) = self.object_pool.get_upvalue(*upval_id) {
                if let Some(val) = uv.get_closed_value() {
                    roots.push(val);
                }
            }
        }

        roots
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
        self.main_state.error(message.into());
        LuaError::RuntimeError
    }

    #[inline]
    pub fn compile_error(&mut self, message: impl Into<String>) -> LuaError {
        self.main_state.error(message.into());
        LuaError::CompileError
    }

    #[inline]
    pub fn get_error_message(&self) -> &str {
        self.main_state.error_msg()
    }

    /// Generate a stack traceback string
    pub fn generate_traceback(&self, error_msg: &str) -> String {
        // Delegate to LuaState's generate_traceback
        let traceback = self.main_state.generate_traceback();
        if !traceback.is_empty() {
            format!("{}\nstack traceback:\n{}", error_msg, traceback)
        } else {
            error_msg.to_string()
        }
    }

    // ============ Protected Call (pcall/xpcall) ============
    // DEPRECATED: These methods are kept for backward compatibility
    // New code should use LuaState::pcall/xpcall directly

    /// Execute a function with protected call (pcall semantics)
    /// DEPRECATED: Use lua_state.pcall() instead
    /// Note: Yields are NOT caught by pcall - they propagate through
    pub fn protected_call(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Delegate to main_state
        self.main_state.pcall(func, args)
    }

    /// ULTRA-OPTIMIZED pcall for CFunction calls
    /// DEPRECATED: Use lua_state.pcall_stack_based() instead
    /// Works directly on the stack without any Vec allocations
    /// Returns: (success, result_count) where results are on stack
    #[inline]
    pub fn protected_call_stack_based(
        &mut self,
        func_idx: usize,
        arg_count: usize,
    ) -> LuaResult<(bool, usize)> {
        // Delegate to main_state
        self.main_state.pcall_stack_based(func_idx, arg_count)
    }

    /// Protected call with error handler (xpcall semantics)
    /// DEPRECATED: Use lua_state.xpcall() instead
    /// The error handler is called if an error occurs
    /// Note: Yields are NOT caught by xpcall - they propagate through
    pub fn protected_call_with_handler(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
        err_handler: LuaValue,
    ) -> LuaResult<(bool, Vec<LuaValue>)> {
        // Delegate to main_state
        self.main_state.xpcall(func, args, err_handler)
    }
}
