// Coroutine library (stub implementation)
// Implements: create, isyieldable, resume, running, status, wrap, yield

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;

pub fn create_coroutine_lib() -> LibraryModule {
    crate::lib_module!("coroutine", {
        "create" => coroutine_create,
        "resume" => coroutine_resume,
        "yield" => coroutine_yield,
        "status" => coroutine_status,
    })
}

fn coroutine_create(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    // Stub: return nil
    Ok(MultiValue::single(LuaValue::nil()))
}

fn coroutine_resume(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    // Stub: return false
    Ok(MultiValue::single(LuaValue::boolean(false)))
}

fn coroutine_yield(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    Err("cannot yield from outside a coroutine".to_string())
}

fn coroutine_status(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = _vm.create_string("dead".to_string());
    Ok(MultiValue::single(LuaValue::from_string_rc(s)))
}
