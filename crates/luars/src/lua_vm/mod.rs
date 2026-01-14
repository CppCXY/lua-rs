// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
mod call_info;
mod execute;
mod lua_error;
mod lua_state;
pub mod opcode;
mod safe_option;

use crate::compiler::{compile_code, compile_code_with_name};
use crate::gc::{GC, GcId, TableId, UpvalueId};
use crate::lua_value::{Chunk, LuaTable, LuaUserdata, LuaValue, LuaValueKind};
pub use crate::lua_vm::call_info::CallInfo;
use crate::lua_vm::execute::lua_execute;
pub use crate::lua_vm::lua_error::LuaError;
pub use crate::lua_vm::lua_state::LuaState;
pub use crate::lua_vm::safe_option::SafeOption;
use crate::stdlib::Stdlib;
use crate::{lib_registry, GcKind, ObjectPool};
pub use execute::TmKind;
pub use execute::{get_metamethod_event, get_metatable};
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
    pub(crate) global: LuaValue,

    /// Registry table (like Lua's LUA_REGISTRYINDEX)
    pub(crate) registry: LuaValue,

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
            global: LuaValue::nil(),
            registry: LuaValue::nil(),
            object_pool: ObjectPool::new(option.clone()),
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
        vm.registry = registry;

        // Set _G to point to the global table itself
        let globals_value = vm.create_table(0, 20);
        vm.global = globals_value;
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
        if let Some(reg_table) = self.registry.as_table_mut() {
            reg_table.set_int(key, value);
        }
    }

    /// Get a value from the registry by integer key
    pub fn registry_get_integer(&self, key: i64) -> Option<LuaValue> {
        if let Some(reg_table) = self.registry.as_table_mut() {
            return reg_table.get_int(key);
        }

        None
    }

    /// Set a value in the registry by string key
    pub fn registry_set(&mut self, key: &str, value: LuaValue) {
        let key_value = self.create_string(key);

        if let Some(reg_table) = self.registry.as_table_mut() {
            reg_table.raw_set(&key_value, value);
        }
    }

    /// Get a value from the registry by string key
    pub fn registry_get(&mut self, key: &str) -> Option<LuaValue> {
        let key = self.create_string(key);
        if let Some(reg_table) = self.registry.as_table_mut() {
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
        self.main_state.check_gc()?;

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

        if let Some(global) = self.global.as_table_mut() {
            return global.raw_get(&key);
        }
        None
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        let key = self.create_string(name);

        if let Some(global) = self.global.as_table_mut() {
            global.raw_set(&key, value);
        }
    }

    /// Set the metatable for all strings
    /// This allows string methods to be called with : syntax (e.g., str:upper())
    pub fn set_string_metatable(&mut self, string_lib_table: LuaValue) {
        // Create a metatable with __index pointing to the string library
        let mt_value = self.create_table(0, 1);

        // Set __index to point to the string library
        let index_key = self.create_string("__index");
        if let Some(mt) = mt_value.as_table_mut() {
            mt.raw_set(&index_key, string_lib_table);
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
        let current_white = self.gc.current_white;
        let value = self.object_pool.create_thread(thread, current_white);
        let id = value.as_thread_id().unwrap();
        // Track thread for GC (IMPORTANT: threads are large objects!)
        self.gc
            .track_object(GcId::ThreadId(id), &mut self.object_pool);
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
        let Some(thread_id) = thread_val.as_thread_id() else {
            return Err(self.error("invalid thread".to_string()));
        };

        if thread_id.is_main() {
            return Err(self.error("cannot resume main thread".to_string()));
        }

        // Get thread's Rc<RefCell<LuaState>>
        let Some(thread) = self.object_pool.get_thread_mut(thread_id) else {
            return Err(self.error("thread not found".to_string()));
        };
        //
        // Borrow mutably and delegate to LuaState::resume
        thread.resume(args)
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
        table.raw_set(&key, value.clone());

        // GC backward barrier (luaC_barrierback)
        // Tables use backward barrier since they may be modified many times
        if value.is_collectable() {
            self.gc_barrier_back(table_id);
        }
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
        if let Some(table) = table_value.as_table_mut() {
            if let Some(val) = table.raw_get(key) {
                return Some(val);
            }

            if let Some(meta_id) = table.get_metatable() {
                if let Some(meta_table) = self.object_pool.get_table(meta_id) {
                    // Try to get __index metamethod
                    let index_key = self.object_pool.tm_index;
                    if let Some(index_mm) = meta_table.raw_get(&index_key) {
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
        let mut meta_id = None;
        if let Some(table) = lua_table_val.as_table_mut() {
            if let Some(r) = table.raw_get(&key) {
                if !r.is_nil() {
                    // Key exists, just set it directly
                    table.raw_set(&key, value);
                    return Ok(());
                }
            }

            if let Some(id) = table.get_metatable() {
                meta_id = Some(id);
            }
        }

        // Key doesn't exist, check for __newindex metamethod
        if let Some(mt) = meta_id {
            let newindex_key = self.object_pool.tm_newindex;
            if let Some(table) = self.object_pool.get_table(mt) {
                if let Some(newindex_mm) = table.raw_get(&newindex_key) {
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
        }

        // No metamethod or metamethod didn't handle it, do raw set
        self.table_set_raw(&lua_table_val, key, value);
        Ok(())
    }

    /// Get value from userdata with metatable support
    /// Handles __index metamethod
    pub fn userdata_get(&mut self, userdata_value: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        if let Some(userdata) = userdata_value.as_userdata_mut() {
            let metatable = userdata.get_metatable();
            if let Some(table) = metatable.as_table_mut() {
                // Try to get __index metamethod
                let index_key = self.object_pool.tm_index;
                if let Some(index_mm) = table.raw_get(&index_key) {
                    if index_mm.is_table() {
                        // __index is a table, do lookup in it
                        return self.table_get_with_meta(&index_mm, key);
                    } else if index_mm.is_function() || index_mm.is_cfunction() {
                        // __index is a function, call it
                        // For now, we'll skip function call to avoid complexity
                    }
                }
            }
        }

        None
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
        let (value, is_new) = self.object_pool.create_string(s, current_white);
        if is_new {
            let s_id = value.as_string_id().unwrap();
            self.gc
                .track_object(GcId::StringId(s_id), &mut self.object_pool);
        }
        value
    }

    pub fn create_binary(&mut self, data: Vec<u8>) -> LuaValue {
        let current_white = self.gc.current_white;
        let value = self.object_pool.create_binary(data, current_white);
        let id = value.as_binary_id().unwrap();
        self.gc
            .track_object(GcId::BinaryId(id), &mut self.object_pool);
        value
    }

    /// Create string from owned String (avoids clone for non-interned strings)
    #[inline]
    pub fn create_string_owned(&mut self, s: String) -> LuaValue {
        let current_white = self.gc.current_white;
        let (value, is_new) = self.object_pool.create_string_owned(s, current_white);
        if is_new {
            let s_id = value.as_string_id().unwrap();
            self.gc
                .track_object(GcId::StringId(s_id), &mut self.object_pool);
        }
        value
    }

    /// Create substring (optimized for string.sub)
    #[inline]
    pub fn create_substring(&mut self, s_value: LuaValue, start: usize, end: usize) -> LuaValue {
        let current_white = self.gc.current_white;
        let (value, is_new) = self
            .object_pool
            .create_substring(s_value, start, end, current_white);

        if is_new {
            let s_id = value.as_string_id().unwrap();
            self.gc
                .track_object(GcId::StringId(s_id), &mut self.object_pool);
        }

        value
    }

    /// Get string by LuaValue (resolves ID from object pool)
    pub fn get_string(&self, value: &LuaValue) -> Option<&str> {
        if let Some(id) = value.as_string_id() {
            self.object_pool.get_string(id)
        } else {
            None
        }
    }

    // ============ GC Write Barriers ============

    /// Forward GC barrier (luaC_barrier in Lua 5.5)
    /// Called when modifying an object to point to another object
    /// If owner is black and value is white, restore invariant
    pub fn gc_barrier(&mut self, owner_id: UpvalueId, value_gc_id: GcId) {
        let owner_gc_id = GcId::UpvalueId(owner_id);
        self.gc
            .barrier(owner_gc_id, value_gc_id, &mut self.object_pool);
    }

    /// Backward GC barrier (luaC_barrierback in Lua 5.5)  
    /// Called when modifying a table - marks table as gray again
    /// More efficient than forward barrier for objects with many modifications
    pub fn gc_barrier_back(&mut self, table_id: TableId) {
        let table_gc_id = GcId::TableId(table_id);
        self.gc.barrier_back(table_gc_id, &mut self.object_pool);
    }

    /// Convert LuaValue to GcId (if it's a GC-managed object)
    pub fn value_to_gc_id(&self, value: &LuaValue) -> Option<GcId> {
        match value.kind() {
            LuaValueKind::Table => value.as_table_id().map(GcId::TableId),
            LuaValueKind::Function => value.as_function_id().map(GcId::FunctionId),
            LuaValueKind::String => value.as_string_id().map(GcId::StringId),
            LuaValueKind::Binary => value.as_binary_id().map(GcId::BinaryId),
            LuaValueKind::Thread => value.as_thread_id().map(GcId::ThreadId),
            LuaValueKind::Userdata => value.as_userdata_id().map(GcId::UserdataId),
            _ => None,
        }
    }

    // ============ Legacy GC Barrier Methods (deprecated) ============

    /// Create a new table in object pool
    /// GC tracks objects via ObjectPool iteration, no allgc list needed
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> LuaValue {
        let current_white = self.gc.current_white;
        let value = self
            .object_pool
            .create_table(array_size, hash_size, current_white);
        let id = value.as_table_id().unwrap();
        // Track object for GC - calculates precise size and updates gc_debt
        self.gc
            .track_object(GcId::TableId(id), &mut self.object_pool);
        value
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
            table_ref.raw_set(&key, value);
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
        let current_white = self.gc.current_white;
        let value = self.object_pool.create_userdata(data, current_white);
        let id = value.as_userdata_id().unwrap();
        self.gc
            .track_object(GcId::UserdataId(id), &mut self.object_pool);
        value
    }

    /// Create a function in object pool
    /// Tracks the object in GC's allgc list for efficient sweep
    #[inline(always)]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalue_ids: Vec<UpvalueId>) -> LuaValue {
        let current_white = self.gc.current_white;
        let value = self
            .object_pool
            .create_function(chunk, upvalue_ids, current_white);
        let id = value.as_function_id().unwrap();
        self.gc
            .track_object(GcId::FunctionId(id), &mut self.object_pool);
        value
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

        let current_white = self.gc.current_white;
        let value = self
            .object_pool
            .create_c_closure(func, upvalue_ids, current_white);
        let id = value.as_function_id().unwrap();
        self.gc
            .track_object(GcId::FunctionId(id), &mut self.object_pool);
        value
    }

    /// Create an open upvalue pointing to a stack index
    #[inline(always)]
    pub fn create_upvalue_open(&mut self, stack_index: usize) -> UpvalueId {
        let current_white = self.gc.current_white;
        let id = self
            .object_pool
            .create_upvalue_open(stack_index, current_white);
        self.gc
            .track_object(GcId::UpvalueId(id), &mut self.object_pool);
        id
    }

    /// Create a closed upvalue with a value
    #[inline(always)]
    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> UpvalueId {
        let current_white = self.gc.current_white;
        let id = self.object_pool.create_upvalue_closed(value, current_white);
        self.gc
            .track_object(GcId::UpvalueId(id), &mut self.object_pool);
        id
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
            lua_str.to_string()
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
        self.get_string(value).map(|s| s.to_string())
    }

    /// Get the type name of a LuaValue
    pub fn value_type_name(&self, value: &LuaValue) -> &'static str {
        match value.kind() {
            LuaValueKind::Nil => "nil",
            LuaValueKind::Boolean => "boolean",
            LuaValueKind::Integer | LuaValueKind::Float => "number",
            LuaValueKind::String => "string",
            LuaValueKind::Binary => "string", // Binary is also a string type in Lua
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

        self.check_gc_step();
    }

    /// Public method to force a GC step (for collectgarbage "step")
    pub fn check_gc_step(&mut self) {
        // Collect roots for GC (like luaC_step in Lua 5.5)
        let roots = self.collect_roots();

        // Perform GC step with complete root set
        self.gc.step(&roots, &mut self.object_pool);
    }

    // ============ GC Management ============
    /// Perform a full GC cycle (like luaC_fullgc in Lua 5.5)
    /// This is the internal version that can be called in emergency situations
    fn full_gc(&mut self, is_emergency: bool) {
        let old_emergency = self.gc.gc_emergency;
        self.gc.gc_emergency = is_emergency;

        // Dispatch based on GC mode (from luaC_fullgc)
        match self.gc.gc_kind {
            GcKind::GenMinor => {
                self.full_gen();
            }
            GcKind::Inc => {
                self.full_inc();
            }
            GcKind::GenMajor => {
                // Temporarily switch to incremental mode
                self.gc.gc_kind = GcKind::Inc;
                self.full_inc();
                self.gc.gc_kind = GcKind::GenMajor;
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
    pub fn collect_roots(&self) -> Vec<LuaValue> {
        let mut roots = Vec::with_capacity(128);

        // 1. Global table
        roots.push(self.global);

        // 2. Registry table (persistent objects storage)
        roots.push(self.registry);

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

    /// Execute a function with protected call (pcall semantics)
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
