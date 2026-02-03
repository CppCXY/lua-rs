use crate::compiler::parser::{
    lua_token_data::LuaTokenData, lua_token_kind::LuaTokenKind, reader::Reader,
};

use super::tokenize_config::TokensizeConfig;

pub struct LuaTokenize<'a> {
    reader: Reader<'a>,
    lexer_config: TokensizeConfig,
    error: Option<String>,
    line: usize,
}

impl<'a> LuaTokenize<'a> {
    pub fn new(reader: Reader<'a>, lexer_config: TokensizeConfig) -> Self {
        LuaTokenize {
            reader,
            lexer_config,
            error: None,
            line: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<LuaTokenData>, String> {
        let mut tokens = vec![];

        while !self.reader.is_eof() {
            let kind = self.lex();
            if kind == LuaTokenKind::TkEof || self.error.is_some() {
                break;
            }

            // Save token with its ending line number (matches Lua's linenumber)
            tokens.push(LuaTokenData::with_line(
                kind,
                self.reader.current_range(),
                self.line,
            ));
        }

        if let Some(err) = &self.error {
            return Err(err.clone());
        }

        Ok(tokens)
    }

    fn name_to_kind(&self, name: &str) -> LuaTokenKind {
        match name {
            "and" => LuaTokenKind::TkAnd,
            "break" => LuaTokenKind::TkBreak,
            "do" => LuaTokenKind::TkDo,
            "else" => LuaTokenKind::TkElse,
            "elseif" => LuaTokenKind::TkElseIf,
            "end" => LuaTokenKind::TkEnd,
            "false" => LuaTokenKind::TkFalse,
            "for" => LuaTokenKind::TkFor,
            "function" => LuaTokenKind::TkFunction,
            "goto" => LuaTokenKind::TkGoto,
            "if" => LuaTokenKind::TkIf,
            "in" => LuaTokenKind::TkIn,
            "local" => LuaTokenKind::TkLocal,
            "nil" => LuaTokenKind::TkNil,
            "not" => LuaTokenKind::TkNot,
            "or" => LuaTokenKind::TkOr,
            "repeat" => LuaTokenKind::TkRepeat,
            "return" => LuaTokenKind::TkReturn,
            "then" => LuaTokenKind::TkThen,
            "true" => LuaTokenKind::TkTrue,
            "until" => LuaTokenKind::TkUntil,
            "while" => LuaTokenKind::TkWhile,
            _ => LuaTokenKind::TkName,
        }
    }

    fn lex(&mut self) -> LuaTokenKind {
        self.reader.reset_buff();

        match self.reader.current_char() {
            '\n' | '\r' => self.lex_new_line(),
            ' ' | '\t' => self.lex_white_space(),
            '-' => {
                self.reader.bump();

                if self.reader.current_char() != '-' {
                    return LuaTokenKind::TkMinus;
                }

                self.reader.bump();
                if self.reader.current_char() == '[' {
                    self.reader.bump();
                    let sep = self.skip_sep();
                    if self.reader.current_char() == '[' {
                        self.reader.bump();
                        self.lex_long_string(sep);
                        return LuaTokenKind::TkLongComment;
                    }
                }

                self.reader.eat_while(|ch| ch != '\n' && ch != '\r');
                LuaTokenKind::TkShortComment
            }
            '[' => {
                self.reader.bump();
                let sep = self.skip_sep();
                if sep == 0 && self.reader.current_char() != '[' {
                    return LuaTokenKind::TkLeftBracket;
                }
                if self.reader.current_char() != '[' {
                    self.error(|| format!("invalid long string delimiter"));
                    return LuaTokenKind::TkLongString;
                }

                self.reader.bump();
                self.lex_long_string(sep)
            }
            '=' => {
                self.reader.bump();
                if self.reader.current_char() != '=' {
                    return LuaTokenKind::TkAssign;
                }
                self.reader.bump();
                LuaTokenKind::TkEq
            }
            '<' => {
                self.reader.bump();
                match self.reader.current_char() {
                    '=' => {
                        self.reader.bump();
                        LuaTokenKind::TkLe
                    }
                    '<' => {
                        if !self.lexer_config.support_integer_operation() {
                            self.error(|| format!("bitwise operation is not supported"));
                        }

                        self.reader.bump();
                        LuaTokenKind::TkShl
                    }
                    _ => LuaTokenKind::TkLt,
                }
            }
            '>' => {
                self.reader.bump();
                match self.reader.current_char() {
                    '=' => {
                        self.reader.bump();
                        LuaTokenKind::TkGe
                    }
                    '>' => {
                        if !self.lexer_config.support_integer_operation() {
                            self.error(|| format!("bitwise operation is not supported"));
                        }

                        self.reader.bump();
                        LuaTokenKind::TkShr
                    }
                    _ => LuaTokenKind::TkGt,
                }
            }
            '~' => {
                self.reader.bump();
                if self.reader.current_char() != '=' {
                    if !self.lexer_config.support_integer_operation() {
                        self.error(|| format!("bitwise operation is not supported"));
                    }
                    return LuaTokenKind::TkBitXor;
                }
                self.reader.bump();
                LuaTokenKind::TkNe
            }
            ':' => {
                self.reader.bump();
                if self.reader.current_char() != ':' {
                    return LuaTokenKind::TkColon;
                }
                self.reader.bump();
                LuaTokenKind::TkDbColon
            }
            '"' | '\'' | '`' => {
                let quote = self.reader.current_char();
                self.reader.bump();
                self.lex_string(quote)
            }
            '.' => {
                if self.reader.next_char().is_ascii_digit() {
                    return self.lex_number();
                }

                self.reader.bump();
                if self.reader.current_char() != '.' {
                    return LuaTokenKind::TkDot;
                }
                self.reader.bump();
                if self.reader.current_char() != '.' {
                    return LuaTokenKind::TkConcat;
                }
                self.reader.bump();
                LuaTokenKind::TkDots
            }
            '0'..='9' => self.lex_number(),
            '/' => {
                self.reader.bump();
                let current_char = self.reader.current_char();
                match current_char {
                    _ if current_char != '/' => LuaTokenKind::TkDiv,
                    _ => {
                        if !self.lexer_config.support_integer_operation() {
                            self.error(|| format!("integer division is not supported"));
                        }

                        self.reader.bump();
                        LuaTokenKind::TkIDiv
                    }
                }
            }
            '*' => {
                self.reader.bump();
                LuaTokenKind::TkMul
            }
            '+' => {
                self.reader.bump();
                LuaTokenKind::TkPlus
            }
            '%' => {
                self.reader.bump();
                LuaTokenKind::TkMod
            }
            '^' => {
                self.reader.bump();
                LuaTokenKind::TkPow
            }
            '#' => {
                // Check if shebang BEFORE bumping
                let is_line_start = self.reader.is_start_of_line();
                self.reader.bump();
                
                // Shebang only on first line at start, and must be followed by !
                if is_line_start && self.line == 1 {
                    self.reader.eat_while(|ch| ch != '\n' && ch != '\r');
                    return LuaTokenKind::TkShebang;
                }

                // Otherwise it's the length operator
                LuaTokenKind::TkLen
            }
            '&' => {
                self.reader.bump();
                if !self.lexer_config.support_integer_operation() {
                    self.error(|| format!("bitwise operation is not supported"));
                }
                LuaTokenKind::TkBitAnd
            }
            '|' => {
                self.reader.bump();
                if !self.lexer_config.support_integer_operation() {
                    self.error(|| format!("bitwise operation is not supported"));
                }
                LuaTokenKind::TkBitOr
            }
            '(' => {
                self.reader.bump();
                LuaTokenKind::TkLeftParen
            }
            ')' => {
                self.reader.bump();
                LuaTokenKind::TkRightParen
            }
            '{' => {
                self.reader.bump();
                LuaTokenKind::TkLeftBrace
            }
            '}' => {
                self.reader.bump();
                LuaTokenKind::TkRightBrace
            }
            ']' => {
                self.reader.bump();
                LuaTokenKind::TkRightBracket
            }
            ';' => {
                self.reader.bump();
                LuaTokenKind::TkSemicolon
            }
            ',' => {
                self.reader.bump();
                LuaTokenKind::TkComma
            }
            _ if self.reader.is_eof() => LuaTokenKind::TkEof,
            ch if is_name_start(ch) => {
                self.reader.bump();
                self.reader.eat_while(is_name_continue);
                let name = self.reader.current_text();
                self.name_to_kind(name)
            }
            _ => {
                self.reader.bump();
                LuaTokenKind::TkUnknown
            }
        }
    }

    fn lex_new_line(&mut self) -> LuaTokenKind {
        match self.reader.current_char() {
            // support \n or \n\r
            '\n' => {
                self.reader.bump();
                if self.reader.current_char() == '\r' {
                    self.reader.bump();
                }
            }
            // support \r or \r\n
            '\r' => {
                self.reader.bump();
                if self.reader.current_char() == '\n' {
                    self.reader.bump();
                }
            }
            _ => {}
        }
        self.line += 1;

        LuaTokenKind::TkEndOfLine
    }

    fn lex_white_space(&mut self) -> LuaTokenKind {
        self.reader.eat_while(|ch| ch == ' ' || ch == '\t' || ch == '\x0B' || ch == '\x0C');
        LuaTokenKind::TkWhitespace
    }

    fn skip_sep(&mut self) -> usize {
        self.reader.eat_when('=')
    }

    fn lex_string(&mut self, quote: char) -> LuaTokenKind {
        while !self.reader.is_eof() {
            let ch = self.reader.current_char();
            if ch == quote || ch == '\n' || ch == '\r' {
                break;
            }

            if ch != '\\' {
                self.reader.bump();
                continue;
            }

            self.reader.bump();
            match self.reader.current_char() {
                'z' => {
                    self.reader.bump();
                    // Skip whitespace after \z, tracking line numbers
                    while !self.reader.is_eof() {
                        let c = self.reader.current_char();
                        if c == ' ' || c == '\t' || c == '\x0B' || c == '\x0C' {
                            self.reader.bump();
                        } else if c == '\r' || c == '\n' {
                            self.lex_new_line();
                        } else {
                            break;
                        }
                    }
                }
                'x' => {
                    // Hexadecimal escape: \xHH
                    self.reader.bump(); // skip 'x'
                    // Need exactly 2 hex digits
                    let ch1 = self.reader.current_char();
                    if !ch1.is_ascii_hexdigit() {
                        // Build error message with context
                        let mut ctx = String::from("\\x");
                        if ch1 != '\0' && ch1 != '\n' && ch1 != '\r' {
                            ctx.push(ch1);
                        }
                        self.error(|| format!("hexadecimal digit expected near '{}'", ctx));
                        return LuaTokenKind::TkString;
                    }
                    self.reader.bump();
                    
                    let ch2 = self.reader.current_char();
                    if !ch2.is_ascii_hexdigit() {
                        // Build error message with context
                        let mut ctx = String::from("\\x");
                        ctx.push(ch1);
                        if ch2 != '\0' && ch2 != '\n' && ch2 != '\r' {
                            ctx.push(ch2);
                        }
                        self.error(|| format!("hexadecimal digit expected near '{}'", ctx));
                        return LuaTokenKind::TkString;
                    }
                    self.reader.bump();
                }
                'u' => {
                    // Unicode escape: \u{XXX}
                    self.reader.bump(); // skip 'u'
                    if self.reader.current_char() != '{' {
                        // Missing '{' after \u
                        // Get context: extract chars before current position
                        // current_text() includes from token start (the quote) to after 'u'
                        let text_before = self.reader.current_text();
                        // Skip opening quote and get last few chars
                        let context_start = if text_before.len() > 6 {
                            // Get last 5 chars (will include `abc\u`)
                            &text_before[text_before.len() - 5..]
                        } else if text_before.len() > 1 {
                            &text_before[1..]  // Skip opening quote
                        } else {
                            ""
                        };
                        let mut ctx = String::from(context_start);
                        let next_ch = self.reader.current_char();
                        if next_ch != '\0' && next_ch != '\n' && next_ch != '\r' {
                            ctx.push(next_ch);
                        }
                        self.error(|| format!("missing '{{' in unicode escape near '{}'", ctx));
                        return LuaTokenKind::TkString;
                    }
                    self.reader.bump(); // skip '{'
                    
                    // Collect hex digits
                    let mut hex_digits = String::new();
                    while self.reader.current_char() != '}' {
                        let ch = self.reader.current_char();
                        if ch == '\0' || ch == '\n' || ch == '\r' {
                            // Unfinished escape
                            let text_before = self.reader.current_text();
                            let context_start = if text_before.len() > 11 {
                                &text_before[text_before.len() - 10..]
                            } else if text_before.len() > 1 {
                                &text_before[1..]
                            } else {
                                ""
                            };
                            let ctx = String::from(context_start);
                            self.error(|| format!("unfinished unicode escape near '{}'", ctx));
                            return LuaTokenKind::TkString;
                        }
                        if !ch.is_ascii_hexdigit() {
                            // Non-hex character
                            let text_before = self.reader.current_text();
                            let context_start = if text_before.len() > 11 {
                                &text_before[text_before.len() - 10..]
                            } else if text_before.len() > 1 {
                                &text_before[1..]
                            } else {
                                ""
                            };
                            let mut ctx = String::from(context_start);
                            ctx.push(ch);
                            self.error(|| format!("hexadecimal digit expected in unicode escape near '{}'", ctx));
                            return LuaTokenKind::TkString;
                        }
                        hex_digits.push(ch);
                        self.reader.bump();
                    }
                    
                    if hex_digits.is_empty() {
                        let text_before = self.reader.current_text();
                        let context_start = if text_before.len() > 11 {
                            &text_before[text_before.len() - 10..]
                        } else if text_before.len() > 1 {
                            &text_before[1..]
                        } else {
                            ""
                        };
                        let mut ctx = String::from(context_start);
                        let next_ch = self.reader.current_char();
                        if next_ch != '\0' && next_ch != '\n' && next_ch != '\r' {
                            ctx.push(next_ch);
                        }
                        self.error(|| format!("hexadecimal digit expected in unicode escape near '{}'", ctx));
                        return LuaTokenKind::TkString;
                    }
                    
                    // Validate UTF-8 value range (0 to 0x7FFFFFFF)
                    match u32::from_str_radix(&hex_digits, 16) {
                        Ok(val) if val > 0x7FFFFFFF => {
                            // Value too large, include context in error
                            let text_before = self.reader.current_text();
                            let context_start = if text_before.len() > 16 {
                                &text_before[text_before.len() - 15..]
                            } else if text_before.len() > 1 {
                                &text_before[1..]
                            } else {
                                ""
                            };
                            // Don't include the closing } - the error is about the value
                            // being too large, which is detected before we accept the }
                            let ctx = String::from(context_start);
                            self.error(|| format!("UTF-8 value too large near '{}'", ctx));
                            return LuaTokenKind::TkString;
                        }
                        Err(_) => {
                            // Parse error means value too large for u32
                            let text_before = self.reader.current_text();
                            let context_start = if text_before.len() > 16 {
                                &text_before[text_before.len() - 15..]
                            } else if text_before.len() > 1 {
                                &text_before[1..]
                            } else {
                                ""
                            };
                            let ctx = String::from(context_start);
                            self.error(|| format!("UTF-8 value too large near '{}'", ctx));
                            return LuaTokenKind::TkString;
                        }
                        Ok(_) => {
                            // Valid value, skip '}'
                            self.reader.bump();
                        }
                    }
                }
                '\r' | '\n' => {
                    self.lex_new_line();
                }
                '0'..='9' => {
                    // Decimal escape: \DDD (up to 3 digits, max 255)
                    let start_ch = self.reader.current_char();
                    let mut digits = String::new();
                    digits.push(start_ch);
                    self.reader.bump();
                    
                    let mut count = 1;
                    while count < 3 && self.reader.current_char().is_ascii_digit() {
                        digits.push(self.reader.current_char());
                        self.reader.bump();
                        count += 1;
                    }
                    
                    // Validate range (0-255)
                    if let Ok(val) = digits.parse::<u16>() {
                        if val > 255 {
                            // Include next char in error context if it's not special
                            let mut ctx = format!("\\{}", digits);
                            let next_ch = self.reader.current_char();
                            if next_ch != '\0' && next_ch != '\n' && next_ch != '\r' {
                                ctx.push(next_ch);
                            }
                            self.error(|| format!("decimal escape too large near '{}'", ctx));
                            return LuaTokenKind::TkString;
                        }
                    }
                }
                'a' | 'b' | 'f' | 'n' | 'r' | 't' | 'v' | '\\' | '\'' | '\"' => {
                    // Valid single-character escapes
                    self.reader.bump();
                }
                _ => {
                    // Invalid escape sequence
                    let ch = self.reader.current_char();
                    self.error(|| format!("invalid escape sequence near '\\{}'", ch));
                    return LuaTokenKind::TkString;
                }
            }
        }

        if self.reader.current_char() != quote {
            self.error(|| format!("unfinished string near <eof>"));
            return LuaTokenKind::TkString;
        }

        self.reader.bump();
        LuaTokenKind::TkString
    }

    fn lex_long_string(&mut self, sep: usize) -> LuaTokenKind {
        let mut end = false;
        while !self.reader.is_eof() {
            match self.reader.current_char() {
                ']' => {
                    self.reader.bump();
                    let count = self.reader.eat_when('=');
                    if count == sep && self.reader.current_char() == ']' {
                        self.reader.bump();
                        end = true;
                        break;
                    }
                }
                '\n' | '\r' => {
                    self.lex_new_line();
                }
                _ => {
                    self.reader.bump();
                }
            }
        }

        if !end {
            self.error(|| format!("unfinished long string or comment near <eof>"));
        }

        LuaTokenKind::TkLongString
    }

    fn lex_number(&mut self) -> LuaTokenKind {
        enum NumberState {
            Int,
            Float,
            Hex,
            HexFloat,
            WithExpo,
            Bin,
        }

        let mut state = NumberState::Int;
        let first = self.reader.current_char();
        self.reader.bump();
        match first {
            '0' if matches!(self.reader.current_char(), 'X' | 'x') => {
                self.reader.bump();
                state = NumberState::Hex;
            }
            '0' if matches!(self.reader.current_char(), 'B' | 'b')
                && self.lexer_config.support_binary_integer() =>
            {
                self.reader.bump();
                state = NumberState::Bin;
            }
            '.' => {
                state = NumberState::Float;
            }
            _ => {}
        }

        while !self.reader.is_eof() {
            let ch = self.reader.current_char();
            let continue_ = match state {
                NumberState::Int => match ch {
                    '0'..='9' => true,
                    '.' => {
                        state = NumberState::Float;
                        true
                    }
                    _ if matches!(self.reader.current_char(), 'e' | 'E') => {
                        if matches!(self.reader.next_char(), '+' | '-') {
                            self.reader.bump();
                        }
                        state = NumberState::WithExpo;
                        true
                    }
                    _ => false,
                },
                NumberState::Float => match ch {
                    '0'..='9' => true,
                    _ if matches!(self.reader.current_char(), 'e' | 'E') => {
                        if matches!(self.reader.next_char(), '+' | '-') {
                            self.reader.bump();
                        }
                        state = NumberState::WithExpo;
                        true
                    }
                    _ => false,
                },
                NumberState::Hex => match ch {
                    '0'..='9' | 'a'..='f' | 'A'..='F' => true,
                    '.' => {
                        state = NumberState::HexFloat;
                        true
                    }
                    _ if matches!(self.reader.current_char(), 'P' | 'p') => {
                        if matches!(self.reader.next_char(), '+' | '-') {
                            self.reader.bump();
                        }
                        state = NumberState::WithExpo;
                        true
                    }
                    _ => false,
                },
                NumberState::HexFloat => match ch {
                    '0'..='9' | 'a'..='f' | 'A'..='F' => true,
                    _ if matches!(self.reader.current_char(), 'P' | 'p') => {
                        if matches!(self.reader.next_char(), '+' | '-') {
                            self.reader.bump();
                        }
                        state = NumberState::WithExpo;
                        true
                    }
                    _ => false,
                },
                NumberState::WithExpo => ch.is_ascii_digit(),
                NumberState::Bin => matches!(ch, '0' | '1'),
            };

            if continue_ {
                self.reader.bump();
            } else {
                break;
            }
        }

        if self.lexer_config.support_complex_number() && self.reader.current_char() == 'i' {
            self.reader.bump();
            return LuaTokenKind::TkComplex;
        }

        if self.lexer_config.support_ll_integer()
            && matches!(
                state,
                NumberState::Int | NumberState::Hex | NumberState::Bin
            )
        {
            self.reader
                .eat_while(|ch| matches!(ch, 'u' | 'U' | 'l' | 'L'));
            return LuaTokenKind::TkInt;
        }

        if self.reader.current_char().is_alphabetic() {
            let ch = self.reader.current_char();
            self.error(|| format!("unexpected character '%{ch}' after number literal", ch = ch));
        }

        match state {
            NumberState::Int | NumberState::Hex => LuaTokenKind::TkInt,
            _ => LuaTokenKind::TkFloat,
        }
    }

    fn error<F, R>(&mut self, msg: F)
    where
        F: FnOnce() -> R,
        R: AsRef<str>,
    {
        self.error = Some(format!("{}: {}", self.line, msg().as_ref()));
    }
}

fn is_name_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_'
}

fn is_name_continue(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}
