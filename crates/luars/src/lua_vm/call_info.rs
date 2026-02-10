// CallInfo - Information about a single function call
// Equivalent to CallInfo structure in Lua C API (lstate.h)

use crate::lua_value::LuaValue;

/// Call status flags (equivalent to Lua's CIST_* flags)
pub mod call_status {
    /// Lua function (has bytecode)
    pub const CIST_LUA: u32 = 1 << 0;
    /// C function
    pub const CIST_C: u32 = 1 << 1;
    /// Function is a tail call
    pub const CIST_TAIL: u32 = 1 << 2;
    #[allow(unused)]
    /// Call is running a for loop
    pub const CIST_HOOKYIELD: u32 = 1 << 3;
    /// Yieldable protected call (pcall body yielded)
    pub const CIST_YPCALL: u32 = 1 << 4;
    #[allow(unused)]
    /// Call is in error-protected mode (pcall/xpcall)
    pub const CIST_FRESH: u32 = 1 << 5;
    /// Function is closing TBC variables during return
    pub const CIST_CLSRET: u32 = 1 << 6;
    /// Error recovery status saved across yield (precover)
    pub const CIST_RECST: u32 = 1 << 7;

    /// Offset for __call metamethod count (bits 8-11)
    pub const CIST_CCMT: u32 = 8;
    /// Mask for __call metamethod count (0xf at bits 8-11 = 0x0F00)
    pub const MAX_CCMT: u32 = 0xF << CIST_CCMT;

    /// Extract __call count from call_status
    pub fn get_ccmt_count(call_status: u32) -> u8 {
        ((call_status & MAX_CCMT) >> CIST_CCMT) as u8
    }

    /// Set __call count in call_status
    pub fn set_ccmt_count(call_status: u32, count: u8) -> u32 {
        (call_status & !MAX_CCMT) | (((count as u32) << CIST_CCMT) & MAX_CCMT)
    }
}

/// Information about a single function call on the call stack
/// This is similar to CallInfo in lstate.h
#[derive(Clone)]
pub struct CallInfo {
    /// The function being called (contains FunctionId or CFunction)
    pub func: LuaValue,

    /// Base index in the stack for this call frame's registers
    /// Equivalent to Lua's CallInfo.func (but we store index, not pointer)
    /// NOTE: This may be updated by VARARGPREP after stack rearrangement
    pub base: usize,

    /// Offset from original base to func position (for vararg functions after buildhiddenargs)
    /// When nextraargs > 0 and buildhiddenargs was called:
    /// - func_offset = totalargs + 1 (the shift amount)
    /// Otherwise: func_offset = 1 (base - 1 = func)
    pub func_offset: usize,

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
    pub call_status: u32,

    /// Number of extra arguments in vararg functions
    /// Equivalent to Lua's CallInfo.u.l.nextraargs
    pub nextraargs: i32,

    /// Saved number of return values (used when CIST_CLSRET is set)
    /// Equivalent to Lua 5.5's CallInfo.u2.nres
    pub saved_nres: i32,

    /// Pending metamethod GET result destination register (relative to base).
    /// When >= 0, a GET metamethod (e.g. __index) yielded and we need to
    /// finish the operation: copy the TM result to R[pending_finish_get]
    /// and restore the stack top.
    /// -1 = no pending operation (default).
    /// -2 = pending SET operation (just restore top, no result to copy).
    pub pending_finish_get: i32,
}

impl CallInfo {
    /// Create a new call frame for a Lua function
    pub fn new_lua(func: LuaValue, base: usize, nparams: usize) -> Self {
        Self {
            func,
            base,
            func_offset: 1, // Initially base - 1 = func
            top: base + nparams,
            pc: 0,
            nresults: -1,
            call_status: call_status::CIST_LUA,
            nextraargs: 0,
            saved_nres: 0,
            pending_finish_get: -1,
        }
    }

    /// Create a new call frame for a C function
    pub fn new_c(func: LuaValue, base: usize, nparams: usize) -> Self {
        Self {
            func,
            base,
            func_offset: 1,
            top: base + nparams,
            pc: 0,
            nresults: -1,
            call_status: call_status::CIST_C,
            nextraargs: 0,
            saved_nres: 0,
            pending_finish_get: -1,
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
            func_offset: 1,
            base: 0,
            top: 0,
            pc: 0,
            nresults: -1,
            call_status: 0,
            nextraargs: 0,
            saved_nres: 0,
            pending_finish_get: -1,
        }
    }
}
