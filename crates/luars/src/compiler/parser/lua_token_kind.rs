use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum LuaTokenKind {
    None,
    // KeyWord
    TkAnd,
    TkBreak,
    TkDo,
    TkElse,
    TkElseIf,
    TkEnd,
    TkFalse,
    TkFor,
    TkFunction,
    TkGoto,
    TkIf,
    TkIn,
    TkLocal,
    TkNil,
    TkNot,
    TkOr,
    TkRepeat,
    TkReturn,
    TkThen,
    TkTrue,
    TkUntil,
    TkWhile,

    TkWhitespace, // whitespace
    TkEndOfLine,  // end of line
    TkPlus,       // +
    TkMinus,      // -
    TkMul,        // *
    TkDiv,        // /
    TkIDiv,       // //
    TkDot,        // .
    TkConcat,     // ..
    TkDots,       // ...
    TkComma,      // ,
    TkAssign,     // =
    TkEq,         // ==
    TkGe,         // >=
    TkLe,         // <=
    TkNe,         // ~=
    TkShl,        // <<
    TkShr,        // >>
    TkLt,         // <
    TkGt,         // >
    TkMod,        // %
    TkPow,        // ^
    TkLen,        // #
    TkBitAnd,     // &
    TkBitOr,      // |
    TkBitXor,     // ~
    TkColon,      // :
    TkDbColon,    // ::
    TkSemicolon,  // ;

    TkLeftBracket,  // [
    TkRightBracket, // ]
    TkLeftParen,    // (
    TkRightParen,   // )
    TkLeftBrace,    // {
    TkRightBrace,   // }
    TkComplex,      // complex
    TkInt,          // int
    TkFloat,        // float

    TkName,         // name
    TkString,       // string
    TkLongString,   // long string
    TkShortComment, // short comment
    TkLongComment,  // long comment
    TkShebang,      // shebang
    TkEof,          // eof

    TkUnknown, // unknown
}

impl fmt::Display for LuaTokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_user_string())
    }
}

impl LuaTokenKind {
    /// Convert token kind to user-readable string (like Lua's luaX_token2str)
    pub fn to_user_string(&self) -> &'static str {
        match self {
            // Keywords
            LuaTokenKind::TkAnd => "and",
            LuaTokenKind::TkBreak => "break",
            LuaTokenKind::TkDo => "do",
            LuaTokenKind::TkElse => "else",
            LuaTokenKind::TkElseIf => "elseif",
            LuaTokenKind::TkEnd => "end",
            LuaTokenKind::TkFalse => "false",
            LuaTokenKind::TkFor => "for",
            LuaTokenKind::TkFunction => "function",
            LuaTokenKind::TkGoto => "goto",
            LuaTokenKind::TkIf => "if",
            LuaTokenKind::TkIn => "in",
            LuaTokenKind::TkLocal => "local",
            LuaTokenKind::TkNil => "nil",
            LuaTokenKind::TkNot => "not",
            LuaTokenKind::TkOr => "or",
            LuaTokenKind::TkRepeat => "repeat",
            LuaTokenKind::TkReturn => "return",
            LuaTokenKind::TkThen => "then",
            LuaTokenKind::TkTrue => "true",
            LuaTokenKind::TkUntil => "until",
            LuaTokenKind::TkWhile => "while",
            // Symbols
            LuaTokenKind::TkPlus => "+",
            LuaTokenKind::TkMinus => "-",
            LuaTokenKind::TkMul => "*",
            LuaTokenKind::TkDiv => "/",
            LuaTokenKind::TkIDiv => "//",
            LuaTokenKind::TkDot => ".",
            LuaTokenKind::TkConcat => "..",
            LuaTokenKind::TkDots => "...",
            LuaTokenKind::TkComma => ",",
            LuaTokenKind::TkAssign => "=",
            LuaTokenKind::TkEq => "==",
            LuaTokenKind::TkGe => ">=",
            LuaTokenKind::TkLe => "<=",
            LuaTokenKind::TkNe => "~=",
            LuaTokenKind::TkShl => "<<",
            LuaTokenKind::TkShr => ">>",
            LuaTokenKind::TkLt => "<",
            LuaTokenKind::TkGt => ">",
            LuaTokenKind::TkMod => "%",
            LuaTokenKind::TkPow => "^",
            LuaTokenKind::TkLen => "#",
            LuaTokenKind::TkBitAnd => "&",
            LuaTokenKind::TkBitOr => "|",
            LuaTokenKind::TkBitXor => "~",
            LuaTokenKind::TkColon => ":",
            LuaTokenKind::TkDbColon => "::",
            LuaTokenKind::TkSemicolon => ";",
            LuaTokenKind::TkLeftBracket => "[",
            LuaTokenKind::TkRightBracket => "]",
            LuaTokenKind::TkLeftParen => "(",
            LuaTokenKind::TkRightParen => ")",
            LuaTokenKind::TkLeftBrace => "{",
            LuaTokenKind::TkRightBrace => "}",
            // Literals
            LuaTokenKind::TkInt => "<integer>",
            LuaTokenKind::TkFloat => "<number>",
            LuaTokenKind::TkName => "<name>",
            LuaTokenKind::TkString => "<string>",
            LuaTokenKind::TkLongString => "<string>",
            LuaTokenKind::TkEof => "<eof>",
            // Others
            _ => "<unknown>",
        }
    }
}

impl LuaTokenKind {
    pub fn is_keyword(self) -> bool {
        matches!(
            self,
            LuaTokenKind::TkAnd
                | LuaTokenKind::TkBreak
                | LuaTokenKind::TkDo
                | LuaTokenKind::TkElse
                | LuaTokenKind::TkElseIf
                | LuaTokenKind::TkEnd
                | LuaTokenKind::TkFalse
                | LuaTokenKind::TkFor
                | LuaTokenKind::TkFunction
                | LuaTokenKind::TkGoto
                | LuaTokenKind::TkIf
                | LuaTokenKind::TkIn
                | LuaTokenKind::TkLocal
                | LuaTokenKind::TkNil
                | LuaTokenKind::TkNot
                | LuaTokenKind::TkOr
                | LuaTokenKind::TkRepeat
                | LuaTokenKind::TkReturn
                | LuaTokenKind::TkThen
                | LuaTokenKind::TkTrue
                | LuaTokenKind::TkUntil
                | LuaTokenKind::TkWhile
        )
    }
}
