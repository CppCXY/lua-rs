// Lua bytecode compiler - Main module
// Port of Lua 5.4 lparser.c and lcode.c

// Submodules
mod code; // Code generation (lcode.c)
mod expr_parser; // Expression parser functions (lparser.c)
mod expression; // Expression parsing and expdesc
mod func_state; // FuncState and related structures (lparser.h)
pub mod parse_literal; // Number parsing utilities
mod parser; // Lexer/token provider
mod statement; // Statement parsing (lparser.c)

// Re-exports
pub use code::*;
pub use expression::*;
pub use func_state::*;

use crate::compiler::parser::{
    LexerConfig, LuaLanguageLevel, LuaLexer, LuaParser, LuaTokenKind, Reader,
};
use crate::gc::ObjectPool;
use crate::lua_value::Chunk;
use crate::lua_vm::OpCode;

// Structures are now in separate files (func_state.rs, expression.rs)

// Port of luaY_parser from lparser.c
pub fn compile_code(source: &str, pool: &mut ObjectPool) -> Result<Chunk, String> {
    compile_code_with_name(source, pool, "@chunk")
}

pub fn compile_code_with_name(
    source: &str,
    pool: &mut ObjectPool,
    chunk_name: &str,
) -> Result<Chunk, String> {
    let level = LuaLanguageLevel::Lua54;
    let tokenize_result = {
        let mut lexer = LuaLexer::new(
            Reader::new(source),
            LexerConfig {
                language_level: level,
            },
        );
        lexer.tokenize()
    };

    let tokens = match tokenize_result {
        Ok(tokens) => tokens,
        Err(err) => {
            return Err(format!("{}:{}", chunk_name, err));
        }
    };

    let mut parser = LuaParser::new(source, tokens, level);
    // Check for lexer errors before parsing

    let mut fs = FuncState::new(&mut parser, pool, true, chunk_name.to_string());

    // Port of mainfunc from lparser.c
    // main function is always vararg

    // Generate VARARGPREP if function is vararg
    if fs.is_vararg {
        code::code_abc(&mut fs, OpCode::VarargPrep, 0, 0, 0);
    }

    // Main function in Lua 5.4 has _ENV as first upvalue (lparser.c:1928-1931)
    // env = allocupvalue(fs);
    // env->instack = 1;
    // env->idx = 0;
    // env->kind = VDKREG;
    fs.upvalues.push(crate::compiler::func_state::Upvaldesc {
        name: "_ENV".to_string(),
        in_stack: true,
        idx: 0,
        kind: VarKind::VDKREG,
    });
    fs.nups = 1;

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
        return Err(fs.token_error("expected end of file"));
    }

    // Generate final RETURN (return with 0 values)
    code::ret(&mut fs, 0, 0);

    // Set vararg flag on chunk
    fs.chunk.is_vararg = fs.is_vararg;
    
    // Set upvalue count
    fs.chunk.upvalue_count = fs.upvalues.len();
    
    // Set source name and line info for main chunk
    fs.chunk.source_name = Some(chunk_name.to_string());
    fs.chunk.linedefined = 0;  // Main function starts at line 0
    fs.chunk.lastlinedefined = 0;  // Main function ends at line 0 (convention)

    Ok(fs.chunk)
}
