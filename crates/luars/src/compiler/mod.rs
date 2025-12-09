// Lua bytecode compiler - Main module
// Compiles Lua source code to bytecode using emmylua_parser
mod exp2reg;
mod expdesc;
mod expr;
mod helpers;
mod stmt;
mod tagmethod;

use rowan::TextRange;
pub(crate) use tagmethod::TagMethod;

use crate::lua_value::Chunk;
use crate::lua_value::UpvalueDesc;
use crate::lua_vm::LuaVM;
use crate::lua_vm::{Instruction, OpCode};
// use crate::optimizer::optimize_constants;  // Disabled for now
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
pub struct Compiler<'a> {
    pub(crate) chunk: Chunk,
    pub(crate) scope_depth: usize,
    pub(crate) freereg: u32, // First free register (replaces next_register)
    pub(crate) peak_freereg: u32, // Peak value of freereg (for max_stack_size)
    pub(crate) nactvar: usize, // Number of active local variables
    pub(crate) loop_stack: Vec<LoopInfo>,
    pub(crate) labels: Vec<Label>,       // Label definitions
    pub(crate) gotos: Vec<GotoInfo>,     // Pending goto statements
    pub(crate) child_chunks: Vec<Chunk>, // Nested function chunks
    pub(crate) scope_chain: Rc<RefCell<ScopeChain>>, // Scope chain for variable resolution
    pub(crate) vm_ptr: *mut LuaVM,       // VM pointer for string pool access
    pub(crate) last_line: u32,           // Last line number for line_info (not used currently)
    pub(crate) line_index: &'a LineIndex, // Line index for error reporting
    pub(crate) _phantom: std::marker::PhantomData<&'a mut LuaVM>,
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
    pub is_const: bool,        // <const> attribute
    pub is_to_be_closed: bool, // <close> attribute
    pub needs_close: bool,     // True if captured by a closure (needs CLOSE on scope exit)
}

/// Loop information for break statements
pub(crate) struct LoopInfo {
    pub break_jumps: Vec<usize>,   // Positions of break statements to patch
    pub scope_depth: usize,        // Scope depth at loop start
    pub first_local_register: u32, // First register of loop-local variables (for CLOSE on break)
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

impl<'a> Compiler<'a> {
    pub fn new(vm: &'a mut LuaVM, line_index: &'a LineIndex) -> Self {
        Compiler {
            chunk: Chunk::new(),
            scope_depth: 0,
            freereg: 0,
            peak_freereg: 0,
            nactvar: 0,
            loop_stack: Vec::new(),
            labels: Vec::new(),
            gotos: Vec::new(),
            child_chunks: Vec::new(),
            scope_chain: ScopeChain::new(),
            vm_ptr: vm as *mut LuaVM,
            last_line: 1,
            line_index,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a new compiler with a parent scope chain
    pub fn new_with_parent(
        parent_scope: Rc<RefCell<ScopeChain>>,
        vm_ptr: *mut LuaVM,
        line_index: &'a LineIndex,
        current_line: u32,
    ) -> Self {
        Compiler {
            chunk: Chunk::new(),
            scope_depth: 0,
            freereg: 0,
            peak_freereg: 0,
            nactvar: 0,
            loop_stack: Vec::new(),
            labels: Vec::new(),
            gotos: Vec::new(),
            child_chunks: Vec::new(),
            scope_chain: ScopeChain::new_with_parent(parent_scope),
            vm_ptr,
            last_line: current_line,
            line_index,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Compile Lua source code to bytecode
    /// Creates raw Box strings - VM must call register_chunk_constants() to intern them
    pub fn compile(vm: &mut LuaVM, source: &str) -> Result<Chunk, String> {
        Self::compile_with_name(vm, source, "chunk")
    }

    /// Compile Lua source code with a specific chunk name
    pub fn compile_with_name(
        vm: &mut LuaVM,
        source: &str,
        chunk_name: &str,
    ) -> Result<Chunk, String> {
        let tree = LuaParser::parse(source, ParserConfig::with_level(LuaLanguageLevel::Lua54));
        let line_index = LineIndex::parse(source);
        if tree.has_syntax_errors() {
            let errors: Vec<String> = tree
                .get_errors()
                .iter()
                .map(|e| {
                    beautify_compiler_error(&e.message, e.range, chunk_name, source, &line_index)
                })
                .collect();
            return Err(format!("Syntax errors:\n{}", errors.join("\n")));
        }
        let mut compiler = Compiler::new(vm, &line_index);
        compiler.chunk.source_name = Some(chunk_name.to_string());

        let chunk_node = tree.get_chunk_node();
        compile_chunk(&mut compiler, &chunk_node)?;

        // Optimize child chunks first
        let optimized_children: Vec<std::rc::Rc<Chunk>> = compiler
            .child_chunks
            .into_iter()
            .map(|child| {
                let opt = child;
                std::rc::Rc::new(opt)
            })
            .collect();
        compiler.chunk.child_protos = optimized_children;

        // Apply optimization to main chunk
        let optimized = compiler.chunk;
        Ok(optimized)
    }

    pub fn save_line_info(&mut self, range: TextRange) {
        if let Some(line) = self.line_index.get_line(range.start()) {
            self.last_line = (line + 1) as u32;
        }
    }
}

/// Compile a chunk (root node)
fn compile_chunk(c: &mut Compiler, chunk: &LuaChunk) -> Result<(), String> {
    // Lua 5.4: Every chunk has _ENV as upvalue[0] for accessing globals
    // Add _ENV upvalue descriptor to the chunk and scope chain
    c.chunk.upvalue_descs.push(UpvalueDesc {
        is_local: true, // Main chunk's _ENV is provided by VM
        index: 0,
    });
    c.chunk.upvalue_count = 1;

    // Add _ENV to scope chain so child functions can resolve it
    c.scope_chain.borrow_mut().upvalues.push(Upvalue {
        name: "_ENV".to_string(),
        is_local: true,
        index: 0,
    });

    // Emit VARARGPREP at the beginning
    c.chunk.is_vararg = true;
    emit(c, Instruction::encode_abc(OpCode::VarargPrep, 0, 0, 0));

    if let Some(block) = chunk.get_block() {
        compile_block(c, &block)?;
    }

    // Check for unresolved gotos before finishing
    check_unresolved_gotos(c)?;

    // Emit return at the end
    let freereg = c.freereg;
    emit(
        c,
        Instruction::create_abck(OpCode::Return, freereg, 1, 0, true),
    );
    Ok(())
}

/// Compile a block of statements
fn compile_block(c: &mut Compiler, block: &LuaBlock) -> Result<(), String> {
    for stat in block.get_stats() {
        compile_stat(c, &stat)?;
    }
    Ok(())
}

fn beautify_compiler_error(
    err: &str,
    range: TextRange,
    chunk_name: &str,
    source: &str,
    line_index: &LineIndex,
) -> String {
    let Some((line, col)) = line_index.get_line_col(range.start(), source) else {
        return err.to_string();
    };

    format!("{}:{}:{}: {}", chunk_name, line + 1, col + 1, err)
}
