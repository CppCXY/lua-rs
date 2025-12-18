use crate::compiler::parser::{lexer_config::LexerConfig, lua_language_level::LuaLanguageLevel};

pub struct ParserConfig {
    level: LuaLanguageLevel,
}

impl ParserConfig {
    pub fn new(level: LuaLanguageLevel) -> Self {
        ParserConfig { level }
    }

    pub fn lexer_config(&self) -> LexerConfig {
        LexerConfig {
            language_level: self.level,
        }
    }
}
