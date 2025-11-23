// C callback support for FFI

use super::ctype::CType;

/// C callback wrapper
pub struct CCallback {
    pub ctype: CType,
    pub lua_function: usize, // Registry index of Lua function
}

impl CCallback {
    pub fn new(ctype: CType, lua_function: usize) -> Self {
        CCallback {
            ctype,
            lua_function,
        }
    }
}
