use crate::compiler::parser::text_range::SourceRange;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LuaParseErrorKind {
    SyntaxError,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LuaParseError {
    pub kind: LuaParseErrorKind,
    pub message: String,
    pub range: SourceRange,
}

impl LuaParseError {
    pub fn syntax_error_from(message: &str, range: SourceRange) -> Self {
        LuaParseError {
            kind: LuaParseErrorKind::SyntaxError,
            message: message.to_string(),
            range: range.into(),
        }
    }
}
