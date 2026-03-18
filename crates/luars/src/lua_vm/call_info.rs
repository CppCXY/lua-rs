// CallInfo - Information about a single function call
// Equivalent to CallInfo structure in Lua C API (lstate.h)

use crate::gc::UpvaluePtr;
use crate::lua_value::{Chunk, LuaValue};

/// Call status flags (equivalent to Lua's CIST_* flags)
pub mod call_status {
    /// Packed nresults field: stores wanted_results + 1, so MULTRET (-1) becomes 0.
    pub const CIST_NRESULTS_MASK: u32 = 0xFFFF;

    /// Lua function (has bytecode)
    pub const CIST_LUA: u32 = 0;
    /// C function
    pub const CIST_C: u32 = 1 << 21;
    /// Function is a tail call
    pub const CIST_TAIL: u32 = 1 << 22;
    #[allow(unused)]
    /// Call is running a for loop
    pub const CIST_HOOKYIELD: u32 = 1 << 23;
    /// Yieldable protected call (pcall body yielded)
    pub const CIST_YPCALL: u32 = 1 << 24;
    #[allow(unused)]
    /// Call is in error-protected mode (pcall/xpcall)
    pub const CIST_FRESH: u32 = 1 << 25;
    /// Function is closing TBC variables during return
    pub const CIST_CLSRET: u32 = 1 << 26;
    /// Error recovery status saved across yield (precover)
    pub const CIST_RECST: u32 = 1 << 20;

    /// Pending metamethod finish operation (GET/SET) after yield resume.
    /// When set, the `pending_finish_get` field contains a valid value
    /// that must be handled in the startfunc loop before dispatching.
    pub const CIST_PENDING_FINISH: u32 = 1 << 27;

    /// xpcall: this C frame is for an xpcall call.
    /// The error handler is stored at func_pos (base - func_offset),
    /// and the actual body starts at func_pos + 1.
    pub const CIST_XPCALL: u32 = 1 << 28;

    /// Yieldable unprotected call (dofile body yielded)
    /// Like CIST_YPCALL but without error catching — results are moved
    /// without prepending true/false.
    pub const CIST_YCALL: u32 = 1 << 29;

    /// Frame was interrupted by a hook (set during hook callback).
    /// Used by debug.getinfo to report namewhat="hook".
    pub const CIST_HOOKED: u32 = 1 << 30;

    /// Offset for __call metamethod count (bits 16-19)
    pub const CIST_CCMT: u32 = 16;
    /// Mask for __call metamethod count (0xf at bits 16-19)
    pub const MAX_CCMT: u32 = 0xF << CIST_CCMT;

    #[inline(always)]
    pub fn with_nresults(call_status: u32, nresults: i32) -> u32 {
        debug_assert!(nresults >= -1, "nresults must be >= -1");
        debug_assert!(
            nresults < (CIST_NRESULTS_MASK as i32),
            "nresults out of range"
        );
        (call_status & !CIST_NRESULTS_MASK) | ((nresults + 1) as u32)
    }

    #[inline(always)]
    pub fn get_nresults(call_status: u32) -> i32 {
        ((call_status & CIST_NRESULTS_MASK) as i32) - 1
    }

    /// Extract __call count from call_status
    #[inline(always)]
    pub fn get_ccmt_count(call_status: u32) -> u8 {
        ((call_status & MAX_CCMT) >> CIST_CCMT) as u8
    }

    /// Set __call count in call_status
    #[inline(always)]
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
    pub func_offset: u32,

    /// Top of stack for this frame (first free slot)
    /// Equivalent to Lua's CallInfo.top
    pub top: u32,

    /// Program counter (for Lua functions only)
    /// Points to next instruction to execute
    /// Equivalent to Lua's CallInfo.u.l.savedpc
    pub pc: u32,

    /// Call status flags (CIST_*)
    /// Low 16 bits pack expected nresults as (nresults + 1).
    /// Equivalent to Lua's CallInfo.callstatus
    pub call_status: u32,

    /// Number of extra arguments in vararg functions
    /// Equivalent to Lua's CallInfo.u.l.nextraargs
    pub nextraargs: i32,

    /// Cached raw pointer to the Chunk for Lua functions.
    /// Avoids Rc deref in the startfunc header (hot path).
    /// Null for C function frames.
    /// Safety: valid as long as the frame is active (func keeps the Rc alive).
    pub chunk_ptr: *const Chunk,

    /// Cached pointer to the upvalue array for Lua closures.
    /// Avoids the func → GcPtr → GcRClosure → LuaFunction → UpvalueStore enum match
    /// chain on every GetUpval/SetUpval (saves 2-3 loads + 1 branch per access).
    /// Null for C function frames (never accessed).
    /// Safety: valid as long as this frame is active (func keeps the closure alive).
    pub upvalue_ptrs: *const UpvaluePtr,

    /// Reused i32 payload.
    /// - When CIST_CLSRET is set: saved nres for return continuation.
    /// - When CIST_PENDING_FINISH is set: pending finish destination/state.
    pub aux_i32: i32,
}

impl CallInfo {
    /// Create a new call frame for a Lua function
    pub fn new_lua(func: LuaValue, base: usize, nparams: usize) -> Self {
        Self {
            func,
            base,
            chunk_ptr: std::ptr::null(),
            upvalue_ptrs: std::ptr::null(),
            func_offset: 1, // Initially base - 1 = func
            top: (base + nparams) as u32,
            pc: 0,
            call_status: call_status::with_nresults(call_status::CIST_LUA, -1),
            nextraargs: 0,
            aux_i32: -1,
        }
    }

    /// Create a new call frame for a C function
    pub fn new_c(func: LuaValue, base: usize, nparams: usize) -> Self {
        Self {
            func,
            base,
            chunk_ptr: std::ptr::null(),
            upvalue_ptrs: std::ptr::null(),
            func_offset: 1,
            top: (base + nparams) as u32,
            pc: 0,
            call_status: call_status::with_nresults(call_status::CIST_C, -1),
            nextraargs: 0,
            aux_i32: -1,
        }
    }

    /// Check if this is a Lua function call
    #[inline(always)]
    pub fn is_lua(&self) -> bool {
        self.call_status & call_status::CIST_C == 0
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

    pub fn save_pc(&mut self, pc: usize) {
        self.pc = pc as u32;
    }

    #[inline(always)]
    pub fn nresults(&self) -> i32 {
        call_status::get_nresults(self.call_status)
    }

    #[inline(always)]
    pub fn set_nresults(&mut self, nresults: i32) {
        self.call_status = call_status::with_nresults(self.call_status, nresults);
    }

    #[inline(always)]
    pub fn saved_nres(&self) -> i32 {
        self.aux_i32
    }

    #[inline(always)]
    pub fn set_saved_nres(&mut self, nres: i32) {
        self.aux_i32 = nres;
    }

    #[inline(always)]
    pub fn pending_finish_get(&self) -> i32 {
        self.aux_i32
    }

    #[inline(always)]
    pub fn set_pending_finish_get(&mut self, value: i32) {
        self.aux_i32 = value;
    }
}

impl Default for CallInfo {
    fn default() -> Self {
        Self {
            func: LuaValue::nil(),
            base: 0,
            chunk_ptr: std::ptr::null(),
            upvalue_ptrs: std::ptr::null(),
            func_offset: 1,
            top: 0,
            pc: 0,
            call_status: call_status::with_nresults(0, -1),
            nextraargs: 0,
            aux_i32: -1,
        }
    }
}

// Compile-time size check: packed CallInfo should fit in 64 bytes.
const _: () = assert!(std::mem::size_of::<CallInfo>() == 64);
