use crate::compiler::parser::{lua_token_kind::LuaTokenKind, text_range::SourceRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuaTokenData {
    pub kind: LuaTokenKind,
    pub range: SourceRange,
    pub line: usize, // line number at the END of this token (matches Lua's linenumber)
}

impl LuaTokenData {
    pub fn new(kind: LuaTokenKind, range: SourceRange) -> Self {
        LuaTokenData {
            kind,
            range,
            line: 1,
        }
    }

    pub fn with_line(kind: LuaTokenKind, range: SourceRange, line: usize) -> Self {
        LuaTokenData { kind, range, line }
    }
}
