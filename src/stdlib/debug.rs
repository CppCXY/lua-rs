// Debug library (stub implementation)
// Implements: debug, gethook, getinfo, getlocal, getmetatable, getregistry,
// getupvalue, getuservalue, sethook, setlocal, setmetatable, setupvalue,
// setuservalue, traceback, upvalueid, upvaluejoin

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::vm::VM;

pub fn create_debug_lib() -> LibraryModule {
    crate::lib_module!("debug", {
        "traceback" => debug_traceback,
        "getinfo" => debug_getinfo,
    })
}

fn debug_traceback(vm: &mut VM) -> Result<MultiValue, String> {
    // Simple traceback
    let message = crate::lib_registry::get_arg(vm, 0)
        .and_then(|v| v.as_string())
        .map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "stack traceback:".to_string());

    let result = vm.create_string(message);
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

fn debug_getinfo(_vm: &mut VM) -> Result<MultiValue, String> {
    // Stub: return nil
    Ok(MultiValue::single(LuaValue::nil()))
}
