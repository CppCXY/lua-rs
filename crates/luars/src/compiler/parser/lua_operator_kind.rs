use crate::compiler::parser::LuaTokenKind;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum UnaryOperator {
    OpNot,  // not
    OpLen,  // #
    OpUnm,  // -
    OpBNot, // ~
    OpNop,  // (empty)
}

#[derive(Debug)]
pub struct PriorityTable {
    pub left: i32,
    pub right: i32,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum BinaryOperator {
    OpAdd,    // +
    OpSub,    // -
    OpMul,    // *
    OpDiv,    // /
    OpIDiv,   // //
    OpMod,    // %
    OpPow,    // ^
    OpBAnd,   // &
    OpBOr,    // |
    OpBXor,   // ~
    OpShl,    // <<
    OpShr,    // >>
    OpConcat, // ..
    OpLt,     // <
    OpLe,     // <=
    OpGt,     // >
    OpGe,     // >=
    OpEq,     // ==
    OpNe,     // ~=
    OpAnd,    // and
    OpOr,     // or
    OpNop,    // (empty)
}

pub const PRIORITY: [PriorityTable; 21] = [
    PriorityTable {
        left: 10,
        right: 10,
    }, // OPR_ADD
    PriorityTable {
        left: 10,
        right: 10,
    }, // OPR_SUB
    PriorityTable {
        left: 11,
        right: 11,
    }, // OPR_MUL
    PriorityTable {
        left: 11,
        right: 11,
    }, // OPR_DIV
    PriorityTable {
        left: 11,
        right: 11,
    }, // OPR_IDIV
    PriorityTable {
        left: 11,
        right: 11,
    }, // OPR_MOD
    PriorityTable {
        left: 14,
        right: 13,
    }, // OPR_POW
    PriorityTable { left: 6, right: 6 }, // OPR_BAND
    PriorityTable { left: 4, right: 4 }, // OPR_BOR
    PriorityTable { left: 5, right: 5 }, // OPR_BXOR
    PriorityTable { left: 7, right: 7 }, // OPR_SHL
    PriorityTable { left: 7, right: 7 }, // OPR_SHR
    PriorityTable { left: 9, right: 8 }, // OPR_CONCAT
    PriorityTable { left: 3, right: 3 }, // OPR_EQ
    PriorityTable { left: 3, right: 3 }, // OPR_LT
    PriorityTable { left: 3, right: 3 }, // OPR_LE
    PriorityTable { left: 3, right: 3 }, // OPR_NE
    PriorityTable { left: 3, right: 3 }, // OPR_GT
    PriorityTable { left: 3, right: 3 }, // OPR_GE
    PriorityTable { left: 2, right: 2 }, // OPR_AND
    PriorityTable { left: 1, right: 1 }, // OPR_OR
];

impl BinaryOperator {
    pub fn get_priority(&self) -> &PriorityTable {
        &PRIORITY[*self as usize]
    }
}

pub use crate::lua_vm::lua_limits::UNARY_PRIORITY;

pub fn to_unary_operator(kind: LuaTokenKind) -> UnaryOperator {
    match kind {
        LuaTokenKind::TkNot => UnaryOperator::OpNot,
        LuaTokenKind::TkLen => UnaryOperator::OpLen,
        LuaTokenKind::TkMinus => UnaryOperator::OpUnm,
        LuaTokenKind::TkBitXor => UnaryOperator::OpBNot,
        _ => UnaryOperator::OpNop,
    }
}

pub fn to_binary_operator(kind: LuaTokenKind) -> BinaryOperator {
    match kind {
        LuaTokenKind::TkPlus => BinaryOperator::OpAdd,
        LuaTokenKind::TkMinus => BinaryOperator::OpSub,
        LuaTokenKind::TkMul => BinaryOperator::OpMul,
        LuaTokenKind::TkMod => BinaryOperator::OpMod,
        LuaTokenKind::TkPow => BinaryOperator::OpPow,
        LuaTokenKind::TkDiv => BinaryOperator::OpDiv,
        LuaTokenKind::TkIDiv => BinaryOperator::OpIDiv,
        LuaTokenKind::TkBitAnd => BinaryOperator::OpBAnd,
        LuaTokenKind::TkBitOr => BinaryOperator::OpBOr,
        LuaTokenKind::TkBitXor => BinaryOperator::OpBXor,
        LuaTokenKind::TkShl => BinaryOperator::OpShl,
        LuaTokenKind::TkShr => BinaryOperator::OpShr,
        LuaTokenKind::TkConcat => BinaryOperator::OpConcat,
        LuaTokenKind::TkLt => BinaryOperator::OpLt,
        LuaTokenKind::TkLe => BinaryOperator::OpLe,
        LuaTokenKind::TkGt => BinaryOperator::OpGt,
        LuaTokenKind::TkGe => BinaryOperator::OpGe,
        LuaTokenKind::TkEq => BinaryOperator::OpEq,
        LuaTokenKind::TkNe => BinaryOperator::OpNe,
        LuaTokenKind::TkAnd => BinaryOperator::OpAnd,
        LuaTokenKind::TkOr => BinaryOperator::OpOr,
        _ => BinaryOperator::OpNop,
    }
}
