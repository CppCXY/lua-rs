// Library registration system for Lua standard libraries
// Provides a clean way to register Rust functions as Lua libraries

use crate::lua_api;
use crate::lua_value::LuaValue;
use crate::lua_vm::LuaState;
use crate::lua_vm::{CFunction, GlobalState, LuaResult};
use crate::stdlib::{self, Stdlib};
// use crate::stdlib;

/// Unified installation interface for libraries provided by luars or external crates.
///
/// A library can expose itself either as a plain [`LibraryModule`], a preload-only
/// module such as [`PreloadModule`], or a custom builder type that performs extra
/// setup before registering itself into the high-level [`crate::Lua`] API.
pub trait LuaLibrary {
    /// Install this library into the high-level [`crate::Lua`] API.
    fn install(&self, lua: &mut lua_api::Lua) -> LuaResult<()>;
}

/// Type for value initializers - functions that create values when the module loads
pub type ValueInitializer = fn(&mut GlobalState) -> LuaResult<LuaValue>;

/// Type for module initializers - functions that set up additional module fields
pub type ModuleInitializer = fn(&mut LuaState) -> LuaResult<()>;

/// Entry in a library module - can be a function or a value
pub enum LibraryEntry {
    Function(CFunction),
    Value(ValueInitializer),
}

/// A simple `require()`-loadable preload module.
pub struct PreloadModule {
    pub name: String,
    pub loader: CFunction,
}

impl PreloadModule {
    /// Create a new preload module descriptor.
    pub fn new(name: impl Into<String>, loader: CFunction) -> Self {
        Self {
            name: name.into(),
            loader,
        }
    }
}

impl LuaLibrary for PreloadModule {
    fn install(&self, lua: &mut lua_api::Lua) -> LuaResult<()> {
        let vm = lua.global_state();
        vm.register_preload(&self.name, self.loader)
    }
}

/// A library module containing multiple functions and values
pub struct LibraryModule {
    pub name: String,
    pub entries: Vec<(&'static str, LibraryEntry)>,
    pub initializer: Option<ModuleInitializer>,
}

impl LibraryModule {
    /// Create a new library module
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
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

impl LuaLibrary for LibraryModule {
    fn install(&self, lua: &mut lua_api::Lua) -> LuaResult<()> {
        let vm = lua.global_state();
        load_library_module(vm, self)
    }
}

/// Builder for creating library modules with functions and values
#[doc(hidden)]
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

/// Public builder macro for simple Lua module registration.
///
/// Supports function entries by default, optional `value` entries, and a final
/// `init` hook for extra setup.
#[macro_export]
macro_rules! lua_module {
    ($name:expr, { $($items:tt)* }) => {{
        let mut module = $crate::LibraryModule::new($name);
        $crate::__lua_module_items!(module, $($items)*);
        module
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __lua_module_items {
    ($module:ident,) => {};
    ($module:ident) => {};
    ($module:ident, init => $init:expr $(, $($rest:tt)*)?) => {{
        $module = $module.with_initializer($init);
        $crate::__lua_module_items!($module $(, $($rest)*)?);
    }};
    ($module:ident, value $name:expr => $value:expr $(, $($rest:tt)*)?) => {{
        $module = $module.with_value($name, $value);
        $crate::__lua_module_items!($module $(, $($rest)*)?);
    }};
    ($module:ident, $name:expr => $func:expr $(, $($rest:tt)*)?) => {{
        $module = $module.with_function($name, $func);
        $crate::__lua_module_items!($module $(, $($rest)*)?);
    }};
}

/// Public helper macro for simple preload-module registration.
#[macro_export]
macro_rules! lua_preload_module {
    ($name:expr => $loader:expr) => {{ $crate::PreloadModule::new($name, $loader) }};
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
    pub fn load_all(&self, vm: &mut GlobalState) -> LuaResult<()> {
        for module in &self.modules {
            self.load_module(vm, module)?;
        }
        Ok(())
    }

    /// Load a specific module into the VM
    pub fn load_module(&self, vm: &mut GlobalState, module: &LibraryModule) -> LuaResult<()> {
        load_library_module(vm, module)
    }

    /// Get a module by name
    pub fn get_module(&self, name: &str) -> Option<&LibraryModule> {
        self.modules.iter().find(|m| m.name == name)
    }
}

fn load_library_module(vm: &mut GlobalState, module: &LibraryModule) -> LuaResult<()> {
    // Create a table for the library
    let lib_table = vm.create_table(0, 0)?;

    // Register all entries in the table
    for (name, entry) in &module.entries {
        let value = match entry {
            LibraryEntry::Function(func) => LuaValue::cfunction(*func),
            LibraryEntry::Value(value_init) => value_init(vm)?,
        };
        let name_key = vm.create_string(name)?;
        vm.raw_set(&lib_table, name_key, value);
    }

    // Set the library table as a global
    if module.name == "_G" {
        // For global functions, register them directly
        for (name, entry) in &module.entries {
            let value = match entry {
                LibraryEntry::Function(func) => LuaValue::cfunction(*func),
                LibraryEntry::Value(value_init) => value_init(vm)?,
            };
            vm.set_global(name, value)?;
        }
        // Also register _G (the globals table) in package.loaded["_G"]
        // This matches C Lua's behavior where luaL_requiref stores the base library
        // result in package.loaded["_G"], enabling pushglobalfuncname to find
        // global C functions like pcall, print, etc.
        let globals = vm.global;
        if let Some(package_table) = vm.get_global("package")?
            && package_table.is_table()
        {
            let loaded_key = vm.create_string("loaded")?;
            if let Some(loaded_table) = vm.raw_get(&package_table, &loaded_key)
                && loaded_table.is_table()
            {
                let mod_key = vm.create_string("_G")?;
                vm.raw_set(&loaded_table, mod_key, globals);
            }
        }
    } else {
        // For module libraries, set the table as global
        vm.set_global(&module.name, lib_table)?;

        // Special handling for string library: set string metatable
        if module.name == "string" {
            // In Lua, all strings share a metatable where __index points to the string library
            // This allows using string methods with : syntax (e.g., str:upper())
            vm.set_string_metatable(lib_table)?;
        }

        // Also register in package.loaded and package.preload (if package exists)
        // This allows require() to find standard libraries
        if let Some(package_table) = vm.get_global("package")?
            && package_table.is_table()
        {
            let loaded_key = vm.create_string("loaded")?;
            if let Some(loaded_table) = vm.raw_get(&package_table, &loaded_key)
                && loaded_table.is_table()
            {
                let mod_key = vm.create_string(&module.name)?;
                vm.raw_set(&loaded_table, mod_key, lib_table);
            }
        }
    }

    // Call the module initializer if it exists
    if let Some(init_fn) = module.initializer {
        init_fn(vm.main_state())?;
    }

    Ok(())
}

impl Default for LibraryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a standard Lua 5.5 library registry with all standard libraries
pub fn create_standard_registry(open_lib: Stdlib) -> LibraryRegistry {
    let mut registry = LibraryRegistry::new();

    if matches!(open_lib, Stdlib::All | Stdlib::Package) {
        registry.register(stdlib::package::create_package_lib());
    }
    // Register all other standard libraries
    if matches!(open_lib, Stdlib::All | Stdlib::Basic) {
        registry.register(stdlib::basic::create_basic_lib());
    }
    if matches!(open_lib, Stdlib::All | Stdlib::String) {
        registry.register(stdlib::string::create_string_lib());
    }
    if matches!(open_lib, Stdlib::All | Stdlib::Table) {
        registry.register(stdlib::table::create_table_lib());
    }
    if matches!(open_lib, Stdlib::All | Stdlib::Math) {
        registry.register(stdlib::math::create_math_lib());
    }
    if matches!(open_lib, Stdlib::All | Stdlib::Io) {
        registry.register(stdlib::io::create_io_lib());
    }
    if matches!(open_lib, Stdlib::All | Stdlib::Os) {
        registry.register(stdlib::os::create_os_lib());
    }
    if matches!(open_lib, Stdlib::All | Stdlib::Utf8) {
        registry.register(stdlib::utf8::create_utf8_lib());
    }
    if matches!(open_lib, Stdlib::All | Stdlib::Coroutine) {
        registry.register(stdlib::coroutine::create_coroutine_lib());
    }
    if matches!(open_lib, Stdlib::All | Stdlib::Debug) {
        registry.register(stdlib::debug::create_debug_lib());
    }
    // #[cfg(feature = "loadlib")]
    // registry.register(stdlib::ffi::create_ffi_lib());
    // #[cfg(feature = "async")]
    // registry.register(stdlib::async_lib::create_async_lib());

    registry
}
