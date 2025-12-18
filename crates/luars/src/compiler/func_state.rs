use crate::Chunk;
// Port of FuncState and related structures from lparser.h
use crate::gc::ObjectPool;
use crate::{LuaValue, compiler::parser::LuaParser};

// Upvalue descriptor
#[derive(Clone)]
pub struct Upvaldesc {
    pub name: String,   // upvalue name
    pub in_stack: bool, // whether it is in stack (register)
    pub idx: u8,        // index of upvalue (in stack or in outer function's list)
    pub kind: VarKind,  // kind of variable
}

// Port of FuncState from lparser.h
pub struct FuncState<'a> {
    pub chunk: Chunk,
    pub prev: Option<&'a mut FuncState<'a>>, // parent function state
    pub lexer: &'a mut LuaParser<'a>,
    pub pool: &'a mut ObjectPool,
    pub block_list: Option<Box<BlockCnt>>,
    pub pc: usize,                     // next position to code (equivalent to pc)
    pub last_target: usize,            // label of last 'jump label'
    pub pending_gotos: Vec<LabelDesc>, // list of pending gotos
    pub labels: Vec<LabelDesc>,        // list of active labels
    pub actvar: Vec<VarDesc>,          // list of active local variables
    pub upvalues: Vec<Upvaldesc>,      // upvalue descriptors
    pub nactvar: u8,                   // number of active local variables
    pub nups: u8,                      // number of upvalues
    pub freereg: u8,                   // first free register
    pub iwthabs: u8,                   // instructions issued since last absolute line info
    pub needclose: bool,               // true if function needs to close upvalues when returning
    pub is_vararg: bool,               // true if function is vararg
    pub first_local: usize,            // index of first local variable in prev
    pub source_name: String,           // source file name for error messages
}

// Port of BlockCnt from lparser.c
pub struct BlockCnt {
    pub previous: Option<Box<BlockCnt>>,
    pub first_label: usize, // index of first label in this block
    pub first_goto: usize,  // index of first pending goto in this block
    pub nactvar: u8,        // number of active variables outside the block
    pub upval: bool,        // true if some variable in block is an upvalue
    pub is_loop: bool,      // true if 'block' is a loop
    pub in_scope: bool,     // true if 'block' is still in scope
}

// Port of LabelDesc from lparser.c
pub struct LabelDesc {
    pub name: String,
    pub pc: usize,
    pub line: usize,
    pub nactvar: u8,
    pub close: bool,
}

// Port of Dyndata from lparser.c
pub struct Dyndata {
    pub actvar: Vec<VarDesc>,  // list of active local variables
    pub gt: Vec<LabelDesc>,    // pending gotos
    pub label: Vec<LabelDesc>, // list of active labels
}

// Port of Vardesc from lparser.c
// Variable kinds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    VDKREG = 0,     // regular variable
    RDKCONST = 1,   // constant variable <const>
    RDKTOCLOSE = 2, // to-be-closed variable <close>
    RDKCTC = 3,     // compile-time constant
}

pub struct VarDesc {
    pub name: String,
    pub kind: VarKind,                 // variable kind
    pub ridx: i16,                     // register holding the variable
    pub vidx: u16,                     // compiler index
    pub const_value: Option<LuaValue>, // constant value for compile-time constants
}

impl<'a> FuncState<'a> {
    pub fn new(lexer: &'a mut LuaParser<'a>, pool: &'a mut ObjectPool, is_vararg: bool, source_name: String) -> Self {
        FuncState {
            chunk: Chunk::new(),
            prev: None,
            lexer,
            pool,
            block_list: None,
            pc: 0,
            last_target: 0,
            pending_gotos: Vec::new(),
            labels: Vec::new(),
            nactvar: 0,
            nups: 0,
            freereg: 0,
            iwthabs: 0,
            needclose: false,
            is_vararg,
            actvar: Vec::new(),
            upvalues: Vec::new(),
            source_name,
            first_local: 0,
        }
    }

    // Unified error generation function (port of luaX_syntaxerror from llex.c)
    pub fn syntax_error(&self, msg: &str) -> String {
        let line = self.lexer.line;
        format!("{}:{}: {}", self.source_name, line, msg)
    }

    // Generate error with current token information
    pub fn token_error(&self, msg: &str) -> String {
        let token_text = self.lexer.current_token_text();
        let line = self.lexer.line;
        format!(
            "{}:{}: {} near '{}'",
            self.source_name, line, msg, token_text
        )
    }

    // Create child function state
    pub fn new_child(parent: &'a mut FuncState<'a>, is_vararg: bool) -> Self {
        // Get references from parent - we'll need unsafe here due to borrow checker
        let lexer_ptr = parent.lexer as *mut LuaParser<'a>;
        let pool_ptr = parent.pool as *mut ObjectPool;

        FuncState {
            chunk: Chunk::new(),
            prev: Some(unsafe { &mut *(parent as *mut FuncState<'a>) }),
            lexer: unsafe { &mut *lexer_ptr },
            pool: unsafe { &mut *pool_ptr },
            block_list: None,
            pc: 0,
            last_target: 0,
            pending_gotos: Vec::new(),
            labels: Vec::new(),
            nactvar: 0,
            nups: 0,
            freereg: 0,
            iwthabs: 0,
            needclose: false,
            is_vararg,
            actvar: Vec::new(),
            upvalues: Vec::new(),
            first_local: parent.actvar.len(),
            source_name: parent.source_name.clone(),
        }
    }

    // Port of new_localvar from lparser.c
    pub fn new_localvar(&mut self, name: String, kind: VarKind) -> u16 {
        let vidx = self.actvar.len() as u16;
        self.actvar.push(VarDesc {
            name,
            kind,
            ridx: self.freereg as i16,
            vidx,
            const_value: None, // Initially no const value
        });
        vidx
    }

    // Get variable descriptor
    pub fn get_local_var_desc(&mut self, vidx: u16) -> Option<&mut VarDesc> {
        self.actvar.get_mut(vidx as usize)
    }

    // Port of adjustlocalvars from lparser.c
    pub fn adjust_local_vars(&mut self, nvars: u8) {
        let new_nactvar = self.nactvar + nvars;
        self.freereg = new_nactvar;

        for i in self.nactvar..new_nactvar {
            if let Some(var) = self.actvar.get_mut(i as usize) {
                var.ridx = i as i16;
                // Add variable name to chunk's locals for debugging
                self.chunk.locals.push(var.name.clone());
            }
        }

        self.nactvar = new_nactvar;
    }

    // Port of removevars from lparser.c
    pub fn remove_vars(&mut self, tolevel: u8) {
        while self.nactvar > tolevel {
            self.nactvar -= 1;
            self.freereg -= 1;
        }
    }

    // Port of searchvar from lparser.c (lines 390-404)
    pub fn searchvar(&self, name: &str, var: &mut crate::compiler::expression::ExpDesc) -> i32 {
        use crate::compiler::expression::ExpKind;

        for i in (0..self.nactvar as usize).rev() {
            if let Some(vd) = self.actvar.get(i) {
                if vd.name == name {
                    if vd.kind == VarKind::RDKCTC {
                        // VCONST: store variable index in u.info for check_readonly
                        *var = crate::compiler::expression::ExpDesc::new_void();
                        var.kind = ExpKind::VCONST;
                        var.u.info = i as i32;
                        return ExpKind::VCONST as i32;
                    } else {
                        // Get register index from variable descriptor
                        let ridx = vd.ridx as u8;
                        *var = crate::compiler::expression::ExpDesc::new_local(ridx, i as u16);
                        return ExpKind::VLOCAL as i32;
                    }
                }
            }
        }
        -1
    }

    // Port of searchupvalue from lparser.c (lines 340-351)
    pub fn searchupvalue(&self, name: &str) -> i32 {
        for i in 0..self.nups as usize {
            if self.upvalues[i].name == name {
                return i as i32;
            }
        }
        -1
    }

    // Port of newupvalue from lparser.c (lines 364-382)
    pub fn newupvalue(&mut self, name: &str, v: &crate::compiler::expression::ExpDesc) -> i32 {
        use crate::compiler::expression::ExpKind;

        let prev_ptr = match &self.prev {
            Some(p) => *p as *const _ as *mut FuncState,
            None => std::ptr::null_mut(),
        };

        let (in_stack, idx, kind) = unsafe {
            if v.kind == ExpKind::VLOCAL {
                let vidx = v.u.var.vidx;
                let ridx = v.u.var.ridx;
                let prev = &*prev_ptr;
                let vd = &prev.actvar[vidx as usize];
                (true, ridx as u8, vd.kind)
            } else {
                let info = v.u.info as usize;
                let prev = &*prev_ptr;
                let up = &prev.upvalues[info];
                (false, info as u8, up.kind)
            }
        };

        self.upvalues.push(Upvaldesc {
            name: name.to_string(),
            in_stack,
            idx,
            kind,
        });
        self.chunk.upvalue_count = self.upvalues.len();
        self.nups = self.upvalues.len() as u8;

        (self.nups - 1) as i32
    }
}
