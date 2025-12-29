use crate::Chunk;
use crate::compiler::{ExpDesc, ExpKind, ExpUnion};
// Port of FuncState and related structures from lparser.h
use crate::gc::{ObjectPool, TableId};
use crate::{LuaValue, compiler::parser::LuaLexer};

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
    pub lexer: &'a mut LuaLexer<'a>,
    pub pool: &'a mut ObjectPool,
    pub compiler_state: &'a mut CompilerState,
    pub block_cnt_id: Option<BlockCntId>,
    pub pc: usize,                     // next position to code (equivalent to pc)
    pub last_target: usize,            // label of last 'jump label'
    pub pending_gotos: Vec<LabelDesc>, // list of pending gotos
    pub labels: Vec<LabelDesc>,        // list of active labels
    pub actvar: Vec<VarDesc>,          // list of all variable descriptors (active and pending)
    pub nactvar: u8,                   // number of active variables (actvar[0..nactvar] are active)
    pub upvalues: Vec<Upvaldesc>,      // upvalue descriptors
    pub nups: u8,                      // number of upvalues
    pub freereg: u8,                   // first free register
    pub iwthabs: u8,                   // instructions issued since last absolute line info
    pub needclose: bool,               // true if function needs to close upvalues when returning
    pub is_vararg: bool,               // true if function is vararg
    pub numparams: u8,                 // number of fixed parameters (excluding vararg parameter)
    pub first_local: usize,            // index of first local variable in prev
    pub source_name: String,           // source file name for error messages
    pub kcache: TableId, // cache table for constant deduplication (per-function, like Lua 5.5's fs->kcache)
}

pub struct CompilerState {
    // pool of BlockCnt structures (Option to allow safe take without invalidating indices)
    pub block_cnt_pool: Vec<Option<BlockCnt>>,
    // pool of LhsAssign structures for assignment chain management
    pub lhs_assign_pool: Vec<Option<LhsAssign>>,
}

impl CompilerState {
    pub fn new() -> Self {
        CompilerState {
            block_cnt_pool: Vec::new(),
            lhs_assign_pool: Vec::new(),
        }
    }

    pub fn alloc_blockcnt(&mut self, block: BlockCnt) -> BlockCntId {
        let id = BlockCntId(self.block_cnt_pool.len());
        self.block_cnt_pool.push(Some(block));
        id
    }

    pub fn get_blockcnt_mut(&mut self, id: BlockCntId) -> Option<&mut BlockCnt> {
        self.block_cnt_pool
            .get_mut(id.0)
            .and_then(|opt| opt.as_mut())
    }

    pub fn take_blockcnt(&mut self, id: BlockCntId) -> Option<BlockCnt> {
        // Use take() instead of remove() to avoid invalidating subsequent BlockCntIds
        self.block_cnt_pool.get_mut(id.0).and_then(|opt| opt.take())
    }

    pub fn alloc_lhs_assign(&mut self, lhs: LhsAssign) -> LhsAssignId {
        let id = LhsAssignId(self.lhs_assign_pool.len());
        self.lhs_assign_pool.push(Some(lhs));
        id
    }

    pub fn get_lhs_assign(&self, id: LhsAssignId) -> Option<&LhsAssign> {
        self.lhs_assign_pool.get(id.0).and_then(|opt| opt.as_ref())
    }

    pub fn get_lhs_assign_mut(&mut self, id: LhsAssignId) -> Option<&mut LhsAssign> {
        self.lhs_assign_pool
            .get_mut(id.0)
            .and_then(|opt| opt.as_mut())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockCntId(pub usize);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LhsAssignId(pub usize);

// Port of LHS_assign from lparser.c
#[derive(Clone)]
pub struct LhsAssign {
    pub prev: Option<LhsAssignId>,
    pub v: ExpDesc,
}

// Port of BlockCnt from lparser.c
#[derive(Clone, Default)]
pub struct BlockCnt {
    pub previous: Option<BlockCntId>, // link to the enclosing block
    pub first_label: usize,           // index of first label in this block
    pub first_goto: usize,            // index of first pending goto in this block
    pub nactvar: u8,                  // number of active variables outside the block
    pub upval: bool,                  // true if some variable in block is an upvalue
    pub is_loop: bool,                // true if 'block' is a loop
    pub in_scope: bool,               // true if 'block' is still in scope
}

// Port of LabelDesc from lparser.c
#[derive(Clone)]
pub struct LabelDesc {
    pub name: String,
    pub pc: usize,
    pub line: usize,
    pub nactvar: u8,
    pub stklevel: u8, // NEW: saved stack level at goto/label creation
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
    VDKREG = 0,     // regular local variable
    RDKCONST = 1,   // local constant (read-only) <const>
    RDKVAVAR = 2,   // vararg parameter
    RDKTOCLOSE = 3, // to-be-closed variable <close>
    RDKCTC = 4,     // local compile-time constant
    GDKREG = 5,     // regular global variable
    GDKCONST = 6,   // global constant
}

impl VarKind {
    // Test for variables that live in registers
    pub fn is_in_reg(self) -> bool {
        matches!(
            self,
            VarKind::VDKREG | VarKind::RDKCONST | VarKind::RDKVAVAR | VarKind::RDKTOCLOSE
        )
    }

    // Test for global variables
    pub fn is_global(self) -> bool {
        matches!(self, VarKind::GDKREG | VarKind::GDKCONST)
    }

    // Test for readonly variables (const or for-loop control variable)
    pub fn is_readonly(self) -> bool {
        matches!(self, VarKind::RDKCONST | VarKind::GDKCONST)
    }
}

pub struct VarDesc {
    pub name: String,
    pub kind: VarKind,                 // variable kind
    pub ridx: i16,                     // register holding the variable
    pub vidx: u16,                     // compiler index
    pub const_value: Option<LuaValue>, // constant value for compile-time constants
}

impl<'a> FuncState<'a> {
    pub fn new(
        lexer: &'a mut LuaLexer<'a>,
        pool: &'a mut ObjectPool,
        compiler_state: &'a mut CompilerState,
        is_vararg: bool,
        source_name: String,
    ) -> Self {
        // Create kcache table for constant deduplication (like Lua 5.5's open_func)
        let kcache = pool.create_table(0, 0);

        FuncState {
            chunk: Chunk::new(),
            prev: None,
            lexer,
            pool,
            compiler_state,
            block_cnt_id: None,
            pc: 0,
            last_target: 0,
            pending_gotos: Vec::new(),
            labels: Vec::new(),
            actvar: Vec::new(),
            nactvar: 0,
            upvalues: Vec::new(),
            nups: 0,
            freereg: 0,
            iwthabs: 0,
            needclose: false,
            is_vararg,
            numparams: 0,
            source_name,
            first_local: 0,
            kcache,
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

    pub fn current_block_cnt(&mut self) -> Option<&mut BlockCnt> {
        if let Some(bl_id) = &self.block_cnt_id {
            self.compiler_state.get_blockcnt_mut(*bl_id)
        } else {
            None
        }
    }

    pub fn take_block_cnt(&mut self) -> Option<BlockCnt> {
        if let Some(bl_id) = self.block_cnt_id.take() {
            self.compiler_state.take_blockcnt(bl_id)
        } else {
            None
        }
    }

    // Create child function state
    pub fn new_child(parent: &'a mut FuncState<'a>, is_vararg: bool) -> Self {
        // Create new kcache table for child function
        let kcache = parent.pool.create_table(0, 0);

        FuncState {
            chunk: Chunk::new(),
            prev: Some(unsafe { &mut *(parent as *mut FuncState<'a>) }),
            lexer: parent.lexer,
            pool: parent.pool,
            compiler_state: parent.compiler_state,
            block_cnt_id: None,
            pc: 0,
            last_target: 0,
            pending_gotos: Vec::new(),
            labels: Vec::new(),
            actvar: Vec::new(),
            nactvar: 0,
            upvalues: Vec::new(),
            nups: 0,
            freereg: 0,
            iwthabs: 0,
            needclose: false,
            is_vararg,
            numparams: 0,
            first_local: parent.actvar.len(),
            source_name: parent.source_name.clone(),
            kcache,
        }
    }

    // Port of new_localvar from lparser.c
    pub fn new_localvar(&mut self, name: String, kind: VarKind) -> u16 {
        let vidx = self.actvar.len() as u16;
        // For global variables and compile-time constants, ridx doesn't matter
        // as they don't occupy stack registers. Set to -1 as a marker.
        let ridx = if kind.is_global() || kind == VarKind::RDKCTC {
            -1
        } else {
            self.freereg as i16
        };
        self.actvar.push(VarDesc {
            name,
            kind,
            ridx,
            vidx,
            const_value: None, // Initially no const value
        });
        vidx
    }

    // Get variable descriptor
    pub fn get_local_var_desc(&mut self, vidx: u16) -> Option<&mut VarDesc> {
        self.actvar.get_mut(vidx as usize)
    }

    // Port of adjustlocalvars from lparser.c:329-338
    pub fn adjust_local_vars(&mut self, nvars: u8) {
        // Variables have already been added to actvar by new_localvar
        // This function assigns register indices to them and marks them as active
        let mut reglevel = self.reglevel(self.nactvar);
        for _ in 0..nvars {
            let vidx = self.nactvar;
            if let Some(var) = self.actvar.get_mut(vidx as usize) {
                var.ridx = reglevel as i16;
                reglevel += 1;
                // Add variable name to chunk's locals for debugging
                self.chunk.locals.push(var.name.clone());
            }
            self.nactvar += 1;
        }
    }

    // Port of reglevel from lparser.c:236-242
    // Returns the register level for variables outside the block
    // Matches Lua 5.5's: while (nvar-- > 0) { if (varinreg(vd)) return vd->ridx + 1; }
    pub fn reglevel(&self, nvar: u8) -> u8 {
        let mut n = nvar as i32 - 1;
        while n >= 0 {
            if let Some(vd) = self.actvar.get(n as usize) {
                // Use is_in_reg() which matches varinreg(v) macro: (v->vd.kind <= RDKTOCLOSE)
                if vd.kind.is_in_reg() {
                    return (vd.ridx + 1) as u8;
                }
            }
            n -= 1;
        }
        0 // no variables in registers
    }

    // Port of luaY_nvarstack from lparser.c:332-334
    // Returns the number of registers used by active variables
    pub fn nvarstack(&self) -> u8 {
        self.reglevel(self.nactvar)
    }

    // Port of removevars from lparser.c
    pub fn remove_vars(&mut self, tolevel: u8) {
        while self.nactvar > tolevel {
            self.nactvar -= 1;
        }
        // Truncate actvar to remove local variables
        // Note: This is different from Lua 5.5 which keeps removed variables in dyd->actvar
        // but we truncate here for memory management
        self.actvar.truncate(self.nactvar as usize);
    }

    // Port of searchvar from lparser.c (lines 414-443)
    pub fn searchvar(&self, name: &str, var: &mut ExpDesc) -> i32 {
        for i in (0..self.nactvar as usize).rev() {
            if let Some(vd) = self.actvar.get(i) {
                // Check for global declaration (lparser.c:419)
                // Note: Lua 5.5 checks global declarations in all scopes,
                // the base parameter is used for other purposes
                if vd.kind.is_global() {
                    // lparser.c:420-421: collective declaration?
                    if vd.name.is_empty() {
                        // lparser.c:421-422: no previous collective declaration?
                        if var.u.info() < 0 {
                            // This is the first one - record its position
                            var.u = ExpUnion::Info((self.first_local + i) as i32);
                        }
                    } else {
                        // lparser.c:424: global name
                        if vd.name == name {
                            // lparser.c:425-426: found!
                            *var = ExpDesc::new_void();
                            var.kind = ExpKind::VGLOBAL;
                            var.u = ExpUnion::Info((self.first_local + i) as i32);
                            return ExpKind::VGLOBAL as i32;
                        } else if var.u.info() == -1 {
                            // lparser.c:428-429: active preambular declaration?
                            // Invalidate preambular declaration
                            var.u = ExpUnion::Info(-2);
                        }
                    }
                } else if vd.name == name {
                    // lparser.c:432: local variable found
                    if vd.kind == VarKind::RDKCTC {
                        // lparser.c:433: compile-time constant
                        *var = ExpDesc::new_void();
                        var.kind = ExpKind::VCONST;
                        var.u = ExpUnion::Info((self.first_local + i) as i32);
                        return ExpKind::VCONST as i32;
                    } else {
                        // lparser.c:435-439: regular local variable
                        let ridx = vd.ridx as u8;
                        *var = ExpDesc::new_local(ridx, i as u16);
                        // lparser.c:437-438: vararg parameter?
                        if vd.kind == VarKind::RDKVAVAR {
                            var.kind = ExpKind::VVARGVAR;
                        }
                        return var.kind as i32;
                    }
                }
            }
        }
        -1 // lparser.c:442: not found
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
    pub fn newupvalue(&mut self, name: &str, v: &ExpDesc) -> i32 {
        let prev_ptr = match &self.prev {
            Some(p) => *p as *const _ as *mut FuncState,
            None => std::ptr::null_mut(),
        };

        let (in_stack, idx, kind) = unsafe {
            if v.kind == ExpKind::VLOCAL || v.kind == ExpKind::VVARGVAR {
                // lparser.c:366-370: local or vararg parameter upvalue
                let vidx = v.u.var().vidx;
                let ridx = v.u.var().ridx;
                
                // Mark the variable in parent function as needing upvalue closure
                if !prev_ptr.is_null() {
                    let prev = &mut *prev_ptr;
                    crate::compiler::statement::mark_upval(prev, vidx as u8);
                }
                
                let prev = &*prev_ptr;
                let vd = &prev.actvar[vidx as usize];
                (true, ridx as u8, vd.kind)
            } else {
                // lparser.c:371-375: upvalue from outer function
                let info = v.u.info() as usize;
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
