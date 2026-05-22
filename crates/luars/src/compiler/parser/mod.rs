mod lua_language_level;
mod lua_operator_kind;
mod lua_token_data;
mod lua_token_kind;
mod lua_tokenize;
mod reader;
mod text_range;
mod tokenize_config;

pub use crate::compiler::parser::{
    lua_language_level::LuaLanguageLevel, lua_operator_kind::*, lua_token_data::LuaTokenData,
    lua_token_kind::LuaTokenKind, lua_tokenize::LuaTokenize, reader::Reader,
    text_range::SourceRange, tokenize_config::TokensizeConfig,
};

pub struct LuaLexer<'a> {
    text: &'a str,
    tokenizer: LuaTokenize<'a>,
    current: LuaTokenData,
    next: LuaTokenData,
    previous: Option<SourceRange>,
    token_index: usize,
    current_token: LuaTokenKind,
    error: Option<String>,
    #[allow(unused)]
    pub level: LuaLanguageLevel,
    pub line: usize,          // current line number (linenumber in Lua)
    pub lastline: usize,      // line of last token consumed (lastline in Lua)
    pub nesting_level: usize, // parser nesting depth (like C Lua's nCcalls during compilation)
}

#[allow(unused)]
impl<'a> LuaLexer<'a> {
    pub fn new(text: &'a str, level: LuaLanguageLevel) -> Result<LuaLexer<'a>, String> {
        let mut tokenizer = LuaTokenize::new(
            Reader::new(text),
            TokensizeConfig {
                language_level: level,
            },
        );

        let current = tokenizer.next_token_data()?;
        let next = tokenizer.next_token_data()?;

        Ok(LuaLexer {
            text,
            tokenizer,
            current_token: current.kind,
            current,
            next,
            previous: None,
            token_index: 0,
            error: None,
            level,
            line: current.line,
            lastline: 1,
            nesting_level: 0,
        })
    }

    pub fn origin_text(&self) -> &'a str {
        self.text
    }

    pub fn current_token(&self) -> LuaTokenKind {
        self.current_token
    }

    pub fn current_token_index(&self) -> usize {
        self.token_index
    }

    pub fn current_token_range(&self) -> SourceRange {
        self.current.range
    }

    pub fn previous_token_range(&self) -> SourceRange {
        self.previous.unwrap_or(SourceRange::EMPTY)
    }

    pub fn current_token_text(&self) -> &str {
        if self.current_token == LuaTokenKind::TkEof {
            return "<eof>";
        }
        let range = &self.current.range;
        &self.text[range.start_offset..range.end_offset()]
    }

    pub fn set_current_token_kind(&mut self, kind: LuaTokenKind) {
        self.current.kind = kind;
        self.current_token = kind;
    }

    pub fn bump(&mut self) {
        // Port of luaX_next from llex.c:565-573
        // Save current line before consuming next token
        self.lastline = self.line;

        self.previous = Some(self.current.range);
        self.current = self.next;
        self.current_token = self.current.kind;
        self.line = self.current.line;
        self.token_index += 1;

        if self.current.kind == LuaTokenKind::TkEof {
            self.current_token = LuaTokenKind::TkEof;
            return;
        }

        match self.tokenizer.next_token_data() {
            Ok(token) => self.next = token,
            Err(err) => {
                self.error = Some(err);
                self.next =
                    LuaTokenData::with_line(LuaTokenKind::TkEof, self.current.range, self.line);
                self.current = self.next;
                self.current_token = LuaTokenKind::TkEof;
            }
        }
    }

    pub fn peek_next_token(&self) -> LuaTokenKind {
        self.next.kind
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
}
