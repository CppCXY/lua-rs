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
    TkGlobal,
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
        write!(f, "{:?}", self)
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
                | LuaTokenKind::TkGlobal
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
