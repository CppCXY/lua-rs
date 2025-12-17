// Port of FuncState and related structures from lparser.h
use crate::compiler::parser::LuaParser;
use crate::lua_value::Chunk;
use crate::gc::ObjectPool;

// Port of FuncState from lparser.h
pub struct FuncState<'a> {
    pub chunk: Chunk,
    pub prev: Option<Box<FuncState<'a>>>,
    pub lexer: &'a mut LuaParser<'a>,
    pub pool: &'a mut ObjectPool,
    pub block_list: Option<Box<BlockCnt>>,
    pub pc: usize,           // next position to code (equivalent to pc)
    pub last_target: usize,  // label of last 'jump label'
    pub pending_gotos: Vec<LabelDesc>, // list of pending gotos
    pub labels: Vec<LabelDesc>, // list of active labels
    pub actvar: Vec<VarDesc>, // list of active local variables
    pub nactvar: u8,         // number of active local variables
    pub nups: u8,            // number of upvalues
    pub freereg: u8,         // first free register
    pub iwthabs: u8,         // instructions issued since last absolute line info
    pub needclose: bool,     // true if function needs to close upvalues when returning
    pub is_vararg: bool,     // true if function is vararg
}

// Port of BlockCnt from lparser.c
pub struct BlockCnt {
    pub previous: Option<Box<BlockCnt>>,
    pub first_label: usize,  // index of first label in this block
    pub first_goto: usize,   // index of first pending goto in this block
    pub nactvar: u8,         // number of active variables outside the block
    pub upval: bool,         // true if some variable in block is an upvalue
    pub is_loop: bool,       // true if 'block' is a loop
    pub in_scope: bool,      // true if 'block' is still in scope
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
    VDKREG = 0,      // regular variable
    RDKCONST = 1,    // constant variable <const>
    RDKTOCLOSE = 2,  // to-be-closed variable <close>
    RDKCTC = 3,      // compile-time constant
}

pub struct VarDesc {
    pub name: String,
    pub kind: VarKind,  // variable kind
    pub ridx: i16,      // register holding the variable
    pub vidx: u16,      // compiler index
}

impl<'a> FuncState<'a> {
    pub fn new(lexer: &'a mut LuaParser<'a>, pool: &'a mut ObjectPool, is_vararg: bool) -> Self {
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
}
