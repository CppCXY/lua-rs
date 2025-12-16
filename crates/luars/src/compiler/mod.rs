// Lua bytecode compiler - Main module
// Compiles Lua source code to bytecode using emmylua_parser
mod exp2reg;
mod expdesc;
mod expr;
mod helpers;
mod parse_lua_number;
mod stmt;
mod tagmethod;
mod var;

use rowan::TextRange;

use crate::lua_value::Chunk;
use crate::lua_vm::LuaVM;
use crate::lua_vm::OpCode;
// use crate::optimizer::optimize_constants;  // Disabled for now
use emmylua_parser::{
    LineIndex, LuaAstNode, LuaBlock, LuaChunk, LuaLanguageLevel, LuaParser, ParserConfig,
};
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
    pub(crate) source: &'a str,          // Source code for error reporting
    pub(crate) chunk_name: String,       // Chunk name for error reporting
    pub(crate) needclose: bool, // Function needs to close upvalues when returning (对齐lparser.h FuncState.needclose)
    pub(crate) block: Option<Box<BlockCnt>>, // Current block (对齐FuncState.bl)
    pub(crate) prev: Option<*mut Compiler<'a>>, // Enclosing function (对齐lparser.h FuncState.prev)
    pub(crate) _phantom: std::marker::PhantomData<&'a mut LuaVM>,
}

/// Block control structure (对齐lparser.c的BlockCnt)
pub(crate) struct BlockCnt {
    pub previous: Option<Box<BlockCnt>>, // Previous block in chain
    pub first_label: usize,              // Index of first label in this block
    pub first_goto: usize,               // Index of first pending goto in this block
    pub nactvar: usize,                  // Number of active locals outside the block
    pub upval: bool,                     // true if some variable in the block is an upvalue
    pub isloop: bool,                    // true if 'block' is a loop
    pub insidetbc: bool,                 // true if inside the scope of a to-be-closed var
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
    #[allow(unused)]
    pub depth: usize,
    pub reg: u32,              // Register index (对齐ridx)
    pub is_const: bool,        // <const> attribute
    pub is_to_be_closed: bool, // <close> attribute
    pub needs_close: bool,     // True if captured by a closure (needs CLOSE on scope exit)
}

/// Loop information for break statements
pub(crate) struct LoopInfo {
    pub break_jumps: Vec<usize>, // Positions of break statements to patch
    #[allow(unused)]
    pub scope_depth: usize, // Scope depth at loop start
    pub first_local_register: u32, // First register of loop-local variables (for CLOSE on break)
}

/// Label definition
pub(crate) struct Label {
    pub name: String,
    pub position: usize, // Code position where label is defined
    #[allow(unused)]
    pub scope_depth: usize, // Scope depth at label definition
    pub nactvar: usize,  // Number of active variables at label (对齐luac Label.nactvar)
}

/// Pending goto statement
pub(crate) struct GotoInfo {
    pub name: String,
    pub jump_position: usize, // Position of the jump instruction
    pub scope_depth: usize,   // Scope depth at goto statement
    pub nactvar: usize,       // Number of active variables at goto (对齐luac Labeldesc.nactvar)
    pub close: usize, // Index of last to-be-closed variable at goto (对齐luac Labeldesc.close)
                      // 0 if no to-be-closed variables
}

impl<'a> Compiler<'a> {
    pub fn new(
        vm: &'a mut LuaVM,
        line_index: &'a LineIndex,
        source: &'a str,
        chunk_name: &str,
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
            scope_chain: ScopeChain::new(),
            vm_ptr: vm as *mut LuaVM,
            last_line: 1,
            line_index,
            source,
            chunk_name: chunk_name.to_string(),
            needclose: false,
            block: None,
            prev: None, // Main compiler has no parent
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a new compiler with a parent scope chain and parent compiler
    pub fn new_with_parent(
        parent_scope: Rc<RefCell<ScopeChain>>,
        vm_ptr: *mut LuaVM,
        line_index: &'a LineIndex,
        source: &'a str,
        chunk_name: &str,
        current_line: u32,
        prev: Option<*mut Compiler<'a>>, // Parent compiler
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
            source,
            chunk_name: chunk_name.to_string(),
            needclose: false,
            block: None,
            prev,
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
        let mut compiler = Compiler::new(vm, &line_index, source, chunk_name);
        compiler.chunk.source_name = Some(chunk_name.to_string());

        let chunk_node = tree.get_chunk_node();
        compile_chunk(&mut compiler, &chunk_node)
            .map_err(|e| format!("{}:{}: {}", chunk_name, compiler.last_line, e))?;

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

/// Compile a chunk (root node) - 对齐mainfunc
fn compile_chunk(c: &mut Compiler, chunk: &LuaChunk) -> Result<(), String> {
    // Enter main block
    enter_block(c, false)?;

    // Main function is vararg
    c.chunk.is_vararg = true;
    helpers::code_abc(c, OpCode::VarargPrep, 0, 0, 0);

    // Add _ENV as first upvalue for main function (Lua 5.4 standard)
    {
        let mut scope = c.scope_chain.borrow_mut();
        let env_upvalue = Upvalue {
            name: "_ENV".to_string(),
            is_local: false, // _ENV is not a local, it comes from outside
            index: 0,        // First upvalue
        };
        scope.upvalues.push(env_upvalue);
    }

    // Compile the body
    if let Some(ref block) = chunk.get_block() {
        compile_statlist(c, block)?;
    }

    // Final return（对齐Lua C中lparser.c的mainfunc/funcbody）
    // 使用freereg而不是nvarstack，因为表达式语句可能改变freereg
    let first = c.freereg;
    helpers::ret(c, first, 0);

    // Store upvalue and local information BEFORE leaving block (对齐luac的Proto信息)
    {
        let scope = c.scope_chain.borrow();
        c.chunk.upvalue_count = scope.upvalues.len();
        c.chunk.upvalue_descs = scope
            .upvalues
            .iter()
            .map(|uv| crate::lua_value::UpvalueDesc {
                is_local: uv.is_local,
                index: uv.index,
            })
            .collect();

        // Store local variable names for debug info
        c.chunk.locals = scope.locals.iter().map(|l| l.name.clone()).collect();
    }

    // Leave main block
    leave_block(c)?;

    // Set max stack size
    if c.peak_freereg > c.chunk.max_stack_size as u32 {
        c.chunk.max_stack_size = c.peak_freereg as usize;
    }

    // 对齐luaK_finish: 最后调整RETURN/TAILCALL指令的k位和C字段
    helpers::finish(c);

    Ok(())
}

/// Compile a block of statements (对齐lparser.c的block)
pub(crate) fn compile_block(c: &mut Compiler, block: &LuaBlock) -> Result<(), String> {
    enter_block(c, false)?;
    compile_statlist(c, block)?;
    leave_block(c)?;
    Ok(())
}

/// Compile a statement list (对齐lparser.c的statlist)
pub(crate) fn compile_statlist(c: &mut Compiler, block: &LuaBlock) -> Result<(), String> {
    // statlist -> { stat [';'] }
    for stat in block.get_stats() {
        // Save line info for error reporting
        c.save_line_info(stat.get_range());
        statement(c, &stat).map_err(|e| format!("{} (at {}:{})", e, c.chunk_name, c.last_line))?;

        // Free registers after each statement
        let nvar = helpers::nvarstack(c);
        c.freereg = nvar;
    }
    Ok(())
}

/// Enter a new block (对齐enterblock)
fn enter_block(c: &mut Compiler, isloop: bool) -> Result<(), String> {
    let bl = BlockCnt {
        previous: c.block.take(),
        first_label: c.labels.len(),
        first_goto: c.gotos.len(),
        nactvar: c.nactvar,
        upval: false,
        isloop,
        insidetbc: c.block.as_ref().map_or(false, |b| b.insidetbc),
    };
    c.block = Some(Box::new(bl));

    // freereg should equal nvarstack
    // TODO: 这个断言在某些情况下会失败，需要修复freereg管理
    // debug_assert!(c.freereg == helpers::nvarstack(c));
    Ok(())
}

/// Leave current block (对齐leaveblock)
fn leave_block(c: &mut Compiler) -> Result<(), String> {
    let bl = c.block.take().expect("No block to leave");

    // Check for unresolved gotos (对齐luac leaveblock)
    let first_goto = bl.first_goto;
    if first_goto < c.gotos.len() {
        // Find first unresolved goto that's still in scope
        for i in first_goto..c.gotos.len() {
            if c.gotos[i].scope_depth > bl.nactvar {
                return Err(format!("no visible label '{}' for <goto>", c.gotos[i].name));
            }
        }
    }

    // Remove local variables
    let nvar = bl.nactvar;
    while c.nactvar > nvar {
        c.nactvar -= 1;
        let mut scope = c.scope_chain.borrow_mut();
        if !scope.locals.is_empty() {
            scope.locals.pop();
        }
    }

    // Handle break statements if this is a loop (对齐luac)
    if bl.isloop {
        // Create break label at current position
        let label_pos = helpers::get_label(c);
        // Collect break jumps before borrowing mutably
        let break_jumps: Vec<usize> = if let Some(loop_info) = c.loop_stack.last() {
            loop_info.break_jumps.clone()
        } else {
            Vec::new()
        };
        // Patch all break jumps to this position
        for &break_pc in &break_jumps {
            helpers::patch_list(c, break_pc as i32, label_pos);
        }
        // Pop loop from stack
        if !c.loop_stack.is_empty() {
            c.loop_stack.pop();
        }
    }

    // Emit CLOSE if needed
    // 参考lparser.c:682: if (!hasclose && bl->previous && bl->upval)
    if bl.upval && bl.previous.is_some() {
        let stklevel = helpers::nvarstack(c);
        helpers::code_abc(c, OpCode::Close, stklevel, 0, 0);
    }

    // Free registers
    let stklevel = helpers::nvarstack(c);
    c.freereg = stklevel;

    // Remove labels from this block
    c.labels.truncate(bl.first_label);

    // Remove gotos from this block
    c.gotos.truncate(first_goto);

    // Restore previous block
    c.block = bl.previous;

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
