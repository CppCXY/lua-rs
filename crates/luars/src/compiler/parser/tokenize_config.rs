use crate::compiler::parser::lua_language_level::LuaLanguageLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokensizeConfig {
    pub language_level: LuaLanguageLevel,
}

impl TokensizeConfig {
    pub fn support_complex_number(&self) -> bool {
        matches!(self.language_level, LuaLanguageLevel::LuaJIT)
    }

    pub fn support_ll_integer(&self) -> bool {
        matches!(self.language_level, LuaLanguageLevel::LuaJIT)
    }

    pub fn support_binary_integer(&self) -> bool {
        matches!(self.language_level, LuaLanguageLevel::LuaJIT)
    }

    pub fn support_integer_operation(&self) -> bool {
        true
    }
}

impl Default for TokensizeConfig {
    fn default() -> Self {
        TokensizeConfig {
            language_level: LuaLanguageLevel::Lua55,
        }
    }
}
