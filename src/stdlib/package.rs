// Package library (stub implementation)
// Implements: config, cpath, loaded, loadlib, path, preload, searchers, searchpath

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;

pub fn create_package_lib() -> LibraryModule {
    let mut module = LibraryModule::new("package");
    
    // Add functions
    module = module.with_function("loadlib", package_loadlib);
    module = module.with_function("searchpath", package_searchpath);
    
    // Add value fields
    module = module.with_value("loaded", create_loaded_table);
    module = module.with_value("path", create_path_string);
    module = module.with_value("cpath", create_cpath_string);
    module = module.with_value("config", create_config_string);
    
    module
}

// Create the package.loaded table
fn create_loaded_table(vm: &mut LuaVM) -> LuaValue {
    let loaded = vm.create_table();
    
    // Pre-populate with already loaded standard libraries
    let lib_names = vec![
        "_G", "string", "table", "math", "io", "os", 
        "utf8", "coroutine", "debug", "package"
    ];
    
    for lib_name in lib_names {
        if let Some(lib_table) = vm.get_global(lib_name) {
            let key = vm.create_string(lib_name.to_string());
            loaded.borrow_mut().raw_set(
                LuaValue::from_string_rc(key),
                lib_table
            );
        }
    }
    
    LuaValue::from_table_rc(loaded)
}

// Create package.path string
fn create_path_string(vm: &mut LuaVM) -> LuaValue {
    let path = "./?.lua;./?/init.lua";
    LuaValue::from_string_rc(vm.create_string(path.to_string()))
}

// Create package.cpath string
fn create_cpath_string(vm: &mut LuaVM) -> LuaValue {
    let cpath = "./?.so;./?.dll";
    LuaValue::from_string_rc(vm.create_string(cpath.to_string()))
}

// Create package.config string
fn create_config_string(vm: &mut LuaVM) -> LuaValue {
    #[cfg(windows)]
    let config = "\\\n;\n?\n!\n-";
    #[cfg(not(windows))]
    let config = "/\n;\n?\n!\n-";
    
    LuaValue::from_string_rc(vm.create_string(config.to_string()))
}

fn package_loadlib(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    // Stub: return nil and error message
    let err = _vm.create_string("loadlib not implemented".to_string());
    Ok(MultiValue::multiple(vec![
        LuaValue::nil(),
        LuaValue::from_string_rc(err),
    ]))
}

fn package_searchpath(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    // Stub: return nil
    Ok(MultiValue::single(LuaValue::nil()))
}
