// TmKind — Lua metamethod event 编号（TMS from ltm.h）。
// 放在公共模块里让新旧执行引擎共享同一个类型。

/// Tag Method types (TMS from ltm.h)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TmKind {
    Index = 0,
    NewIndex = 1,
    Gc = 2,
    Mode = 3,
    Len = 4,
    Eq = 5,
    Add = 6,
    Sub = 7,
    Mul = 8,
    Mod = 9,
    Pow = 10,
    Div = 11,
    IDiv = 12,
    Band = 13,
    Bor = 14,
    Bxor = 15,
    Shl = 16,
    Shr = 17,
    Unm = 18,
    Bnot = 19,
    Lt = 20,
    Le = 21,
    Concat = 22,
    Call = 23,
    Close = 24,
    ToString = 25,
    None = 26, // Not a real metamethod, used to indicate no metamethod found
}

impl TmKind {
    /// Convert u8 to TmKind
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Index,
            1 => Self::NewIndex,
            2 => Self::Gc,
            3 => Self::Mode,
            4 => Self::Len,
            5 => Self::Eq,
            6 => Self::Add,
            7 => Self::Sub,
            8 => Self::Mul,
            9 => Self::Mod,
            10 => Self::Pow,
            11 => Self::Div,
            12 => Self::IDiv,
            13 => Self::Band,
            14 => Self::Bor,
            15 => Self::Bxor,
            16 => Self::Shl,
            17 => Self::Shr,
            18 => Self::Unm,
            19 => Self::Bnot,
            20 => Self::Lt,
            21 => Self::Le,
            22 => Self::Concat,
            23 => Self::Call,
            24 => Self::Close,
            25 => Self::ToString,
            _ => Self::None,
        }
    }

    /// Get the metamethod name
    pub const fn name(self) -> &'static str {
        match self {
            TmKind::Index => "__index",
            TmKind::NewIndex => "__newindex",
            TmKind::Gc => "__gc",
            TmKind::Mode => "__mode",
            TmKind::Len => "__len",
            TmKind::Eq => "__eq",
            TmKind::Add => "__add",
            TmKind::Sub => "__sub",
            TmKind::Mul => "__mul",
            TmKind::Mod => "__mod",
            TmKind::Pow => "__pow",
            TmKind::Div => "__div",
            TmKind::IDiv => "__idiv",
            TmKind::Band => "__band",
            TmKind::Bor => "__bor",
            TmKind::Bxor => "__bxor",
            TmKind::Shl => "__shl",
            TmKind::Shr => "__shr",
            TmKind::Unm => "__unm",
            TmKind::Bnot => "__bnot",
            TmKind::Lt => "__lt",
            TmKind::Le => "__le",
            TmKind::Concat => "__concat",
            TmKind::Call => "__call",
            TmKind::Close => "__close",
            TmKind::ToString => "__tostring",
            TmKind::None => "__none", // Not a real metamethod
        }
    }
}

impl From<TmKind> for u8 {
    fn from(value: TmKind) -> Self {
        value as u8
    }
}
