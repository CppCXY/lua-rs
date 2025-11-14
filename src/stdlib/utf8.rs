// UTF-8 library (stub implementation)
// Implements: char, charpattern, codes, codepoint, len, offset

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;

pub fn create_utf8_lib() -> LibraryModule {
    crate::lib_module!("utf8", {
        "len" => utf8_len,
        "char" => utf8_char,
    })
}

fn utf8_len(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = crate::lib_registry::require_arg(vm, 0, "utf8.len")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'utf8.len' (string expected)".to_string())?;

    let len = s.as_str().chars().count();
    Ok(MultiValue::single(LuaValue::integer(len as i64)))
}

fn utf8_char(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);

    let mut result = String::new();
    for arg in args {
        if let Some(code) = arg.as_integer() {
            if let Some(ch) = char::from_u32(code as u32) {
                result.push(ch);
            }
        }
    }

    let s = vm.create_string(result);
    Ok(MultiValue::single(LuaValue::from_string_rc(s)))
}
