use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum LuaLanguageLevel {
    LuaJIT,
    #[default]
    Lua54,
    Lua55,
}

impl fmt::Display for LuaLanguageLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LuaLanguageLevel::Lua54 => write!(f, "Lua 5.4"),
            LuaLanguageLevel::LuaJIT => write!(f, "LuaJIT"),
            LuaLanguageLevel::Lua55 => write!(f, "Lua 5.5"),
        }
    }
}
