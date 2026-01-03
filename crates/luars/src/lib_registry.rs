// Library registration system for Lua standard libraries
// Provides a clean way to register Rust functions as Lua libraries

use crate::lua_value::LuaValue;
use crate::lua_vm::{CFunction, LuaResult, LuaVM};
use crate::lua_vm::LuaState;
use crate::stdlib;
// use crate::stdlib;

/// Type for value initializers - functions that create values when the module loads
pub type ValueInitializer = fn(&mut LuaVM) -> LuaValue;

/// Type for module initializers - functions that set up additional module fields
pub type ModuleInitializer = fn(&mut LuaState) -> LuaResult<()>;

/// Entry in a library module - can be a function or a value
pub enum LibraryEntry {
    Function(CFunction),
    Value(ValueInitializer),
}

/// A library module containing multiple functions and values
pub struct LibraryModule {
    pub name: &'static str,
    pub entries: Vec<(&'static str, LibraryEntry)>,
    pub initializer: Option<ModuleInitializer>,
}

impl LibraryModule {
    /// Create a new library module
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            entries: Vec::new(),
            initializer: None,
        }
    }

    /// Add a function to this library
    pub fn with_function(mut self, name: &'static str, func: CFunction) -> Self {
        self.entries.push((name, LibraryEntry::Function(func)));
        self
    }

    /// Add a value to this library
    pub fn with_value(mut self, name: &'static str, value_init: ValueInitializer) -> Self {
        self.entries.push((name, LibraryEntry::Value(value_init)));
        self
    }

    /// Set the module initializer function
    pub fn with_initializer(mut self, init: ModuleInitializer) -> Self {
        self.initializer = Some(init);
        self
    }
}

/// Builder for creating library modules with functions and values
#[macro_export]
macro_rules! lib_module {
    ($name:expr, {
        $($item_name:expr => $item:expr),* $(,)?
    }) => {{
        let mut module = $crate::lib_registry::LibraryModule::new($name);
        $(
            module.entries.push(($item_name, $crate::lib_registry::LibraryEntry::Function($item)));
        )*
        module
    }};
}

/// Builder for creating library modules with explicit types
#[macro_export]
macro_rules! lib_module_ex {
    ($name:expr, {
        $($item_type:ident : $item_name:expr => $item:expr),* $(,)?
    }) => {{
        let mut module = $crate::lib_registry::LibraryModule::new($name);
        $(
            module.entries.push((
                $item_name,
                lib_module_ex!(@entry $item_type, $item)
            ));
        )*
        module
    }};

    (@entry function, $func:expr) => {
        $crate::lib_registry::LibraryEntry::Function($func)
    };

    (@entry value, $value_init:expr) => {
        $crate::lib_registry::LibraryEntry::Value($value_init)
    };
}

/// Registry for all Lua standard libraries
pub struct LibraryRegistry {
    modules: Vec<LibraryModule>, // Use Vec to preserve insertion order
}

impl LibraryRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    /// Register a library module
    pub fn register(&mut self, module: LibraryModule) {
        self.modules.push(module);
    }

    /// Load all registered libraries into a VM
    pub fn load_all(&self, vm: &mut LuaVM) -> LuaResult<()> {
        for module in &self.modules {
            self.load_module(vm, module)?;
        }
        Ok(())
    }

    /// Load a specific module into the VM
    pub fn load_module(&self, vm: &mut LuaVM, module: &LibraryModule) -> LuaResult<()> {
        // Create a table for the library
        let lib_table = vm.create_table(0, 0);

        // Register all entries in the table
        for (name, entry) in &module.entries {
            let value = match entry {
                LibraryEntry::Function(func) => LuaValue::cfunction(*func),
                LibraryEntry::Value(value_init) => value_init(vm),
            };
            let name_key = vm.create_string(name);
            vm.table_set_with_meta(lib_table, name_key, value)?;
        }

        // Set the library table as a global
        if module.name == "_G" {
            // For global functions, register them directly
            for (name, entry) in &module.entries {
                let value = match entry {
                    LibraryEntry::Function(func) => LuaValue::cfunction(*func),
                    LibraryEntry::Value(value_init) => value_init(vm),
                };
                vm.set_global(name, value);
            }
        } else {
            // For module libraries, set the table as global
            vm.set_global(module.name, lib_table);

            // Special handling for string library: set string metatable
            if module.name == "string" {
                // In Lua, all strings share a metatable where __index points to the string library
                // This allows using string methods with : syntax (e.g., str:upper())
                // vm.set_string_metatable(lib_table.clone());
            }

            // Note: coroutine.wrap is now implemented in Rust (stdlib/coroutine.rs)
            // No need for Lua override anymore

            // Also register in package.loaded (if package exists)
            // This allows require() to find standard libraries
            if let Some(package_table) = vm.get_global("package") {
                if package_table.is_table() {
                    let loaded_key = vm.create_string("loaded");
                    if let Some(loaded_table) = vm.table_get_with_meta(&package_table, &loaded_key)
                        && loaded_table.is_table()
                    {
                        let mod_key = vm.create_string(module.name);
                        vm.table_set_with_meta(loaded_table, mod_key, lib_table.clone())?;
                    }
                }
            }
        }

        // Call the module initializer if it exists
        if let Some(init_fn) = module.initializer {
            init_fn(&mut vm.main_state)?;
        }

        Ok(())
    }

    /// Get a module by name
    pub fn get_module(&self, name: &str) -> Option<&LibraryModule> {
        self.modules.iter().find(|m| m.name == name)
    }
}

impl Default for LibraryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a standard Lua 5.4 library registry with all standard libraries
pub fn create_standard_registry() -> LibraryRegistry {
    let mut registry = LibraryRegistry::new();

    // // Register package library FIRST so package.loaded exists
    // // before other libraries try to register themselves
    registry.register(stdlib::package::create_package_lib());
    // Register all other standard libraries
    registry.register(stdlib::basic::create_basic_lib());
    registry.register(stdlib::string::create_string_lib());
    registry.register(stdlib::table::create_table_lib());
    registry.register(stdlib::math::create_math_lib());
    registry.register(stdlib::io::create_io_lib());
    registry.register(stdlib::os::create_os_lib());
    registry.register(stdlib::utf8::create_utf8_lib());
    registry.register(stdlib::coroutine::create_coroutine_lib());
    registry.register(stdlib::debug::create_debug_lib());
    // #[cfg(feature = "loadlib")]
    // registry.register(stdlib::ffi::create_ffi_lib());
    // #[cfg(feature = "async")]
    // registry.register(stdlib::async_lib::create_async_lib());

    registry
}

/// Helper to get function arguments from VM
#[inline]
pub fn get_args(vm: &LuaVM) -> Vec<LuaValue> {
    vm.main_state.get_args()
}

/// Helper to get a specific argument
/// 1-based index (Lua convention)
#[inline(always)]
pub fn get_arg(vm: &LuaVM, index: usize) -> Option<LuaValue> {
    vm.main_state.get_arg(index)
}

/// Helper to require an argument
/// 1-based index
#[inline]
pub fn require_arg(vm: &mut LuaVM, index: usize, func_name: &str) -> LuaResult<LuaValue> {
    let Some(arg) = vm.main_state.get_arg(index) else {
        return Err(vm.error(format!("{}() requires argument {}", func_name, index)));
    };
    Ok(arg)
}

/// Helper to get argument count
#[inline(always)]
pub fn arg_count(vm: &LuaVM) -> usize {
    vm.main_state.arg_count()
}
