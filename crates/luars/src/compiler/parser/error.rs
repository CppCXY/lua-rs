use crate::compiler::parser::text_range::SourceRange;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LuaParseErrorKind {
    SyntaxError,
    DocError,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LuaParseError {
    pub kind: LuaParseErrorKind,
    pub message: String,
    pub range: SourceRange,
}

impl LuaParseError {
    pub fn new(kind: LuaParseErrorKind, message: &str, range: SourceRange) -> Self {
        LuaParseError {
            kind,
            message: message.to_string(),
            range,
        }
    }

    pub fn syntax_error_from(message: &str, range: SourceRange) -> Self {
        LuaParseError {
            kind: LuaParseErrorKind::SyntaxError,
            message: message.to_string(),
            range: range.into(),
        }
    }
}
