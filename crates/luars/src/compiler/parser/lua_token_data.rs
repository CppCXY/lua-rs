use crate::compiler::parser::{lua_token_kind::LuaTokenKind, text_range::SourceRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuaTokenData {
    pub kind: LuaTokenKind,
    pub range: SourceRange,
}

impl LuaTokenData {
    pub fn new(kind: LuaTokenKind, range: SourceRange) -> Self {
        LuaTokenData { kind, range }
    }
}
