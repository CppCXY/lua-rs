use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum LuaLanguageLevel {
    LuaJIT,
    #[default]
    Lua55,
}

impl fmt::Display for LuaLanguageLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LuaLanguageLevel::LuaJIT => write!(f, "LuaJIT"),
            LuaLanguageLevel::Lua55 => write!(f, "Lua 5.5"),
        }
    }
}
