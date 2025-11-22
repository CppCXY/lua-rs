// Debug library (stub implementation)
// Implements: debug, gethook, getinfo, getlocal, getmetatable, getregistry,
// getupvalue, getuservalue, sethook, setlocal, setmetatable, setupvalue,
// setuservalue, traceback, upvalueid, upvaluejoin

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

pub fn create_debug_lib() -> LibraryModule {
    crate::lib_module!("debug", {
        "traceback" => debug_traceback,
        "getinfo" => debug_getinfo,
    })
}

fn debug_traceback(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Simple traceback
    let message = crate::lib_registry::get_arg(vm, 0).unwrap_or(LuaValue::nil());
    // .and_then(|v| v.as_lua_string())
    // .map(|s| s.as_str().to_string())
    // .unwrap_or_else(|| "stack traceback:".to_string());

    // let result = vm.create_string(message);
    Ok(MultiValue::single(message))
}

fn debug_getinfo(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Stub: return nil
    Ok(MultiValue::single(LuaValue::nil()))
}
