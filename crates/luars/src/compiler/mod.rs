// Lua bytecode compiler - Main module
// Port of Lua 5.4 lparser.c and lcode.c

// Submodules
mod code;           // Code generation (lcode.c)
mod expression;     // Expression parsing and expdesc
mod expr_parser;    // Expression parser functions (lparser.c)
mod func_state;     // FuncState and related structures (lparser.h)
pub mod parse_literal; // Number parsing utilities
mod parser;         // Lexer/token provider
mod statement;      // Statement parsing (lparser.c)

// Re-exports
pub use code::*;
pub use expression::*;
pub use func_state::*;

use crate::compiler::parser::{LuaLanguageLevel, LuaParser, LuaTokenKind};
use crate::lua_value::Chunk;
use crate::gc::ObjectPool;
use crate::lua_vm::OpCode;

// Structures are now in separate files (func_state.rs, expression.rs)

// Port of luaY_parser from lparser.c
pub fn compile_code(source: &str, pool: &mut ObjectPool) -> Result<Chunk, String> {
    compile_code_with_name(source, pool, "@<input>")
}

pub fn compile_code_with_name(source: &str, pool: &mut ObjectPool, chunk_name: &str) -> Result<Chunk, String> {
    let level = LuaLanguageLevel::Lua54;
    let mut parser = LuaParser::new(source, level);
    
    let mut fs = FuncState::new(&mut parser, pool, true);
    fs.source_name = chunk_name.to_string();
    
    // Port of mainfunc from lparser.c
    // main function is always vararg
    
    // Generate VARARGPREP if function is vararg
    if fs.is_vararg {
        code::code_abc(&mut fs, OpCode::VarargPrep, 0, 0, 0);
    }
    
    // Main function in Lua 5.4 has _ENV as first local variable
    fs.new_localvar("_ENV".to_string(), VarKind::VDKREG);
    fs.adjust_local_vars(1);
    
    // Open first block  
    let bl = BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: false,
        in_scope: true,
    };
    fs.block_list = Some(Box::new(bl));
    
    // Parse statements (statlist)
    statement::statlist(&mut fs)?;
    
    // Check for proper ending
    if fs.lexer.current_token() != LuaTokenKind::TkEof {
        return Err(format!(
            "{}:{}: syntax error: expected end of file, got '{}'",
            fs.source_name,
            fs.lexer.line,
            fs.lexer.current_token_text()
        ));
    }
    
    // Generate final RETURN (return with 0 values)
    code::ret(&mut fs, 0, 0);
    
    Ok(fs.chunk)
}

