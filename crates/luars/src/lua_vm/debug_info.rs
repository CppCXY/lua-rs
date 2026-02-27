// DebugInfo - Rust equivalent of C Lua's lua_Debug struct
// Contains all debug information about a function or stack frame.
// This is the core data structure; debug.getinfo merely wraps it into a Lua table.

use crate::compiler::format_source;
use crate::lua_value::LuaValue;

/// Debug information about a function or stack frame.
/// Mirrors C Lua's `lua_Debug` struct from lua.h.
///
/// Each field is tagged with the 'what' option character that populates it:
/// - `'S'` — source info (source, short_src, linedefined, lastlinedefined, what)
/// - `'l'` — current line (currentline)
/// - `'u'` — upvalue info (nups, nparams, isvararg)
/// - `'n'` — name info (name, namewhat)
/// - `'t'` — tail/extra info (istailcall, extraargs)
/// - `'r'` — transfer info (ftransfer, ntransfer)
/// - `'L'` — active lines (activelines)
/// - `'f'` — the function itself (func)
#[derive(Debug, Clone, Default)]
pub struct DebugInfo {
    // 'S' fields
    /// Full source name (e.g. "@test.lua", "=stdin", or the code string)
    pub source: Option<String>,
    /// Formatted short source for display (e.g. "test.lua", "[string \"...\"]")
    pub short_src: Option<String>,
    /// Line where the function definition starts (0 for main chunk, -1 for C)
    pub linedefined: Option<i32>,
    /// Line where the function definition ends (-1 for C)
    pub lastlinedefined: Option<i32>,
    /// Function type: "Lua", "C", "main", or "tail"
    pub what: Option<&'static str>,

    // 'l' field
    /// Current line being executed (-1 if not available)
    pub currentline: Option<i32>,

    // 'u' fields
    /// Number of upvalues
    pub nups: Option<u8>,
    /// Number of fixed parameters
    pub nparams: Option<u8>,
    /// Whether the function is vararg
    pub isvararg: Option<bool>,

    // 'n' fields
    /// Function name (if found from calling context)
    pub name: Option<String>,
    /// How the name was resolved: "global", "local", "method", "field", "upvalue",
    /// "metamethod", "for iterator", or "" if not found
    pub namewhat: Option<String>,

    // 't' fields
    /// Whether this is a tail call
    pub istailcall: Option<bool>,
    /// Number of extra arguments from __call metamethods
    pub extraargs: Option<u8>,

    // 'r' fields
    /// Index of first value being transferred (for hooks)
    pub ftransfer: Option<i32>,
    /// Number of values being transferred (for hooks)
    pub ntransfer: Option<i32>,

    // 'L' field
    /// Active lines: list of line numbers that have associated bytecode
    pub activelines: Option<Vec<i32>>,

    // 'f' field
    /// The function value itself
    pub func: Option<LuaValue>,
}

impl DebugInfo {
    /// Create a new empty DebugInfo
    pub fn new() -> Self {
        Self::default()
    }

    /// Fill source-related fields ('S') from a Lua chunk
    pub(crate) fn fill_source(
        &mut self,
        source_name: Option<&str>,
        linedefined: i32,
        lastlinedefined: i32,
    ) {
        let source = source_name.unwrap_or("=?");
        self.source = Some(source.to_string());
        self.short_src = Some(format_source(source));
        self.linedefined = Some(linedefined);
        self.lastlinedefined = Some(lastlinedefined);
        self.what = Some(if linedefined == 0 { "main" } else { "Lua" });
    }

    /// Fill source-related fields ('S') for a C function
    pub(crate) fn fill_source_c(&mut self) {
        self.source = Some("=[C]".to_string());
        self.short_src = Some("[C]".to_string());
        self.linedefined = Some(-1);
        self.lastlinedefined = Some(-1);
        self.what = Some("C");
    }

    /// Fill current line ('l')
    pub(crate) fn fill_currentline(&mut self, currentline: i32) {
        self.currentline = Some(currentline);
    }

    /// Fill upvalue info ('u') for Lua function
    pub(crate) fn fill_upvalues(&mut self, nups: u8, nparams: u8, isvararg: bool) {
        self.nups = Some(nups);
        self.nparams = Some(nparams);
        self.isvararg = Some(isvararg);
    }

    /// Fill upvalue info ('u') for C function
    pub(crate) fn fill_upvalues_c(&mut self, nups: u8) {
        self.nups = Some(nups);
        self.nparams = Some(0);
        self.isvararg = Some(true);
    }

    /// Fill name info ('n')
    pub(crate) fn fill_name(&mut self, namewhat: &str, name: &str) {
        self.namewhat = Some(namewhat.to_string());
        self.name = Some(name.to_string());
    }

    /// Fill name info ('n') when no name found
    pub(crate) fn fill_name_empty(&mut self) {
        self.namewhat = Some(String::new());
        self.name = None;
    }

    /// Fill tail call info ('t')
    pub(crate) fn fill_tail(&mut self, istailcall: bool, extraargs: u8) {
        self.istailcall = Some(istailcall);
        self.extraargs = Some(extraargs);
    }

    /// Fill transfer info ('r')
    pub(crate) fn fill_transfer(&mut self, ftransfer: i32, ntransfer: i32) {
        self.ftransfer = Some(ftransfer);
        self.ntransfer = Some(ntransfer);
    }

    /// Fill active lines ('L') from line_info (absolute line numbers per instruction)
    pub(crate) fn fill_activelines(&mut self, line_info: &[u32], is_vararg: bool) {
        let mut lines = Vec::new();
        let start = if is_vararg { 1 } else { 0 }; // skip VARARGPREP for vararg functions
        for i in start..line_info.len() {
            let line = line_info[i] as i32;
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
        self.activelines = Some(lines);
    }

    /// Fill active lines as nil (for C functions)
    pub(crate) fn fill_activelines_nil(&mut self) {
        self.activelines = None;
    }

    /// Fill function value ('f')
    pub(crate) fn fill_func(&mut self, func: LuaValue) {
        self.func = Some(func);
    }
}
