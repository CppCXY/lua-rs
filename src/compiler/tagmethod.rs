//! Tag Methods (Metamethods) for Lua 5.4
//!
//! These correspond to the metamethod events in Lua.
//! Reference: Lua 5.4 ltm.h

/// Tag method events for metamethods
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TagMethod {
    Index = 0,    // __index
    NewIndex = 1, // __newindex
    Gc = 2,       // __gc
    Mode = 3,     // __mode
    Len = 4,      // __len
    Eq = 5,       // __eq
    Add = 6,      // __add
    Sub = 7,      // __sub
    Mul = 8,      // __mul
    Mod = 9,      // __mod
    Pow = 10,     // __pow
    Div = 11,     // __div
    IDiv = 12,    // __idiv
    BAnd = 13,    // __band
    BOr = 14,     // __bor
    BXor = 15,    // __bxor
    Shl = 16,     // __shl
    Shr = 17,     // __shr
    Unm = 18,     // __unm (unary minus)
    BNot = 19,    // __bnot (bitwise not)
    Lt = 20,      // __lt
    Le = 21,      // __le
    Concat = 22,  // __concat
    Call = 23,    // __call
    Close = 24,   // __close
}

impl TagMethod {
    /// Convert TagMethod to u32 for instruction encoding
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    /// Get the metamethod name
    #[allow(dead_code)]
    pub const fn name(self) -> &'static str {
        match self {
            TagMethod::Index => "__index",
            TagMethod::NewIndex => "__newindex",
            TagMethod::Gc => "__gc",
            TagMethod::Mode => "__mode",
            TagMethod::Len => "__len",
            TagMethod::Eq => "__eq",
            TagMethod::Add => "__add",
            TagMethod::Sub => "__sub",
            TagMethod::Mul => "__mul",
            TagMethod::Mod => "__mod",
            TagMethod::Pow => "__pow",
            TagMethod::Div => "__div",
            TagMethod::IDiv => "__idiv",
            TagMethod::BAnd => "__band",
            TagMethod::BOr => "__bor",
            TagMethod::BXor => "__bxor",
            TagMethod::Shl => "__shl",
            TagMethod::Shr => "__shr",
            TagMethod::Unm => "__unm",
            TagMethod::BNot => "__bnot",
            TagMethod::Lt => "__lt",
            TagMethod::Le => "__le",
            TagMethod::Concat => "__concat",
            TagMethod::Call => "__call",
            TagMethod::Close => "__close",
        }
    }
}
