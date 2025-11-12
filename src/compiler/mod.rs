// Lua bytecode compiler - Main module
// Compiles Lua source code to bytecode using emmylua_parser

use emmylua_parser::{
    LuaBlock, LuaChunk, LuaLanguageLevel, LuaParser, ParserConfig
};

use crate::opcode::{OpCode, Instruction};
use crate::value::Chunk;

mod expr;
mod stmt;
mod helpers;

use helpers::*;
use stmt::*;

/// Compiler state
pub struct Compiler {
    pub(crate) chunk: Chunk,
    pub(crate) locals: Vec<Local>,
    pub(crate) scope_depth: usize,
    pub(crate) next_register: u32,
}

/// Local variable info
pub(crate) struct Local {
    pub name: String,
    pub depth: usize,
    pub register: u32,
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            chunk: Chunk::new(),
            locals: Vec::new(),
            scope_depth: 0,
            next_register: 0,
        }
    }

    /// Compile Lua source code to bytecode
    pub fn compile(source: &str) -> Result<Chunk, String> {
        let mut compiler = Compiler::new();
        
        let tree = LuaParser::parse(source, ParserConfig::with_level(LuaLanguageLevel::Lua54));
        
        if tree.has_syntax_errors() {
            let errors: Vec<String> = tree.get_errors()
                .iter()
                .map(|e| format!("{:?}", e))
                .collect();
            return Err(format!("Syntax errors: {}", errors.join(", ")));
        }
        
        let chunk = tree.get_chunk_node();
        compile_chunk(&mut compiler, &chunk)?;
        
        Ok(compiler.chunk)
    }
}

/// Compile a chunk (root node)
fn compile_chunk(c: &mut Compiler, chunk: &LuaChunk) -> Result<(), String> {
    if let Some(block) = chunk.get_block() {
        compile_block(c, &block)?;
    }
    
    // Emit return at the end
    emit(c, Instruction::encode_abc(OpCode::Return, 0, 1, 0));
    Ok(())
}

/// Compile a block of statements
fn compile_block(c: &mut Compiler, block: &LuaBlock) -> Result<(), String> {
    for stat in block.get_stats() {
        compile_stat(c, &stat)?;
    }
    Ok(())
}
