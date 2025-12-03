use crate::{FunctionId, LuaValue};

/// LuaCallFrame - Minimal version, modeled after Lua C's CallInfo
///
/// Uses NonBox technique to store function info and call status in func_id_ptr:
///
/// func_id_ptr layout (64-bit):
/// - bit 63: CIST_LUA - Is Lua function (1=Lua function, 0=C function)
/// - bit 62: CIST_FRESH - Fresh call, should stop execution on return
/// - bit 61: CIST_YPCALL - Is pcall (protected call)
/// - bit 60: CIST_TAIL - Tail call
/// - bit 59-48: Reserved (for future extension)
/// - bit 47-0: 48-bit payload
///   - If Lua function: stores FunctionId (u32)
///   - If C function: stores C function pointer (48 bits is enough)
///
/// Key fields:
/// - func_id_ptr: Uses NonBox technique to store function info and call status
/// - code_ptr: Direct pointer to instruction array, hot path optimization
/// - constants_ptr: Direct pointer to constant array, hot path optimization
/// - base_ptr: Register stack base address  
/// - top: Stack top (for argument passing)
/// - pc: Program counter
/// - nresults: Expected return count
/// - result_reg: Register position to write return value
/// - vararg_start: Start position of vararg arguments on stack (absolute index)
/// - vararg_count: Number of vararg arguments
///
/// Memory layout (48 bytes):
/// - 8 bytes: func_id_ptr (NonBox: high 16 bits=flags, low 48 bits=function ID or pointer)
/// - 8 bytes: code_ptr  
/// - 8 bytes: constants_ptr
/// - 4 bytes: base_ptr
/// - 4 bytes: top
/// - 4 bytes: pc
/// - 4 bytes: result_reg
/// - 4 bytes: vararg_start
/// - 2 bytes: nresults
/// - 2 bytes: vararg_count
///
/// Note: upvalues are accessed through FunctionId -> GcFunction::upvalues
/// This is fast because upvalue access uses SlotMap's O(1) lookup
#[derive(Clone)]
pub struct LuaCallFrame {
    /// NonBox field: high bits store call status, low 48 bits store function ID or C function pointer
    pub func_id_ptr: u64, // 8 bytes
    pub code_ptr: *const u32, // 8 bytes - direct pointer to instruction array
    pub constants_ptr: *const LuaValue, // 8 bytes - direct pointer to constant array
    pub base_ptr: u32,        // 4 bytes - register stack base address
    pub top: u32,             // 4 bytes - stack top
    pub pc: u32,              // 4 bytes - program counter
    result_reg: u32,          // 4 bytes - return value write position
    vararg_start: u32,        // 4 bytes - vararg start position (absolute index)
    nresults: i16,            // 2 bytes - expected return count (-1 = LUA_MULTRET)
    vararg_count: u16,        // 2 bytes - number of vararg arguments
}

// ============================================================================
// NonBox bit layout constants
// ============================================================================

/// 48-bit payload mask (low 48 bits)
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

// CallStatus flags - stored in high 16 bits (modeled after Lua's CIST_* flags)
/// bit 63: Is Lua function
const FLAG_LUA: u64 = 1 << 63;
/// bit 62: Fresh call, should stop execution on return
const FLAG_FRESH: u64 = 1 << 62;
/// bit 61: Is pcall (protected call)
const FLAG_YPCALL: u64 = 1 << 61;
/// bit 60: Tail call
const FLAG_TAIL: u64 = 1 << 60;

// Special values
#[allow(dead_code)]
pub const LUA_MULTRET: i16 = -1;

impl Default for LuaCallFrame {
    #[inline(always)]
    fn default() -> Self {
        LuaCallFrame {
            func_id_ptr: 0, // C function, no flags
            code_ptr: std::ptr::null(),
            constants_ptr: std::ptr::null(),
            base_ptr: 0,
            top: 0,
            pc: 0,
            result_reg: 0,
            vararg_start: 0,
            nresults: 0,
            vararg_count: 0,
        }
    }
}

impl LuaCallFrame {
    // ========================================================================
    // NonBox helper methods
    // ========================================================================

    /// Create func_id_ptr with flags from FunctionId (Lua function)
    #[inline(always)]
    fn pack_lua_function(func_id: FunctionId) -> u64 {
        FLAG_LUA | (func_id.0 as u64)
    }

    /// Create func_id_ptr from C function pointer (C function)
    #[inline(always)]
    #[allow(dead_code)]
    fn pack_c_function(c_func_ptr: usize) -> u64 {
        // C function has no FLAG_LUA
        (c_func_ptr as u64) & PAYLOAD_MASK
    }

    /// Extract FunctionId (valid only for Lua functions)
    #[inline(always)]
    fn unpack_function_id(func_id_ptr: u64) -> FunctionId {
        FunctionId((func_id_ptr & PAYLOAD_MASK) as u32)
    }

    /// Extract C function pointer (valid only for C functions)
    #[inline(always)]
    fn unpack_c_function_ptr(func_id_ptr: u64) -> usize {
        (func_id_ptr & PAYLOAD_MASK) as usize
    }

    // ========================================================================
    // Constructors
    // ========================================================================

    /// Create Lua function call frame
    #[inline(always)]
    pub fn new_lua_function(
        func_id: FunctionId,
        code_ptr: *const u32,
        constants_ptr: *const LuaValue,
        base_ptr: usize,
        top: usize,
        result_reg: usize,
        nresults: i16,
    ) -> Self {
        LuaCallFrame {
            func_id_ptr: Self::pack_lua_function(func_id),
            code_ptr,
            constants_ptr,
            base_ptr: base_ptr as u32,
            top: top as u32,
            pc: 0,
            result_reg: result_reg as u32,
            vararg_start: 0,
            nresults,
            vararg_count: 0,
        }
    }

    /// Create C function call frame
    #[inline(always)]
    pub fn new_c_function(base_ptr: usize, top: usize) -> Self {
        LuaCallFrame {
            func_id_ptr: 0, // C function, no FLAG_LUA
            code_ptr: std::ptr::null(),
            constants_ptr: std::ptr::null(),
            base_ptr: base_ptr as u32,
            top: top as u32,
            pc: 0,
            result_reg: 0,
            vararg_start: 0,
            nresults: 0,
            vararg_count: 0,
        }
    }

    pub fn as_function_value(&self) -> LuaValue {
        if self.is_lua() {
            LuaValue::function(Self::unpack_function_id(self.func_id_ptr))
        } else {
            LuaValue::cfunction_ptr(Self::unpack_c_function_ptr(self.func_id_ptr))
        }
    }

    // ========================================================================
    // Vararg methods
    // ========================================================================

    /// Set vararg information for this frame
    #[inline(always)]
    pub fn set_vararg(&mut self, start: usize, count: usize) {
        self.vararg_start = start as u32;
        self.vararg_count = count as u16;
    }

    /// Get vararg start position (absolute stack index)
    #[inline(always)]
    pub fn get_vararg_start(&self) -> usize {
        self.vararg_start as usize
    }

    /// Get vararg count
    #[inline(always)]
    pub fn get_vararg_count(&self) -> usize {
        self.vararg_count as usize
    }

    // ========================================================================
    // Call status flag methods (using NonBox high bits)
    // ========================================================================

    /// Is Lua function
    #[inline(always)]
    pub fn is_lua(&self) -> bool {
        (self.func_id_ptr & FLAG_LUA) != 0
    }

    /// Is fresh call (should stop execution on return)
    #[inline(always)]
    pub fn is_fresh(&self) -> bool {
        (self.func_id_ptr & FLAG_FRESH) != 0
    }

    /// Set as fresh call
    #[inline(always)]
    pub fn set_fresh(&mut self) {
        self.func_id_ptr |= FLAG_FRESH;
    }

    /// Clear fresh call flag
    #[inline(always)]
    pub fn clear_fresh(&mut self) {
        self.func_id_ptr &= !FLAG_FRESH;
    }

    /// Is protected call
    #[inline(always)]
    pub fn is_protected(&self) -> bool {
        (self.func_id_ptr & FLAG_YPCALL) != 0
    }

    /// Set protected call flag
    #[inline(always)]
    pub fn set_protected(&mut self, protected: bool) {
        if protected {
            self.func_id_ptr |= FLAG_YPCALL;
        } else {
            self.func_id_ptr &= !FLAG_YPCALL;
        }
    }

    /// Is tail call
    #[inline(always)]
    pub fn is_tailcall(&self) -> bool {
        (self.func_id_ptr & FLAG_TAIL) != 0
    }

    /// Set tail call flag
    #[inline(always)]
    pub fn set_tailcall(&mut self) {
        self.func_id_ptr |= FLAG_TAIL;
    }

    // ========================================================================
    // Return value related methods
    // ========================================================================

    #[inline(always)]
    pub fn get_nresults(&self) -> i16 {
        self.nresults
    }

    #[inline(always)]
    pub fn set_nresults(&mut self, n: i16) {
        self.nresults = n;
    }

    #[inline(always)]
    pub fn get_result_reg(&self) -> usize {
        self.result_reg as usize
    }

    #[inline(always)]
    pub fn get_num_results(&self) -> usize {
        if self.nresults < 0 {
            usize::MAX
        } else {
            self.nresults as usize
        }
    }

    // ========================================================================
    // Function ID access
    // ========================================================================

    /// Get function ID - for ObjectPool lookup
    /// Returns Some only if Lua function
    #[inline(always)]
    pub fn get_function_id(&self) -> Option<FunctionId> {
        if self.is_lua() {
            Some(Self::unpack_function_id(self.func_id_ptr))
        } else {
            None
        }
    }

    /// Get function ID (without checking if Lua function)
    /// # Safety
    /// Caller must ensure this is a Lua function frame
    #[inline(always)]
    pub unsafe fn get_function_id_unchecked(&self) -> FunctionId {
        Self::unpack_function_id(self.func_id_ptr)
    }
}

impl std::fmt::Debug for LuaCallFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaCallFrame")
            .field("func_id_ptr", &format_args!("{:#018x}", self.func_id_ptr))
            .field("base_ptr", &self.base_ptr)
            .field("top", &self.top)
            .field("pc", &self.pc)
            .field("is_lua", &self.is_lua())
            .field("is_fresh", &self.is_fresh())
            .field("is_tailcall", &self.is_tailcall())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lua_call_frame_size() {
        // Verify frame size is compact
        assert_eq!(std::mem::size_of::<LuaCallFrame>(), 48);
    }

    #[test]
    fn test_nonbox_lua_function() {
        let func_id = FunctionId(12345);
        let frame =
            LuaCallFrame::new_lua_function(func_id, std::ptr::null(), std::ptr::null(), 0, 0, 0, 0);

        assert!(frame.is_lua());
        assert!(!frame.is_fresh());
        assert!(!frame.is_tailcall());
        assert!(!frame.is_protected());
        assert_eq!(frame.get_function_id(), Some(FunctionId(12345)));
    }

    #[test]
    fn test_nonbox_c_function() {
        let frame = LuaCallFrame::new_c_function(100, 200);

        assert!(!frame.is_lua());
        assert_eq!(frame.get_function_id(), None);
        assert_eq!(frame.base_ptr, 100);
        assert_eq!(frame.top, 200);
    }

    #[test]
    fn test_nonbox_flags() {
        let func_id = FunctionId(999);
        let mut frame =
            LuaCallFrame::new_lua_function(func_id, std::ptr::null(), std::ptr::null(), 0, 0, 0, 0);

        // Test fresh flag
        assert!(!frame.is_fresh());
        frame.set_fresh();
        assert!(frame.is_fresh());
        frame.clear_fresh();
        assert!(!frame.is_fresh());

        // Test tailcall flag
        assert!(!frame.is_tailcall());
        frame.set_tailcall();
        assert!(frame.is_tailcall());

        // Test protected flag
        assert!(!frame.is_protected());
        frame.set_protected(true);
        assert!(frame.is_protected());
        frame.set_protected(false);
        assert!(!frame.is_protected());

        // Confirm FunctionId is still correct after modifying flags
        assert_eq!(frame.get_function_id(), Some(FunctionId(999)));
    }

    #[test]
    fn test_nonbox_payload_preservation() {
        // Test large FunctionId value
        let func_id = FunctionId(0xFFFF_FFFF);
        let mut frame =
            LuaCallFrame::new_lua_function(func_id, std::ptr::null(), std::ptr::null(), 0, 0, 0, 0);

        // Set all flags
        frame.set_fresh();
        frame.set_tailcall();
        frame.set_protected(true);

        // Verify FunctionId is still correct
        assert_eq!(frame.get_function_id(), Some(FunctionId(0xFFFF_FFFF)));

        // Verify all flags are set
        assert!(frame.is_lua());
        assert!(frame.is_fresh());
        assert!(frame.is_tailcall());
        assert!(frame.is_protected());
    }
}
