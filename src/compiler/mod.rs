// Lua bytecode compiler - Main module
// Compiles Lua source code to bytecode using emmylua_parser
mod expr;
mod helpers;
mod stmt;

use crate::lua_value::Chunk;
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
    pub(crate) next_register: u32,
    pub(crate) loop_stack: Vec<LoopInfo>,
    pub(crate) labels: Vec<Label>,       // Label definitions
    pub(crate) gotos: Vec<GotoInfo>,     // Pending goto statements
    pub(crate) child_chunks: Vec<Chunk>, // Nested function chunks
    pub(crate) scope_chain: Rc<RefCell<ScopeChain>>, // Scope chain for variable resolution
    pub(crate) vm_ptr: *mut LuaVM,       // VM pointer for string pool access
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
    pub is_const: bool,          // <const> attribute
    pub is_to_be_closed: bool,   // <close> attribute
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

impl<'a> Compiler<'a> {
    pub fn new(vm: &'a mut LuaVM) -> Self {
        Compiler {
            chunk: Chunk::new(),
            scope_depth: 0,
            next_register: 0,
            loop_stack: Vec::new(),
            labels: Vec::new(),
            gotos: Vec::new(),
            child_chunks: Vec::new(),
            scope_chain: ScopeChain::new(),
            vm_ptr: vm as *mut LuaVM,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a new compiler with a parent scope chain
    pub fn new_with_parent(parent_scope: Rc<RefCell<ScopeChain>>, vm_ptr: *mut LuaVM) -> Self {
        Compiler {
            chunk: Chunk::new(),
            scope_depth: 0,
            next_register: 0,
            loop_stack: Vec::new(),
            labels: Vec::new(),
            gotos: Vec::new(),
            child_chunks: Vec::new(),
            scope_chain: ScopeChain::new_with_parent(parent_scope),
            vm_ptr,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Compile Lua source code to bytecode
    /// Creates raw Box strings - VM must call register_chunk_constants() to intern them
    pub fn compile(vm: &mut LuaVM, source: &str) -> Result<Chunk, String> {
        let mut compiler = Compiler::new(vm);

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

        // Optimize child chunks first
        let optimized_children: Vec<std::rc::Rc<Chunk>> = compiler
            .child_chunks
            .into_iter()
            .map(|child| {
                let opt = optimize_chunk(child);
                std::rc::Rc::new(opt)
            })
            .collect();
        compiler.chunk.child_protos = optimized_children;

        // Apply optimization to main chunk
        let optimized = optimize_chunk(compiler.chunk);
        Ok(optimized)
    }
}

/// Compile a chunk (root node)
fn compile_chunk(c: &mut Compiler, chunk: &LuaChunk) -> Result<(), String> {
    // Emit VARARGPREP at the beginning - Lua 5.4 always emits this for main chunks
    // It adjusts vararg parameters for functions that accept ... (varargs)
    // For non-vararg functions, nparams = param_count; for vararg functions, nparams is used
    // Main chunks are always considered vararg (param_count = 0, is_vararg = true)
    c.chunk.is_vararg = true;
    emit(c, Instruction::encode_abc(OpCode::VarargPrep, 0, 0, 0));
    
    if let Some(block) = chunk.get_block() {
        compile_block(c, &block)?;
    }

    // Check for unresolved gotos before finishing
    check_unresolved_gotos(c)?;

    // Emit return at the end
    // Lua 5.4: RETURN instruction format: RETURN A B C k
    // A = first register to return (usually 0 or freereg)
    // B = number of values to return + 1 (1 means 0 returns, 2 means 1 return, 0 means return to top)
    // C = is vararg flag (main chunks are vararg, so C should be > 0)
    // k = needs to close upvalues
    // 
    // For main chunk final return:
    // - A should be the current free register (or 0 if no registers used)
    // - B = 1 (return 0 values)
    // - C = 1 (chunk is vararg, need to correct func - actually encoded in k bit + C field)
    // - k = depends on whether we have upvalues to close
    //
    // Looking at luac output, for main chunks it uses: RETURN freereg 1 1
    // Main chunk always uses regular RETURN (not Return0), with k=1
    let freereg = c.next_register;
    emit(c, Instruction::create_abck(OpCode::Return, freereg, 1, 0, true));
    Ok(())
}

/// Compile a block of statements
fn compile_block(c: &mut Compiler, block: &LuaBlock) -> Result<(), String> {
    for stat in block.get_stats() {
        compile_stat(c, &stat)?;
    }
    Ok(())
}

/// Apply optimization to a chunk
fn optimize_chunk(chunk: Chunk) -> Chunk {
    // Optimizer temporarily disabled - causes issues with loops
    // The simple constant folder doesn't handle control flow correctly
    // Need proper basic block analysis before enabling optimizations
    
    // Return chunk unchanged
    chunk
    
    /* Disabled optimizer code:
    let (optimized_code, optimized_constants) = optimize_constants(&chunk.code, &chunk.constants);
    
    Chunk {
        code: optimized_code,
        constants: optimized_constants,
        locals: chunk.locals,
        upvalue_count: chunk.upvalue_count,
        param_count: chunk.param_count,
        is_vararg: chunk.is_vararg,
        max_stack_size: chunk.max_stack_size,
        child_protos: chunk.child_protos,
        upvalue_descs: chunk.upvalue_descs,
        source_name: chunk.source_name,
        line_info: chunk.line_info,
    }
    */
}
