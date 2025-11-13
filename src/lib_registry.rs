// Library registration system for Lua standard libraries
// Provides a clean way to register Rust functions as Lua libraries

use crate::stdlib;
use crate::value::{LuaValue, MultiValue};
use crate::vm::VM;
use std::collections::HashMap;

/// Type for native functions that can be called from Lua
pub type NativeFunction = fn(&mut VM) -> Result<MultiValue, String>;

/// A library module containing multiple functions
pub struct LibraryModule {
    pub name: &'static str,
    pub functions: Vec<(&'static str, NativeFunction)>,
}

impl LibraryModule {
    /// Create a new library module
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            functions: Vec::new(),
        }
    }

    /// Add a function to this library
    pub fn with_function(mut self, name: &'static str, func: NativeFunction) -> Self {
        self.functions.push((name, func));
        self
    }
}

/// Builder for creating library functions
#[macro_export]
macro_rules! lib_module {
    ($name:expr, {
        $($func_name:expr => $func:expr),* $(,)?
    }) => {{
        let mut module = $crate::lib_registry::LibraryModule::new($name);
        $(
            module.functions.push(($func_name, $func));
        )*
        module
    }};
}

/// Registry for all Lua standard libraries
pub struct LibraryRegistry {
    modules: HashMap<&'static str, LibraryModule>,
}

impl LibraryRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    /// Register a library module
    pub fn register(&mut self, module: LibraryModule) {
        self.modules.insert(module.name, module);
    }

    /// Load all registered libraries into a VM
    pub fn load_all(&self, vm: &mut VM) -> Result<(), String> {
        for module in self.modules.values() {
            self.load_module(vm, module)?;
        }
        Ok(())
    }

    /// Load a specific module into the VM
    pub fn load_module(&self, vm: &mut VM, module: &LibraryModule) -> Result<(), String> {
        // Create a table for the library
        let lib_table = vm.create_table();

        // Register all functions in the table
        for (name, func) in &module.functions {
            let func_value = LuaValue::CFunction(*func);
            let name_key = vm.create_string(name.to_string());
            lib_table
                .borrow_mut()
                .raw_set(LuaValue::String(name_key), func_value);
        }

        // Set the library table as a global
        if module.name == "_G" {
            // For global functions, register them directly
            for (name, func) in &module.functions {
                let func_value = LuaValue::CFunction(*func);
                vm.set_global(name, func_value);
            }
        } else {
            // For module libraries, set the table as global
            // let module_name = vm.create_string(module.name.to_string());
            vm.set_global(module.name, LuaValue::Table(lib_table));
        }

        Ok(())
    }

    /// Get a module by name
    pub fn get_module(&self, name: &str) -> Option<&LibraryModule> {
        self.modules.get(name)
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

    // Register all standard libraries
    registry.register(stdlib::basic::create_basic_lib());
    registry.register(stdlib::string::create_string_lib());
    registry.register(stdlib::table::create_table_lib());
    registry.register(stdlib::math::create_math_lib());
    registry.register(stdlib::io::create_io_lib());
    registry.register(stdlib::os::create_os_lib());
    registry.register(stdlib::utf8::create_utf8_lib());
    registry.register(stdlib::coroutine::create_coroutine_lib());
    registry.register(stdlib::debug::create_debug_lib());
    registry.register(stdlib::package::create_package_lib());

    registry
}

/// Helper to get function arguments from VM registers
pub fn get_args(vm: &VM) -> Vec<LuaValue> {
    let frame = vm.frames.last().unwrap();
    let registers = &frame.registers;

    // Skip register 0 (the function itself)
    registers.iter().skip(1).cloned().collect()
}

/// Helper to get a specific argument
pub fn get_arg(vm: &VM, index: usize) -> Option<LuaValue> {
    let frame = vm.frames.last().unwrap();
    let registers = &frame.registers;

    // Arguments start at register 1 (register 0 is the function)
    if index + 1 < registers.len() {
        Some(registers[index + 1].clone())
    } else {
        None
    }
}

/// Helper to require an argument
pub fn require_arg(vm: &VM, index: usize, func_name: &str) -> Result<LuaValue, String> {
    get_arg(vm, index).ok_or_else(|| format!("{}() requires argument {}", func_name, index + 1))
}

/// Helper to get argument count
pub fn arg_count(vm: &VM) -> usize {
    let frame = vm.frames.last().unwrap();
    // Subtract 1 for the function itself
    frame.registers.len().saturating_sub(1)
}
