use crate::compiler::parser::text_range::SourceRange;

#[derive(Debug, Clone, PartialEq)]
pub struct LuaParseError {
    pub message: String,
    pub range: SourceRange,
}

impl LuaParseError {
    pub fn syntax_error_from(message: &str, range: SourceRange) -> Self {
        LuaParseError {
            message: message.to_string(),
            range: range.into(),
        }
    }
}
