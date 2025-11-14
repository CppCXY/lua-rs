// Package library (stub implementation)
// Implements: config, cpath, loaded, loadlib, path, preload, searchers, searchpath

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;

pub fn create_package_lib() -> LibraryModule {
    crate::lib_module!("package", {
        "loadlib" => package_loadlib,
        "searchpath" => package_searchpath,
    })
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
