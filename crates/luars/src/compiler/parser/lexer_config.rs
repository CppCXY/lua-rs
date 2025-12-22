use crate::compiler::parser::{
    lua_language_level::LuaLanguageLevel, tokenize_config::TokensizeConfig,
};

pub struct ParserConfig {
    level: LuaLanguageLevel,
}

impl ParserConfig {
    pub fn new(level: LuaLanguageLevel) -> Self {
        ParserConfig { level }
    }

    pub fn lexer_config(&self) -> TokensizeConfig {
        TokensizeConfig {
            language_level: self.level,
        }
    }
}
