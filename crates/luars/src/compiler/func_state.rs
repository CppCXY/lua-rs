use crate::compiler::{ExpDesc, ExpKind, ExpUnion, statement};
use crate::lua_vm::lua_limits::{MAX_SRC_LEN, MAXCCALLS, MAXUPVAL, MAXVARS};
use crate::{LuaProto, LuaVM};
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
    pub chunk: LuaProto,
    /// Parent function state. Stored as raw pointer to break the self-referential
    /// linked-list cycle: a child FuncState is created on the stack inside `body()`
    /// while the parent is alive. Rust's borrow checker cannot express this pattern
    /// with safe references. Access via `parent()` / `parent_mut()` — those methods
    /// encapsulate the single unsafe dereference.
    prev: Option<*mut FuncState<'a>>,
    pub lexer: &'a mut LuaLexer<'a>,
    pub vm: &'a mut LuaVM,
    pub compiler_state: &'a mut CompilerState,
    pub block_cnt_id: Option<BlockCntId>,
    pub pc: usize,                     // next position to code (equivalent to pc)
    pub last_target: usize,            // label of last 'jump label'
    pub pending_gotos: Vec<LabelDesc>, // list of pending gotos
    pub labels: Vec<LabelDesc>,        // list of active labels
    pub actvar: Vec<VarDesc>,          // list of all variable descriptors (active and pending)
    pub nactvar: u16,                  // number of active variables (actvar[0..nactvar] are active)
    pub upvalues: Vec<Upvaldesc>,      // upvalue descriptors
    pub nups: u8,                      // number of upvalues
    pub freereg: u8,                   // first free register
    #[allow(unused)]
    pub iwthabs: u8, // instructions issued since last absolute line info
    pub needclose: bool,               // true if function needs to close upvalues when returning
    pub is_vararg: bool,               // true if function is vararg
    pub numparams: u8,                 // number of fixed parameters (excluding vararg parameter)
    pub first_local: usize,            // index of first local variable in prev
    pub source_name: String,           // source file name for error messages
    pub kcache: LuaValue, // cache table for constant deduplication (per-function, like Lua 5.5's fs->kcache)
    pub checklimit_error: Option<String>, // deferred error from checkstack/checklimit
}

pub struct CompilerState {
    // pool of BlockCnt structures (Option to allow safe take without invalidating indices)
    pub block_cnt_pool: Vec<Option<BlockCnt>>,
    // pool of LhsAssign structures for assignment chain management
    pub lhs_assign_pool: Vec<Option<LhsAssign>>,
}

impl Default for CompilerState {
    fn default() -> Self {
        Self::new()
    }
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
    pub nactvar: u16,                 // number of active variables outside the block
    pub upval: bool,                  // true if some variable in block is an upvalue
    pub is_loop: u8,                  // 0: not a loop; 1: loop; 2: loop with pending breaks
    pub in_scope: bool,               // true if 'block' is still in scope
}

// Port of LabelDesc from lparser.c
#[derive(Clone)]
pub struct LabelDesc {
    pub name: String,
    pub pc: usize,
    pub line: usize,
    pub nactvar: u16,
    pub stklevel: u8, // NEW: saved stack level at goto/label creation
    pub close: bool,
}

// Port of Vardesc from lparser.c
// Variable kinds
#[allow(clippy::upper_case_acronyms)]
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

    // Test for readonly variables (const, vararg parameter, or for-loop control variable)
    // In Lua 5.5: vkisreadonly(v) => v->kind >= RDKCONST
    #[allow(unused)]
    pub fn is_readonly(self) -> bool {
        matches!(
            self,
            VarKind::RDKCONST | VarKind::RDKVAVAR | VarKind::GDKCONST
        )
    }
}

pub struct VarDesc {
    pub name: String,
    pub kind: VarKind, // variable kind
    pub ridx: i16,     // register holding the variable
    #[allow(unused)]
    pub vidx: u16, // compiler index
    pub pidx: usize,   // index into chunk.locals (LocVar debug info)
    pub const_value: Option<LuaValue>, // constant value for compile-time constants
}

impl<'a> FuncState<'a> {
    pub fn new(
        lexer: &'a mut LuaLexer<'a>,
        vm: &'a mut LuaVM,
        compiler_state: &'a mut CompilerState,
        is_vararg: bool,
        source_name: String,
    ) -> Self {
        // Create kcache table for constant deduplication (like Lua 5.5's open_func)
        let kcache = vm.create_table(0, 0).unwrap();
        FuncState {
            chunk: LuaProto::new(),
            prev: None,
            lexer,
            vm,
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
            checklimit_error: None,
        }
    }

    // Unified error generation function (port of luaX_syntaxerror from llex.c)
    // Always adds "near <token>" like C Lua's luaX_syntaxerror
    pub fn syntax_error(&self, msg: &str) -> String {
        self.token_error(msg)
    }

    // Semantic error - uses lastline (port of luaK_semerror from lcode.c)
    // Reports error at the line of the last consumed token, not the current lookahead
    pub fn sem_error(&self, msg: &str) -> String {
        let line = self.lexer.lastline;
        format!("{}:{}: {}", format_source(&self.source_name), line, msg)
    }

    // Port of errorlimit from lparser.c:73-84
    // Generates "too many <what> (limit is <limit>) in <where>" error
    pub fn errorlimit(&self, limit: usize, what: &str) -> String {
        let linedefined = self.chunk.linedefined;
        let where_str = if linedefined == 0 {
            "main function".to_string()
        } else {
            format!("function at line {}", linedefined)
        };
        let msg = format!("too many {} (limit is {}) in {}", what, limit, where_str);
        self.syntax_error(&msg)
    }

    // Check and consume any pending checklimit error
    pub fn check_pending_checklimit(&mut self) -> Result<(), String> {
        if let Some(err) = self.checklimit_error.take() {
            Err(err)
        } else {
            Ok(())
        }
    }

    // Port of enterlevel/leavelevel from lparser.c
    // Tracks recursion depth during parsing; MAXCCALLS = 200
    pub fn enter_level(&mut self) -> Result<(), String> {
        self.lexer.nesting_level += 1;
        if self.lexer.nesting_level >= MAXCCALLS {
            return Err(self.syntax_error("chunk has too many syntax levels"));
        }
        Ok(())
    }

    pub fn leave_level(&mut self) {
        self.lexer.nesting_level -= 1;
    }

    // Generate error with current token information
    pub fn token_error(&self, msg: &str) -> String {
        let token_text = self.lexer.current_token_text();
        let line = self.lexer.line;
        let source = format_source(&self.source_name);
        // Special handling for <eof> - don't add quotes around it
        if token_text == "<eof>" {
            format!("{}:{}: {} near {}", source, line, msg, token_text)
        } else {
            // Check if the token is a single non-printable character
            // (including bytes 128-255 mapped from Latin-1)
            let chars: Vec<char> = token_text.chars().collect();
            if chars.len() == 1 {
                let c = chars[0];
                let code = c as u32;
                if code <= 255 && !(c.is_ascii_graphic() || c == ' ') {
                    // Non-printable byte — show as <\N>
                    return format!("{}:{}: {} near '<\\{}>'", source, line, msg, code);
                }
            }
            format!("{}:{}: {} near '{}'", source, line, msg, token_text)
        }
    }

    pub fn current_block_cnt(&mut self) -> Option<&mut BlockCnt> {
        if let Some(bl_id) = &self.block_cnt_id {
            self.compiler_state.get_blockcnt_mut(*bl_id)
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn take_block_cnt(&mut self) -> Option<BlockCnt> {
        if let Some(bl_id) = self.block_cnt_id.take() {
            self.compiler_state.take_blockcnt(bl_id)
        } else {
            None
        }
    }

    /// Safe accessor: get an immutable reference to the parent FuncState.
    pub fn parent(&self) -> Option<&FuncState<'a>> {
        // SAFETY: `prev` is set once in `new_child` from a `&'a mut` reference
        // that outlives the current FuncState. The function stack is torn down
        // strictly bottom-up (children finished before parents continue), so
        // this pointer is always valid when called.
        self.prev.map(|p| unsafe { &*p })
    }

    /// Safe accessor: get a mutable reference to the parent FuncState.
    pub fn parent_mut(&mut self) -> Option<&mut FuncState<'a>> {
        // SAFETY: same invariant as `parent()`. The mutable alias is exclusive
        // because no other code holds a `&mut` to the parent while we do.
        self.prev.map(|p| unsafe { &mut *p })
    }

    // Create child function state
    // SAFETY: `parent` must be a valid `&mut FuncState` that outlives the child.
    // The child stores `parent` as a raw pointer (to break the self-referential
    // linked-list cycle) and borrows `lexer`/`vm`/`compiler_state` from it.
    pub fn new_child(parent: *mut FuncState<'a>, is_vararg: bool) -> Self {
        // SAFETY: caller guarantees `parent` is a valid &mut FuncState that
        // outlives the child. The function body creates the child on its stack
        // (strictly nested), so the parent lives until after the child is dropped.
        let parent_ref = unsafe { &mut *parent };
        let kcache = parent_ref.vm.create_table(0, 0).unwrap();
        FuncState {
            chunk: LuaProto::new(),
            prev: Some(parent),
            lexer: parent_ref.lexer,
            vm: parent_ref.vm,
            compiler_state: parent_ref.compiler_state,
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
            first_local: parent_ref.actvar.len(),
            source_name: parent_ref.source_name.clone(),
            kcache,
            checklimit_error: None,
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
            pidx: 0,           // Will be set in adjust_local_vars
            const_value: None, // Initially no const value
        });
        vidx
    }

    // Get variable descriptor
    pub fn get_local_var_desc(&mut self, vidx: u16) -> Option<&mut VarDesc> {
        self.actvar.get_mut(vidx as usize)
    }

    // Port of adjustlocalvars from lparser.c:329-338
    pub fn adjust_local_vars(&mut self, nvars: u16) -> Result<(), String> {
        // Variables have already been added to actvar by new_localvar.
        // This function assigns register indices to them and marks them as active.
        let mut reglevel = self.reglevel(self.nactvar);
        let startpc = self.chunk.code.len() as u32;
        for _ in 0..nvars {
            let vidx = self.nactvar;
            if let Some(var) = self.actvar.get_mut(vidx as usize) {
                var.ridx = reglevel as i16;
                reglevel += 1;
                // Check limit: MAXVARS
                if reglevel as usize > MAXVARS {
                    return Err(self.errorlimit(MAXVARS, "local variables"));
                }
                // Add variable debug info (LocVar) to chunk
                let pidx = self.chunk.locals.len();
                var.pidx = pidx;
                self.chunk.locals.push(crate::lua_value::LocVar {
                    name: var.name.clone(),
                    startpc,
                    endpc: 0, // Will be set in remove_vars
                });
            }
            self.nactvar += 1;
        }
        Ok(())
    }

    // Port of reglevel from lparser.c:236-242
    // Returns the register level for variables outside the block
    // Matches Lua 5.5's: while (nvar-- > 0) { if (varinreg(vd)) return vd->ridx + 1; }
    pub fn reglevel(&self, nvar: u16) -> u8 {
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
    pub fn remove_vars(&mut self, tolevel: u16) {
        let endpc = self.chunk.code.len() as u32;
        while self.nactvar > tolevel {
            self.nactvar -= 1;
            // Set endpc for the LocVar debug entry
            if let Some(var) = self.actvar.get(self.nactvar as usize) {
                let pidx = var.pidx;
                if pidx < self.chunk.locals.len() {
                    self.chunk.locals[pidx].endpc = endpc;
                }
            }
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
                        // Use i (relative index) for per-function actvar array
                        var.u = ExpUnion::Info(i as i32);
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
        // Port of luaY_checklimit(fs, fs->nups + 1, MAXUPVAL, "upvalues")
        if self.nups as usize + 1 > MAXUPVAL && self.checklimit_error.is_none() {
            self.checklimit_error = Some(self.errorlimit(MAXUPVAL, "upvalues"));
        }

        let (in_stack, idx, kind) = {
            if v.kind == ExpKind::VLOCAL || v.kind == ExpKind::VVARGVAR {
                // lparser.c:366-370: local or vararg parameter upvalue
                let vidx = v.u.var().vidx;
                let ridx = v.u.var().ridx;

                let parent = self.parent_mut().expect("parent expected for upvalue");
                statement::mark_upval(parent, vidx);
                let vd = &parent.actvar[vidx as usize];
                (true, ridx as u8, vd.kind)
            } else {
                // lparser.c:371-375: upvalue from outer function
                let info = v.u.info() as usize;
                let parent = self.parent().expect("parent expected for upvalue");
                let up = &parent.upvalues[info];
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
        // Cap nups at MAXUPVAL to avoid u8 overflow in debug builds.
        // The checklimit_error above already recorded the "too many upvalues"
        // error; compilation continues (deferred error, matching C Lua).
        self.nups = self.upvalues.len().min(MAXUPVAL) as u8;

        (self.upvalues.len() - 1) as i32
    }
}

/// Format source name for error messages (port of luaO_chunkid from lobject.c)
/// - "@filename" → "filename" (strip @ prefix, truncate if needed)
/// - "=display" → "display" (strip = prefix, use as-is)
/// - other → [string "first_line..."] (wrap in [string "..."], truncate)
pub fn format_source(source: &str) -> String {
    // LUA_IDSIZE = 60 in luaconf.h, but includes null terminator, so max usable = 59
    const MAX_SRC: usize = MAX_SRC_LEN;
    if let Some(name) = source.strip_prefix('=') {
        // Remove the '=' prefix, take as display name, truncate to MAX_SRC
        if name.len() <= MAX_SRC {
            name.to_string()
        } else {
            name[..MAX_SRC].to_string()
        }
    } else if let Some(name) = source.strip_prefix('@') {
        if name.len() <= MAX_SRC {
            name.to_string()
        } else {
            // Truncate from the end, add "..."
            format!("...{}", &name[name.len() - (MAX_SRC - 3)..])
        }
    } else {
        // Source is a literal string - wrap as [string "..."]
        // Take first line only, truncate if needed
        let first_line = source.lines().next().unwrap_or(source);
        let max_content = MAX_SRC - "[string \"...\"]".len();
        if first_line.len() <= max_content && source.lines().count() <= 1 {
            format!("[string \"{}\"]", first_line)
        } else {
            let truncated = if first_line.len() > max_content {
                &first_line[..max_content]
            } else {
                first_line
            };
            format!("[string \"{}...\"]", truncated)
        }
    }
}
