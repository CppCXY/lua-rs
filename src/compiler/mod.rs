// Lua bytecode compiler - Main module
// Compiles Lua source code to bytecode using emmylua_parser
mod expr;
mod helpers;
mod stmt;

use crate::lua_value::{Chunk, StringPool};
use crate::opcode::{Instruction, OpCode};
use emmylua_parser::{LineIndex, LuaBlock, LuaChunk, LuaLanguageLevel, LuaParser, ParserConfig};
use helpers::*;
use std::cell::RefCell;
use std::rc::Rc;
use stmt::*;

/// Scope chain for variable and upvalue resolution
/// This allows efficient lookup through parent scopes without cloning
pub struct ScopeChain {
    #[allow(private_interfaces)]
    pub locals: Vec<Local>,
    #[allow(private_interfaces)]
    pub upvalues: Vec<Upvalue>,
    pub parent: Option<Rc<RefCell<ScopeChain>>>,
}

impl ScopeChain {
    pub fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(ScopeChain {
            locals: Vec::new(),
            upvalues: Vec::new(),
            parent: None,
        }))
    }

    pub fn new_with_parent(parent: Rc<RefCell<ScopeChain>>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(ScopeChain {
            locals: Vec::new(),
            upvalues: Vec::new(),
            parent: Some(parent),
        }))
    }
}

/// Compiler state
pub struct Compiler {
    pub(crate) chunk: Chunk,
    pub(crate) scope_depth: usize,
    pub(crate) next_register: u32,
    pub(crate) loop_stack: Vec<LoopInfo>,
    pub(crate) labels: Vec<Label>,       // Label definitions
    pub(crate) gotos: Vec<GotoInfo>,     // Pending goto statements
    pub(crate) child_chunks: Vec<Chunk>, // Nested function chunks
    pub(crate) scope_chain: Rc<RefCell<ScopeChain>>, // Scope chain for variable resolution
    pub(crate) string_pool: Rc<RefCell<StringPool>>, // String interning pool
}

/// Upvalue information
#[derive(Clone)]
pub(crate) struct Upvalue {
    pub name: String,
    pub is_local: bool, // true if captures local, false if captures upvalue from parent
    pub index: u32,     // Index in parent's locals or upvalues
}

/// Local variable info
#[derive(Clone)]
pub(crate) struct Local {
    pub name: String,
    pub depth: usize,
    pub register: u32,
}

/// Loop information for break statements
pub(crate) struct LoopInfo {
    pub break_jumps: Vec<usize>, // Positions of break statements to patch
}

/// Label definition
pub(crate) struct Label {
    pub name: String,
    pub position: usize,    // Code position where label is defined
    pub scope_depth: usize, // Scope depth at label definition
}

/// Pending goto statement
pub(crate) struct GotoInfo {
    pub name: String,
    pub jump_position: usize, // Position of the jump instruction
    #[allow(unused)]
    pub scope_depth: usize, // Scope depth at goto statement
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            chunk: Chunk::new(),
            scope_depth: 0,
            next_register: 0,
            loop_stack: Vec::new(),
            labels: Vec::new(),
            gotos: Vec::new(),
            child_chunks: Vec::new(),
            scope_chain: ScopeChain::new(),
            string_pool: Rc::new(RefCell::new(StringPool::new())),
        }
    }

    /// Create a new compiler with a parent scope chain and shared string pool
    pub fn new_with_parent(
        parent_scope: Rc<RefCell<ScopeChain>>,
        string_pool: Rc<RefCell<StringPool>>,
    ) -> Self {
        Compiler {
            chunk: Chunk::new(),
            scope_depth: 0,
            next_register: 0,
            loop_stack: Vec::new(),
            labels: Vec::new(),
            gotos: Vec::new(),
            child_chunks: Vec::new(),
            scope_chain: ScopeChain::new_with_parent(parent_scope),
            string_pool,
        }
    }

    /// Compile Lua source code to bytecode
    pub fn compile(source: &str) -> Result<Chunk, String> {
        let mut compiler = Compiler::new();

        let tree = LuaParser::parse(source, ParserConfig::with_level(LuaLanguageLevel::Lua54));
        let _line_index = LineIndex::parse(source);
        if tree.has_syntax_errors() {
            let errors: Vec<String> = tree
                .get_errors()
                .iter()
                .map(|e| format!("{:?}", e))
                .collect();
            return Err(format!("Syntax errors: {}", errors.join(", ")));
        }

        let chunk = tree.get_chunk_node();
        compile_chunk(&mut compiler, &chunk)?;

        // Move child chunks to main chunk
        let child_protos: Vec<std::rc::Rc<Chunk>> = compiler
            .child_chunks
            .into_iter()
            .map(std::rc::Rc::new)
            .collect();
        compiler.chunk.child_protos = child_protos;

        // Print string pool statistics (for debugging)
        let (string_count, total_bytes) = compiler.string_pool.borrow().stats();
        if string_count > 0 {
            eprintln!(
                "[StringPool] Interned {} unique strings, {} bytes total",
                string_count, total_bytes
            );
        }

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
