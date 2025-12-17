mod error;
mod lexer;
mod lexer_config;
mod lua_language_level;
mod lua_token_data;
mod lua_token_kind;
mod parser_config;
mod reader;
mod text_range;

pub use crate::compiler::parser::{
    error::LuaParseError, lexer::LuaLexer, lua_language_level::LuaLanguageLevel,
    lua_token_data::LuaTokenData, lua_token_kind::LuaTokenKind, parser_config::ParserConfig,
    reader::Reader, text_range::SourceRange,
};

pub struct LuaParser<'a> {
    text: &'a str,
    tokens: Vec<LuaTokenData>,
    token_index: usize,
    current_token: LuaTokenKind,
    pub parse_config: ParserConfig,
    #[allow(unused)]
    pub(crate) errors: Vec<LuaParseError>,
    pub line: usize
}

impl<'a> LuaParser<'a> {
    pub fn new(text: &'a str, level: LuaLanguageLevel) -> LuaParser<'a> {
        let mut errors: Vec<LuaParseError> = Vec::new();
        let config = ParserConfig::new(level);
        let tokens = {
            let mut lexer =
                LuaLexer::new(Reader::new(text), config.lexer_config(), Some(&mut errors));
            lexer.tokenize()
        };

        let mut parser = LuaParser {
            text,
            tokens,
            token_index: 0,
            current_token: LuaTokenKind::None,
            parse_config: config,
            errors,
            line: 1
        };

        parser.init();
        parser
    }

    fn init(&mut self) {
        if self.tokens.is_empty() {
            self.current_token = LuaTokenKind::TkEof;
        } else {
            self.current_token = self.tokens[0].kind;
        }

        if is_trivia_kind(self.current_token) {
            self.bump();
        }
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
        if self.token_index >= self.tokens.len() {
            if self.tokens.is_empty() {
                return SourceRange::EMPTY;
            } else {
                return self.tokens[self.tokens.len() - 1].range;
            }
        }

        self.tokens[self.token_index].range
    }

    pub fn previous_token_range(&self) -> SourceRange {
        if self.token_index == 0 || self.tokens.is_empty() {
            return SourceRange::EMPTY;
        }

        // Find the previous non-trivia token
        let mut prev_index = self.token_index - 1;
        while prev_index > 0 && is_trivia_kind(self.tokens[prev_index].kind) {
            prev_index -= 1;
        }

        // If we found a non-trivia token or reached the first token
        if prev_index < self.tokens.len() && !is_trivia_kind(self.tokens[prev_index].kind) {
            self.tokens[prev_index].range
        } else if prev_index == 0 {
            // If the first token is also trivia, return its range anyway
            self.tokens[0].range
        } else {
            SourceRange::EMPTY
        }
    }

    pub fn current_token_text(&self) -> &str {
        let range = &self.tokens[self.token_index].range;
        &self.text[range.start_offset..range.end_offset()]
    }

    pub fn set_current_token_kind(&mut self, kind: LuaTokenKind) {
        if self.token_index < self.tokens.len() {
            self.tokens[self.token_index].kind = kind;
            self.current_token = kind;
        }
    }

    pub fn bump(&mut self) {
        let mut next_index = self.token_index + 1;
        self.skip_trivia_and_update_line(&mut next_index);
        self.token_index = next_index;

        if self.token_index >= self.tokens.len() {
            self.current_token = LuaTokenKind::TkEof;
            return;
        }

        self.current_token = self.tokens[self.token_index].kind;
    }

    pub fn peek_next_token(&self) -> LuaTokenKind {
        let mut next_index = self.token_index + 1;
        self.skip_trivia(&mut next_index);

        if next_index >= self.tokens.len() {
            LuaTokenKind::None
        } else {
            self.tokens[next_index].kind
        }
    }

    fn skip_trivia(&self, index: &mut usize) {
        if index >= &mut self.tokens.len() {
            return;
        }

        let mut kind = self.tokens[*index].kind;
        while is_trivia_kind(kind) {
            *index += 1;
            if *index >= self.tokens.len() {
                break;
            }
            kind = self.tokens[*index].kind;
        }
    }

    fn skip_trivia_and_update_line(&mut self, index: &mut usize) {
        if index >= &mut self.tokens.len() {
            return;
        }

        let mut kind = self.tokens[*index].kind;
        while is_trivia_kind(kind) {
            if kind == LuaTokenKind::TkEndOfLine {
                self.line += 1;
            }
            *index += 1;
            if *index >= self.tokens.len() {
                break;
            }
            kind = self.tokens[*index].kind;
        }
    }
}

fn is_trivia_kind(kind: LuaTokenKind) -> bool {
    matches!(
        kind,
        LuaTokenKind::TkShortComment
            | LuaTokenKind::TkLongComment
            | LuaTokenKind::TkEndOfLine
            | LuaTokenKind::TkWhitespace
            | LuaTokenKind::TkShebang
    )
}
