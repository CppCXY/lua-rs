// StkId — Stack slot identifier, equivalent to C Lua's StkId (TValue*)
//
// Encapsulates a raw *mut LuaValue pointer to a stack slot. All methods are
// #[inline(always)]: internally unsafe, externally safe. Eliminates scattered
// `unsafe` blocks and repeated `lua_state.stack_mut().as_mut_ptr()` calls
// in opcode handlers.
//
// # Safety Invariant
// The pointer must point to a valid LuaValue within the Lua stack.

use crate::{
    LuaRawTable,
    gc::{GcObjectPtr, TablePtr},
    lua_value::{
        LUA_VFALSE, LUA_VNIL, LUA_VNUMFLT, LUA_VNUMINT, LUA_VTRUE, LuaInnerValue, LuaValue,
    },
};

/// Stack slot handle — a raw pointer wrapped for safe(er) access.
#[derive(Clone, Copy)]
pub struct StkId(*mut LuaValue);

impl StkId {
    // ===== Construction / Addressing =====

    /// Construct from a raw pointer (used once per frame at outer loop init).
    #[inline(always)]
    pub fn from_mut_ptr(ptr: *mut LuaValue) -> Self {
        Self(ptr)
    }

    pub fn from_const_ptr(ptr: *const LuaValue) -> Self {
        Self(ptr as *mut LuaValue)
    }

    /// Null StkId (used for uninitialized CallInfo).
    #[inline(always)]
    pub fn null() -> Self {
        Self(std::ptr::null_mut())
    }

    /// Compute StkId from stack base pointer + absolute index.
    #[inline(always)]
    pub fn from_stack(sp: *mut LuaValue, idx: usize) -> Self {
        Self(unsafe { sp.add(idx) })
    }

    /// Register-relative offset: `base.offset(reg)` = &stack[base + reg].
    #[inline(always)]
    pub fn offset(self, reg: usize) -> Self {
        unsafe { Self(self.0.add(reg)) }
    }

    // ===== Type Checks =====

    #[inline(always)]
    pub fn is_integer(self) -> bool {
        unsafe { (*self.0).tt == LUA_VNUMINT }
    }

    #[inline(always)]
    pub fn is_float(self) -> bool {
        unsafe { (*self.0).tt == LUA_VNUMFLT }
    }

    #[inline(always)]
    pub fn is_table(self) -> bool {
        unsafe { (*self.0).is_table() }
    }

    #[inline(always)]
    pub fn is_nil(self) -> bool {
        unsafe { (*self.0).tt == LUA_VNIL }
    }

    #[inline(always)]
    pub fn is_false_or_nil(self) -> bool {
        unsafe {
            let tt = (*self.0).tt;
            tt == LUA_VFALSE || tt == LUA_VNIL
        }
    }

    #[inline(always)]
    pub fn is_short_string(self) -> bool {
        unsafe { (*self.0).is_short_string() }
    }

    // ===== Value Reads =====

    #[inline(always)]
    pub fn ivalue(self) -> i64 {
        unsafe { (*self.0).value.i }
    }

    #[inline(always)]
    pub fn fltvalue(self) -> f64 {
        unsafe { (*self.0).value.n }
    }

    #[inline(always)]
    pub fn hvalue(self) -> &'static LuaRawTable {
        unsafe { (*self.0).hvalue() }
    }

    #[inline(always)]
    pub fn hvalue_mut(self) -> &'static mut LuaRawTable {
        unsafe { (*self.0).hvalue_mut() }
    }

    #[inline(always)]
    pub fn as_gc_ptr(self) -> GcObjectPtr {
        unsafe { (*self.0).as_gc_ptr_unchecked() }
    }

    #[inline(always)]
    pub fn as_table_ptr(self) -> TablePtr {
        unsafe { (*self.0).table_ptr_raw() }
    }

    #[inline(always)]
    pub fn is_collectable(self) -> bool {
        unsafe { (*self.0).is_collectable() }
    }

    // ===== Value Writes =====

    /// Copy 16 bytes from another stack slot (equivalent to C Lua's setobjs2s).
    #[inline(always)]
    pub fn set(self, src: StkId) {
        unsafe {
            *self.0 = *src.0;
        }
    }

    /// Copy a LuaValue from an arbitrary reference (constants table etc.).
    #[inline(always)]
    pub fn write(self, v: &LuaValue) {
        unsafe {
            *self.0 = *v;
        }
    }

    #[inline(always)]
    pub fn write_parts(self, tt: u8, value: LuaInnerValue) {
        unsafe {
            (*self.0).tt = tt;
            (*self.0).value = value;
        }
    }

    #[inline(always)]
    pub fn set_integer(self, i: i64) {
        unsafe {
            (*self.0).tt = LUA_VNUMINT;
            (*self.0).value.i = i;
        }
    }

    #[inline(always)]
    pub fn set_float(self, n: f64) {
        unsafe {
            (*self.0).tt = LUA_VNUMFLT;
            (*self.0).value.n = n;
        }
    }

    #[inline(always)]
    pub fn set_bool(self, b: bool) {
        unsafe {
            (*self.0).tt = if b { LUA_VTRUE } else { LUA_VFALSE };
            (*self.0).value = LuaInnerValue::NIL;
        }
    }

    #[inline(always)]
    pub fn set_nil(self) {
        unsafe {
            (*self.0).tt = LUA_VNIL;
            (*self.0).value = LuaInnerValue::NIL;
        }
    }

    /// Write only the integer value field without touching the type tag.
    /// Used by FORLOOP for in-place counter/idx updates.
    #[inline(always)]
    pub fn set_raw_i(self, i: i64) {
        unsafe {
            (*self.0).value.i = i;
        }
    }

    // ===== Raw Pointer Access (for helper functions expecting pointers) =====

    #[inline(always)]
    pub fn as_ptr(self) -> *mut LuaValue {
        self.0
    }

    #[inline(always)]
    pub fn as_const_ptr(self) -> *const LuaValue {
        self.0 as *const LuaValue
    }

    #[inline(always)]
    pub fn is_valid(self) -> bool {
        !self.0.is_null()
    }

    #[inline(always)]
    pub fn get(self) -> LuaValue {
        unsafe { *self.0 }
    }

    pub fn get_ref(self) -> &'static LuaValue {
        unsafe { &*self.0 }
    }
}
