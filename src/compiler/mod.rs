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
    pub(crate) loop_stack: Vec<LoopInfo>,
    pub(crate) labels: Vec<Label>,           // Label definitions
    pub(crate) gotos: Vec<GotoInfo>,         // Pending goto statements
    pub(crate) child_chunks: Vec<Chunk>,     // Nested function chunks
    pub(crate) upvalues: Vec<Upvalue>,       // Upvalues for current function
}

/// Upvalue information
pub(crate) struct Upvalue {
    pub name: String,
    pub is_local: bool,    // true if captures local, false if captures upvalue from parent
    pub index: u32,        // Index in parent's locals or upvalues
}

/// Local variable info
pub(crate) struct Local {
    pub name: String,
    pub depth: usize,
    pub register: u32,
}

/// Loop information for break statements
pub(crate) struct LoopInfo {
    pub break_jumps: Vec<usize>,  // Positions of break statements to patch
}

/// Label definition
pub(crate) struct Label {
    pub name: String,
    pub position: usize,        // Code position where label is defined
    pub scope_depth: usize,     // Scope depth at label definition
}

/// Pending goto statement
pub(crate) struct GotoInfo {
    pub name: String,
    pub jump_position: usize,   // Position of the jump instruction
    pub scope_depth: usize,     // Scope depth at goto statement
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            chunk: Chunk::new(),
            locals: Vec::new(),
            scope_depth: 0,
            next_register: 0,
            loop_stack: Vec::new(),
            labels: Vec::new(),
            gotos: Vec::new(),
            child_chunks: Vec::new(),
            upvalues: Vec::new(),
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
        
        // Move child chunks to main chunk
        let child_protos: Vec<std::rc::Rc<Chunk>> = compiler.child_chunks
            .into_iter()
            .map(std::rc::Rc::new)
            .collect();
        compiler.chunk.child_protos = child_protos;
        
        Ok(compiler.chunk)
    }
}

/// Compile a chunk (root node)
fn compile_chunk(c: &mut Compiler, chunk: &LuaChunk) -> Result<(), String> {
    if let Some(block) = chunk.get_block() {
        compile_block(c, &block)?;
    }
    
    // Check for unresolved gotos before finishing
    check_unresolved_gotos(c)?;
    
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
