// CallInfo - Information about a single function call
// Equivalent to CallInfo structure in Lua C API (lstate.h)

use crate::lua_value::LuaValue;

/// Call status flags (equivalent to Lua's CIST_* flags)
pub mod call_status {
    /// Lua function (has bytecode)
    pub const CIST_LUA: u8 = 1 << 0;
    /// C function
    pub const CIST_C: u8 = 1 << 1;
    /// Function is a tail call
    pub const CIST_TAIL: u8 = 1 << 2;
    /// Call is running a for loop
    pub const CIST_HOOKYIELD: u8 = 1 << 3;
    /// Last hook yielded
    pub const CIST_YPCALL: u8 = 1 << 4;
    /// Call is in error-protected mode (pcall/xpcall)
    pub const CIST_FRESH: u8 = 1 << 5;
}

/// Information about a single function call on the call stack
/// This is similar to CallInfo in lstate.h
#[derive(Clone)]
pub struct CallInfo {
    /// The function being called (contains FunctionId or CFunction)
    pub func: LuaValue,

    /// Base index in the stack for this call frame's registers
    /// Equivalent to Lua's CallInfo.func (but we store index, not pointer)
    pub base: usize,

    /// Top of stack for this frame (first free slot)
    /// Equivalent to Lua's CallInfo.top
    pub top: usize,

    /// Program counter (for Lua functions only)
    /// Points to next instruction to execute
    /// Equivalent to Lua's CallInfo.u.l.savedpc
    pub pc: u32,

    /// Number of expected results from this call
    /// -1 means variable number of results (similar to Lua's LUA_MULTRET)
    /// Equivalent to Lua's CallInfo.nresults
    pub nresults: i32,

    /// Call status flags (CIST_*)
    /// Equivalent to Lua's CallInfo.callstatus
    pub call_status: u8,
}

impl CallInfo {
    /// Create a new call frame for a Lua function
    pub fn new_lua(func: LuaValue, base: usize, nparams: usize) -> Self {
        Self {
            func,
            base,
            top: base + nparams,
            pc: 0,
            nresults: -1,
            call_status: call_status::CIST_LUA,
        }
    }

    /// Create a new call frame for a C function
    pub fn new_c(func: LuaValue, base: usize, nparams: usize) -> Self {
        Self {
            func,
            base,
            top: base + nparams,
            pc: 0,
            nresults: -1,
            call_status: call_status::CIST_C,
        }
    }

    /// Check if this is a Lua function call
    #[inline(always)]
    pub fn is_lua(&self) -> bool {
        self.call_status & call_status::CIST_LUA != 0
    }

    /// Check if this is a C function call
    #[inline(always)]
    pub fn is_c(&self) -> bool {
        self.call_status & call_status::CIST_C != 0
    }

    /// Check if this is a tail call
    #[inline(always)]
    pub fn is_tail(&self) -> bool {
        self.call_status & call_status::CIST_TAIL != 0
    }

    /// Mark as tail call
    #[inline(always)]
    pub fn set_tail(&mut self) {
        self.call_status |= call_status::CIST_TAIL;
    }
}

impl Default for CallInfo {
    fn default() -> Self {
        Self {
            func: LuaValue::nil(),
            base: 0,
            top: 0,
            pc: 0,
            nresults: -1,
            call_status: 0,
        }
    }
}
