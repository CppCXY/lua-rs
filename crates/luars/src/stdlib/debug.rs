// Debug library implementation
// Implements: traceback, getinfo, getlocal, getmetatable, getupvalue, etc.

use crate::Instruction;
use crate::lib_registry::LibraryModule;
use crate::lua_value::{Chunk, LuaValue};
use crate::lua_vm::opcode::OpCode;
use crate::lua_vm::{LuaError, LuaResult, LuaState, get_metatable};

/// Get the type name of an object, checking __name in metatable first.
/// Mirrors C Lua's luaT_objtypename.
pub fn objtypename(l: &mut LuaState, v: &LuaValue) -> String {
    // Special case: nil is "no value" (matches C Lua's luaT_objtypename behavior)
    if v.is_nil() {
        return "no value".to_string();
    }
    if let Some(mt) = get_metatable(l, v) {
        if let Some(mt_table) = mt.as_table() {
            // Create a string key for __name lookup
            if let Ok(key) = l.create_string("__name") {
                if let Some(name_val) = mt_table.raw_get(&key) {
                    if let Some(s) = name_val.as_str() {
                        return s.to_string();
                    }
                }
            }
        }
    }
    v.type_name().to_string()
}

// ============================================================================
// Function name resolution (mirrors Lua 5.5 ldebug.c)
// ============================================================================

/// Get the name of the Nth active local variable at the given PC.
/// Mirrors Lua 5.5's luaF_getlocalname.
/// local_number is 1-based.
fn getlocalname(chunk: &Chunk, local_number: usize, pc: usize) -> Option<&str> {
    let mut n = local_number;
    for locvar in &chunk.locals {
        if (locvar.startpc as usize) > pc {
            break;
        }
        if pc < locvar.endpc as usize {
            n -= 1;
            if n == 0 {
                return Some(&locvar.name);
            }
        }
    }
    None
}

/// Whether the opcode writes to register A (testAMode)
fn test_a_mode(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Move
            | OpCode::LoadI
            | OpCode::LoadF
            | OpCode::LoadK
            | OpCode::LoadKX
            | OpCode::LoadFalse
            | OpCode::LFalseSkip
            | OpCode::LoadTrue
            | OpCode::LoadNil
            | OpCode::GetUpval
            | OpCode::GetTabUp
            | OpCode::GetTable
            | OpCode::GetI
            | OpCode::GetField
            | OpCode::NewTable
            | OpCode::Self_
            | OpCode::AddI
            | OpCode::AddK
            | OpCode::SubK
            | OpCode::MulK
            | OpCode::ModK
            | OpCode::PowK
            | OpCode::DivK
            | OpCode::IDivK
            | OpCode::BAndK
            | OpCode::BOrK
            | OpCode::BXorK
            | OpCode::ShlI
            | OpCode::ShrI
            | OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::Div
            | OpCode::IDiv
            | OpCode::BAnd
            | OpCode::BOr
            | OpCode::BXor
            | OpCode::Shl
            | OpCode::Shr
            | OpCode::Unm
            | OpCode::BNot
            | OpCode::Not
            | OpCode::Len
            | OpCode::Concat
            | OpCode::TestSet
            | OpCode::Call
            | OpCode::TailCall
            | OpCode::ForLoop
            | OpCode::ForPrep
            | OpCode::TForLoop
            | OpCode::Closure
            | OpCode::Vararg
            | OpCode::GetVarg
            | OpCode::VarargPrep
    )
}

/// Whether the opcode is a metamethod instruction (OP_MMBIN*)
fn test_mm_mode(op: OpCode) -> bool {
    matches!(op, OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK)
}

/// Get the upvalue name from chunk
fn upvalname(chunk: &Chunk, uv: usize) -> String {
    if uv < chunk.upvalue_descs.len() {
        chunk.upvalue_descs[uv].name.clone()
    } else {
        "?".to_string()
    }
}

/// Get a constant name (if it's a string)
fn kname(chunk: &Chunk, index: usize) -> Option<String> {
    if index < chunk.constants.len() {
        if let Some(s) = chunk.constants[index].as_str() {
            return Some(s.to_string());
        }
    }
    None
}

/// Find the last instruction before lastpc that sets register reg.
/// Returns -1 if not found.
/// Mirrors Lua 5.5's findsetreg.
fn findsetreg(chunk: &Chunk, lastpc: usize, reg: u32) -> i32 {
    let mut setreg: i32 = -1;
    let mut jmptarget: usize = 0;

    // If the instruction at lastpc is an MM-mode instruction, back up one
    let lastpc = if lastpc < chunk.code.len() && test_mm_mode(chunk.code[lastpc].get_opcode()) {
        lastpc.saturating_sub(1)
    } else {
        lastpc
    };

    for pc in 0..lastpc {
        let i = chunk.code[pc];
        let op = i.get_opcode();
        let a = i.get_a();

        let change = match op {
            OpCode::LoadNil => reg >= a && reg <= a + i.get_b(),
            OpCode::TForCall => reg >= a + 2,
            OpCode::Call | OpCode::TailCall => reg >= a,
            OpCode::Jmp => {
                let b = i.get_sj();
                let dest = (pc as i32 + 1 + b) as usize;
                if dest <= lastpc && dest > jmptarget {
                    jmptarget = dest;
                }
                false
            }
            _ => test_a_mode(op) && reg == a,
        };

        if change {
            // filterpc: if inside a jump target region, discard
            setreg = if pc < jmptarget { -1 } else { pc as i32 };
        }
    }
    setreg
}

/// Basic object name resolution.
/// Returns (kind, name) or None.
/// Mirrors Lua 5.5's basicgetobjname.
fn basicgetobjname(chunk: &Chunk, pc: &mut i32, reg: u32) -> Option<(&'static str, String)> {
    let pc_val = *pc as usize;

    // First try: is reg a local variable at this PC?
    if let Some(name) = getlocalname(chunk, (reg + 1) as usize, pc_val) {
        return Some(("local", name.to_string()));
    }

    // Symbolic execution: find the instruction that set this register
    let setreg_pc = findsetreg(chunk, pc_val, reg);
    *pc = setreg_pc;

    if setreg_pc >= 0 {
        let i = chunk.code[setreg_pc as usize];
        let op = i.get_opcode();

        match op {
            OpCode::Move => {
                let b = i.get_b();
                if b < i.get_a() {
                    return basicgetobjname(chunk, pc, b);
                }
            }
            OpCode::GetUpval => {
                let b = i.get_b() as usize;
                let name = upvalname(chunk, b);
                return Some(("upvalue", name));
            }
            OpCode::LoadK => {
                let bx = i.get_bx() as usize;
                if let Some(name) = kname(chunk, bx) {
                    return Some(("constant", name));
                }
            }
            OpCode::LoadKX => {
                if (setreg_pc as usize + 1) < chunk.code.len() {
                    let ax = chunk.code[setreg_pc as usize + 1].get_ax() as usize;
                    if let Some(name) = kname(chunk, ax) {
                        return Some(("constant", name));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Get a register name for rname helper
fn rname(chunk: &Chunk, pc: usize, c: u32) -> String {
    let mut ppc = pc as i32;
    if let Some((kind, name)) = basicgetobjname(chunk, &mut ppc, c) {
        if kind == "constant" {
            return name;
        }
    }
    "?".to_string()
}

/// Check if the table operand names _ENV (making it a "global")
fn is_env(chunk: &Chunk, pc: usize, i: Instruction, isup: bool) -> &'static str {
    let t = i.get_b();
    let name = if isup {
        Some(upvalname(chunk, t as usize))
    } else {
        let mut ppc = pc as i32;
        match basicgetobjname(chunk, &mut ppc, t) {
            Some(("local", name)) | Some(("upvalue", name)) => Some(name),
            _ => None,
        }
    };
    match name {
        Some(ref n) if n == "_ENV" => "global",
        _ => "field",
    }
}

/// Extended object name resolution (handles table accesses).
/// Mirrors Lua 5.5's getobjname.
fn getobjname(chunk: &Chunk, lastpc: usize, reg: u32) -> Option<(&'static str, String)> {
    let mut pc = lastpc as i32;
    if let Some(result) = basicgetobjname(chunk, &mut pc, reg) {
        return Some(result);
    }
    if pc >= 0 {
        let i = chunk.code[pc as usize];
        match i.get_opcode() {
            OpCode::GetTabUp => {
                let k = i.get_c() as usize;
                let name = kname(chunk, k).unwrap_or_else(|| "?".to_string());
                let kind = is_env(chunk, pc as usize, i, true);
                return Some((kind, name));
            }
            OpCode::GetTable => {
                let k = i.get_c();
                let name = rname(chunk, pc as usize, k);
                let kind = is_env(chunk, pc as usize, i, false);
                return Some((kind, name));
            }
            OpCode::GetI => {
                return Some(("field", "integer index".to_string()));
            }
            OpCode::GetField => {
                let k = i.get_c() as usize;
                let field_name = kname(chunk, k).unwrap_or_else(|| "?".to_string());
                let kind = is_env(chunk, pc as usize, i, false);
                return Some((kind, field_name));
            }
            OpCode::Self_ => {
                let k = i.get_c() as usize;
                let name = kname(chunk, k).unwrap_or_else(|| "?".to_string());
                return Some(("method", name));
            }
            _ => {}
        }
    }
    None
}

/// Determine function name from bytecode at the calling instruction.
/// Mirrors Lua 5.5's funcnamefromcode.
fn funcnamefromcode(chunk: &Chunk, pc: usize) -> Option<(&'static str, String)> {
    if pc >= chunk.code.len() {
        return None;
    }
    let i = chunk.code[pc];
    match i.get_opcode() {
        OpCode::Call | OpCode::TailCall => getobjname(chunk, pc, i.get_a()),
        OpCode::TForCall => Some(("for iterator", "for iterator".to_string())),
        // Metamethod-triggering instructions
        OpCode::Self_ | OpCode::GetTabUp | OpCode::GetTable | OpCode::GetI | OpCode::GetField => {
            Some(("metamethod", "index".to_string()))
        }
        OpCode::SetTabUp | OpCode::SetTable | OpCode::SetI | OpCode::SetField => {
            Some(("metamethod", "newindex".to_string()))
        }
        OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK => {
            use crate::lua_vm::TmKind;
            if let Some(tm) = TmKind::from_u8(i.get_c() as u8) {
                let name: &str = tm.name();
                Some(("metamethod", name[2..].to_string()))
            } else {
                None
            }
        }
        OpCode::Unm => Some(("metamethod", "unm".to_string())),
        OpCode::BNot => Some(("metamethod", "bnot".to_string())),
        OpCode::Len => Some(("metamethod", "len".to_string())),
        OpCode::Concat => Some(("metamethod", "concat".to_string())),
        OpCode::Eq => Some(("metamethod", "eq".to_string())),
        OpCode::Lt | OpCode::LtI | OpCode::GtI => Some(("metamethod", "lt".to_string())),
        OpCode::Le | OpCode::LeI | OpCode::GeI => Some(("metamethod", "le".to_string())),
        OpCode::Close | OpCode::Return => Some(("metamethod", "close".to_string())),
        _ => None,
    }
}

/// Get function name by looking at the calling frame.
/// Mirrors Lua 5.5's getfuncname.
/// ci_frame_idx is the frame index of the TARGET function.
fn getfuncname(l: &LuaState, ci_frame_idx: usize) -> Option<(&'static str, String)> {
    if ci_frame_idx == 0 {
        return None; // No caller frame
    }
    let ci = l.get_frame(ci_frame_idx)?;
    // If tail call, cannot find name
    if ci.is_tail() {
        return None;
    }
    // Look at the immediately previous frame only (matching C Lua's getfuncname)
    let prev_idx = ci_frame_idx - 1;
    let prev = l.get_frame(prev_idx)?;
    if prev.is_lua() {
        // Get caller's chunk
        let prev_func = l.get_frame_func(prev_idx)?;
        let lua_func = prev_func.as_lua_function()?;
        let chunk = lua_func.chunk();
        // prev.pc points to the instruction AFTER the call (due to pc += 1 in fetch).
        // So the call instruction is at pc - 1.
        let pc = prev.pc.saturating_sub(1) as usize;
        return funcnamefromcode(chunk, pc);
    }
    // Previous frame is C — cannot determine name from bytecode
    None
}

// ============================================================================
// Public API for error message generation (mirrors ldebug.c luaG_typeerror)
// ============================================================================

/// Generate variable info string like " (global 'X')" for error messages.
/// Mirrors Lua 5.5's varinfo() from ldebug.c.
/// Must be called AFTER save_pc so the current frame's PC is up to date.
pub fn varinfo(l: &LuaState) -> String {
    let ci_idx = l.call_depth().wrapping_sub(1);
    let ci = match l.get_frame(ci_idx) {
        Some(ci) => ci,
        None => return String::new(),
    };
    if !ci.is_lua() {
        return String::new();
    }
    let func = match l.get_frame_func(ci_idx) {
        Some(f) => f,
        None => return String::new(),
    };
    let lua_func = match func.as_lua_function() {
        Some(f) => f,
        None => return String::new(),
    };
    let chunk = lua_func.chunk();
    // currentpc: saved pc points AFTER the current instruction (pc += 1 in fetch)
    let currentpc = ci.pc.saturating_sub(1) as usize;

    // Get the instruction at currentpc to determine which register holds the object
    if currentpc >= chunk.code.len() {
        return String::new();
    }
    let instr = chunk.code[currentpc];
    let op = instr.get_opcode();

    // Determine which register to look up based on the opcode
    let reg = match op {
        // GET* instructions: table is in register B
        OpCode::GetTable | OpCode::GetI | OpCode::GetField | OpCode::Self_ => Some(instr.get_b()),
        // SET* instructions: table is in register A
        OpCode::SetTable | OpCode::SetI | OpCode::SetField => Some(instr.get_a()),
        // GETTABUP: table is upvalue B — the upvalue itself is being indexed
        // When this instruction fails, it's because the upvalue is not indexable
        OpCode::GetTabUp => {
            let upval_idx = instr.get_b() as usize;
            if upval_idx < chunk.upvalue_descs.len() {
                let upname = &chunk.upvalue_descs[upval_idx].name;
                if upname == "_ENV" {
                    // For _ENV, report the key as global
                    let c = instr.get_c() as usize;
                    let name = kname(chunk, c).unwrap_or_else(|| "?".to_string());
                    return format!(" (global '{}')", name);
                } else {
                    return format!(" (upvalue '{}')", upname);
                }
            }
            return String::new();
        }
        // SETTABUP: table is upvalue A — the upvalue itself is being indexed
        OpCode::SetTabUp => {
            let upval_idx = instr.get_a() as usize;
            if upval_idx < chunk.upvalue_descs.len() {
                let upname = &chunk.upvalue_descs[upval_idx].name;
                if upname == "_ENV" {
                    // For _ENV, report the key as global
                    let b = instr.get_b() as usize;
                    let name = kname(chunk, b).unwrap_or_else(|| "?".to_string());
                    return format!(" (global '{}')", name);
                } else {
                    return format!(" (upvalue '{}')", upname);
                }
            }
            return String::new();
        }
        // CALL/TAILCALL: function being called is in register A
        OpCode::Call | OpCode::TailCall => Some(instr.get_a()),
        // Unary ops: operand is in register B
        OpCode::Unm | OpCode::BNot | OpCode::Len | OpCode::Not => Some(instr.get_b()),
        // CONCAT: operand is in register A (first concat value)
        OpCode::Concat => Some(instr.get_a()),
        // MmBin: look at previous instruction for the actual arithmetic/comparison op
        OpCode::MmBin => {
            // MmBin is emitted AFTER the arithmetic op (ADD, SUB, etc.)
            // The previous instruction has the operands
            if currentpc > 0 {
                let prev_instr = chunk.code[currentpc - 1];
                let prev_op = prev_instr.get_opcode();
                match prev_op {
                    OpCode::Add
                    | OpCode::Sub
                    | OpCode::Mul
                    | OpCode::Mod
                    | OpCode::Pow
                    | OpCode::Div
                    | OpCode::IDiv
                    | OpCode::BAnd
                    | OpCode::BOr
                    | OpCode::BXor
                    | OpCode::Shl
                    | OpCode::Shr
                    | OpCode::Eq
                    | OpCode::Lt
                    | OpCode::Le => {
                        // Binary ops: first operand in register A (aka sRA)
                        Some(prev_instr.get_a())
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        OpCode::MmBinI => {
            if currentpc > 0 {
                let prev_instr = chunk.code[currentpc - 1];
                Some(prev_instr.get_a())
            } else {
                None
            }
        }
        OpCode::MmBinK => {
            if currentpc > 0 {
                let prev_instr = chunk.code[currentpc - 1];
                Some(prev_instr.get_a())
            } else {
                None
            }
        }
        _ => None,
    };

    if let Some(reg) = reg {
        if let Some((kind, name)) = getobjname(chunk, currentpc, reg) {
            return format!(" ({} '{}')", kind, name);
        }
    }
    String::new()
}

/// Generate a type error with variable info.
/// Mirrors Lua 5.5's luaG_typeerror.
/// `op` is typically "index" for table access errors.
pub fn typeerror(l: &mut LuaState, val: &LuaValue, op: &str) -> LuaError {
    let tname = objtypename(l, val);
    let info = varinfo(l);
    l.error(format!("attempt to {} a {} value{}", op, tname, info))
}

/// Get the name and kind of the current function from the calling frame's bytecode.
/// Used by C stdlib functions to get their name for error messages.
/// Mirrors C Lua's approach in luaL_argerror: lua_getinfo(L, 0, "n").
pub fn current_func_name_with_kind(l: &LuaState) -> Option<(&'static str, String)> {
    let ci_idx = l.call_depth().wrapping_sub(1);
    getfuncname(l, ci_idx)
}

/// Get just the name of the current function.
pub fn current_func_name(l: &LuaState) -> Option<String> {
    current_func_name_with_kind(l).map(|(_, name)| name)
}

/// Search through loaded modules (package.loaded) to find the name of a function.
/// Mirrors C Lua's pushglobalfuncname / findfield.
/// Returns e.g. "table.sort", "string.sub", "math.sin", etc.
fn find_global_func_name(l: &LuaState, target: &LuaValue) -> Option<String> {
    // Get _LOADED from registry by iterating registry entries
    let vm = unsafe { &*l.vm_ptr() };
    let registry_table = vm.registry.as_table()?;
    let mut loaded: Option<LuaValue> = None;
    for (key, val) in registry_table.iter_all() {
        if let Some(s) = key.as_str() {
            if s == "_LOADED" {
                loaded = Some(val);
                break;
            }
        }
    }
    let loaded = loaded?;
    let loaded_table = loaded.as_table()?;

    // Search through loaded modules (level 1: check each module's values)
    for (mod_key, mod_val) in loaded_table.iter_all() {
        if let Some(mod_name) = mod_key.as_str() {
            // Check if the module itself IS the target
            if mod_val == *target {
                let name = mod_name.to_string();
                // Strip _G. prefix
                return Some(if name.starts_with("_G.") {
                    name[3..].to_string()
                } else {
                    name
                });
            }
            // Search within the module table (level 2)
            if let Some(mod_table) = mod_val.as_table() {
                for (field_key, field_val) in mod_table.iter_all() {
                    if let Some(field_name) = field_key.as_str() {
                        if field_val == *target {
                            let full_name = format!("{}.{}", mod_name, field_name);
                            // Strip _G. prefix
                            return Some(if full_name.starts_with("_G.") {
                                full_name[3..].to_string()
                            } else {
                                full_name
                            });
                        }
                    }
                }
            }
        }
    }
    None
}

/// Generate a standard argument error message.
/// Mirrors C Lua's luaL_argerror.
pub fn argerror(l: &mut LuaState, narg: usize, extramsg: &str) -> LuaError {
    let result = current_func_name_with_kind(l);
    let (kind, fname) = match &result {
        Some((k, n)) => (*k, n.as_str()),
        None => {
            // Fallback: search loaded modules for the function (like pushglobalfuncname)
            let ci_idx = l.call_depth().wrapping_sub(1);
            let func_val = l.get_frame_func(ci_idx);
            let global_name = func_val.as_ref().and_then(|f| find_global_func_name(l, f));
            if let Some(name) = global_name {
                return l.error_from_c(format!(
                    "bad argument #{} to '{}' ({})",
                    narg, name, extramsg
                ));
            }
            ("function", "?")
        }
    };
    // For method calls, adjust argument numbering and handle "bad self"
    if kind == "method" {
        let adjusted_narg = narg.wrapping_sub(1);
        if adjusted_narg == 0 {
            return l.error_from_c(format!("calling '{}' on bad self ({})", fname, extramsg));
        }
        return l.error_from_c(format!(
            "bad argument #{} to '{}' ({})",
            adjusted_narg, fname, extramsg
        ));
    }
    l.error_from_c(format!(
        "bad argument #{} to '{}' ({})",
        narg, fname, extramsg
    ))
}

/// Generate a type error for a function argument.
/// Mirrors C Lua's luaL_typeerror.
pub fn arg_typeerror(l: &mut LuaState, narg: usize, expected: &str, val: &LuaValue) -> LuaError {
    let actual = objtypename(l, val);
    argerror(l, narg, &format!("{} expected, got {}", expected, actual))
}

/// Get variable info for a specific register.
/// Like varinfo() but for a known register number.
pub fn varinfo_for_reg(l: &LuaState, reg: u32) -> String {
    let ci_idx = l.call_depth().wrapping_sub(1);
    let ci = match l.get_frame(ci_idx) {
        Some(ci) => ci,
        None => return String::new(),
    };
    if !ci.is_lua() {
        return String::new();
    }
    let func = match l.get_frame_func(ci_idx) {
        Some(f) => f,
        None => return String::new(),
    };
    let lua_func = match func.as_lua_function() {
        Some(f) => f,
        None => return String::new(),
    };
    let chunk = lua_func.chunk();
    let currentpc = ci.pc.saturating_sub(1) as usize;
    if let Some((kind, name)) = getobjname(chunk, currentpc, reg) {
        format!(" ({} '{}')", kind, name)
    } else {
        String::new()
    }
}

/// Generate an arithmetic/bitwise type error (mirrors luaG_opinterror).
/// Determines which operand is the "bad" one and generates a type error.
pub fn opinterror(
    l: &mut LuaState,
    p1_reg: u32,
    p2_reg: u32,
    p1: &crate::LuaValue,
    p2: &crate::LuaValue,
    op: &str,
) -> LuaError {
    // If p1 is not a number, blame p1; otherwise blame p2
    let (blame_val, blame_reg) = if !p1.is_number() && !p1.is_integer() {
        (p1, p1_reg)
    } else {
        (p2, p2_reg)
    };
    let blame_type = objtypename(l, blame_val);
    let info = varinfo_for_reg(l, blame_reg);
    l.error(format!("attempt to {} a {} value{}", op, blame_type, info))
}

/// Generate a comparison error (mirrors luaG_ordererror).
pub fn ordererror(l: &mut LuaState, p1: &crate::LuaValue, p2: &crate::LuaValue) -> LuaError {
    let t1 = objtypename(l, p1);
    let t2 = objtypename(l, p2);
    if t1 == t2 {
        l.error(format!("attempt to compare two {} values", t1))
    } else {
        l.error(format!("attempt to compare {} with {}", t1, t2))
    }
}

/// Generate a call error with function name info (mirrors luaG_callerror).
/// Used when attempting to call a non-callable value.
pub fn callerror(l: &mut LuaState, val: &crate::LuaValue) -> LuaError {
    // Look at the current frame's instruction to determine what was being called
    let ci_idx = l.call_depth().wrapping_sub(1);
    if let Some(ci) = l.get_frame(ci_idx) {
        if ci.is_lua() {
            if let Some(func) = l.get_frame_func(ci_idx) {
                if let Some(lua_func) = func.as_lua_function() {
                    let chunk = lua_func.chunk();
                    let pc = ci.pc.saturating_sub(1) as usize;
                    if let Some((kind, name)) = funcnamefromcode(chunk, pc) {
                        let t = objtypename(l, val);
                        return l.error(format!(
                            "attempt to call a {} value ({} '{}')",
                            t, kind, name
                        ));
                    }
                }
            }
        }
    }
    // Fallback: no name info available
    let t = objtypename(l, val);
    l.error(format!("attempt to call a {} value", t))
}

/// Get the function name for a given frame index (public wrapper).
/// Returns (kind, name) or None.
pub fn pub_getfuncname(l: &LuaState, ci_frame_idx: usize) -> Option<(&'static str, String)> {
    getfuncname(l, ci_frame_idx)
}

pub fn create_debug_lib() -> LibraryModule {
    crate::lib_module!("debug", {
        "traceback" => debug_traceback,
        "getinfo" => debug_getinfo,
        "getmetatable" => debug_getmetatable,
        "setmetatable" => debug_setmetatable,
        "getregistry" => debug_getregistry,
        "getlocal" => debug_getlocal,
        "setlocal" => debug_setlocal,
        "getupvalue" => debug_getupvalue,
        "setupvalue" => debug_setupvalue,
        "upvalueid" => debug_upvalueid,
        "upvaluejoin" => debug_upvaluejoin,
        "gethook" => debug_gethook,
        "sethook" => debug_sethook,
        "setuservalue" => debug_setuservalue,
        "getuservalue" => debug_getuservalue,
    })
}

/// debug.traceback([message [, level]]) - Get stack traceback
fn debug_traceback(l: &mut LuaState) -> LuaResult<usize> {
    // Get message argument (can be nil)
    let message_val = l.get_arg(1).unwrap_or(LuaValue::nil());
    let message_str = if message_val.is_nil() {
        None
    } else if let Some(s) = message_val.as_str() {
        Some(s.to_string())
    } else {
        return Err(l.error("bad argument #1 to 'traceback' (string or nil expected)".to_string()));
    };

    // Get level argument (default is 1)
    let level = l
        .get_arg(2)
        .and_then(|v| v.as_integer())
        .unwrap_or(1)
        .max(0) as usize;

    // Generate traceback
    let mut trace = String::new();

    if let Some(msg) = message_str {
        trace.push_str(&msg);
        trace.push('\n');
    }

    trace.push_str("stack traceback:");

    // Get call stack info
    let call_depth = l.call_depth();

    // Adjust level to skip the traceback function itself if called from Lua
    let start_level = level;

    // Port of luaL_traceback from lauxlib.c
    // LEVELS1 = 10 (first part), LEVELS2 = 11 (last part)
    const LEVELS1: usize = 10;
    const LEVELS2: usize = 11;

    // Collect frame indices (most-recent first, matching C Lua's level ordering)
    if start_level < call_depth {
        let frames: Vec<usize> = (start_level..call_depth).rev().collect();
        let total = frames.len();
        let limit2show: isize = if total > LEVELS1 + LEVELS2 {
            LEVELS1 as isize
        } else {
            -1 // show all
        };

        let mut countdown = limit2show;

        for (idx, &i) in frames.iter().enumerate() {
            if countdown == 0 {
                // Skip middle frames
                let n = total - LEVELS1 - LEVELS2;
                trace.push_str(&format!("\n\t...\t(skipping {} levels)", n));
                countdown -= 1; // prevent re-triggering
                continue;
            } else if countdown > 0 {
                countdown -= 1;
            }

            // Skip frames in the gap
            if limit2show > 0 && idx > LEVELS1 && idx < total - LEVELS2 {
                continue;
            }

            if let Some(func) = l.get_frame_func(i) {
                let pc = l.get_frame_pc(i);

                // Try to get function info
                if let Some(func_obj) = func.as_lua_function() {
                    let chunk = func_obj.chunk();
                    // Lua function
                    let source = chunk.source_name.as_deref().unwrap_or("?");

                    // Format source name (strip @ prefix if present)
                    let source_display = if source.starts_with('@') {
                        &source[1..]
                    } else {
                        source
                    };

                    // Get line number from pc
                    let pc_idx = pc.saturating_sub(1) as usize;
                    let line = if !chunk.line_info.is_empty() && pc_idx < chunk.line_info.len() {
                        chunk.line_info[pc_idx]
                    } else {
                        0
                    };

                    // Determine function name and type
                    let (name_what, func_name) = if chunk.linedefined == 0 {
                        // Main chunk (linedefined == 0 means top-level code)
                        ("main chunk", String::new())
                    } else if i == call_depth - 1 {
                        // Also main chunk if at bottom of stack
                        ("main chunk", String::new())
                    } else {
                        // Use getfuncname to resolve function name from calling frame
                        match getfuncname(l, i) {
                            Some((kind, name)) => (kind, name),
                            None => ("function '?'", String::new()),
                        }
                    };

                    if line > 0 {
                        if func_name.is_empty() {
                            trace.push_str(&format!(
                                "\n\t{}:{}: in {}",
                                source_display, line, name_what
                            ));
                        } else {
                            trace.push_str(&format!(
                                "\n\t{}:{}: in {} '{}'",
                                source_display, line, name_what, func_name
                            ));
                        }
                    } else {
                        if func_name.is_empty() {
                            trace.push_str(&format!("\n\t{}: in {}", source_display, name_what));
                        } else {
                            trace.push_str(&format!(
                                "\n\t{}: in {} '{}'",
                                source_display, name_what, func_name
                            ));
                        }
                    }
                } else if func.is_c_callable() {
                    // C function - try to get name from calling frame
                    match getfuncname(l, i) {
                        Some((kind, name)) => {
                            trace.push_str(&format!("\n\t[C]: in {} '{}'", kind, name));
                        }
                        None => {
                            trace.push_str("\n\t[C]: in ?");
                        }
                    }
                } else {
                    trace.push_str("\n\t?: in function");
                }
            }
        }
    }

    let result = l.create_string(&trace)?;
    l.push_value(result)?;
    Ok(1)
}

/// debug.getinfo([thread,] f [, what]) - Get function info
fn debug_getinfo(l: &mut LuaState) -> LuaResult<usize> {
    // Parse arguments
    let arg1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("getinfo requires at least 1 argument".to_string()))?;
    let arg2 = l.get_arg(2);

    // Determine if arg1 is a function or a stack level
    let (func, what_str) = if arg1.is_function() {
        // arg1 is a function, arg2 is what
        let what = if let Some(w) = arg2 {
            if let Some(s) = w.as_str() {
                s.to_string()
            } else {
                "flnSrtu".to_string() // Default what
            }
        } else {
            "flnSrtu".to_string()
        };
        (arg1, what)
    } else if let Some(level) = arg1.as_integer() {
        // arg1 is a stack level, arg2 is what
        let what = if let Some(w) = arg2 {
            if let Some(s) = w.as_str() {
                s.to_string()
            } else {
                "flnSrtu".to_string()
            }
        } else {
            "flnSrtu".to_string()
        };

        // Get function at stack level
        // Level mapping: level 0 = current function (getinfo itself)
        //                level 1 = caller of current function
        //                level n = n steps up the call stack
        // Frame index mapping: frame call_depth-1 = top of stack (most recent)
        //                      frame 0 = bottom of stack (oldest)
        // So: frame_idx = call_depth - 1 - level
        let call_depth = l.call_depth();
        if level < 0 || level as usize >= call_depth {
            // Level out of range
            return Ok(0);
        }

        let frame_idx = call_depth - 1 - (level as usize);
        let func = l
            .get_frame_func(frame_idx)
            .ok_or_else(|| l.error("invalid stack level".to_string()))?;
        (func, what)
    } else {
        return Err(
            l.error("bad argument #1 to 'getinfo' (function or number expected)".to_string())
        );
    };

    // Create result table
    let info_table = l.create_table(0, 8)?;

    // Process function info based on 'what' parameter
    if let Some(lua_func) = func.as_lua_function() {
        let chunk = lua_func.chunk();
        // Lua function
        if what_str.contains('S') {
            // Source info
            let source = chunk.source_name.as_deref().unwrap_or("?");
            let source_key = l.create_string("source")?;
            let source_val = l.create_string(source)?;
            l.raw_set(&info_table, source_key, source_val);

            let short_src_key = l.create_string("short_src")?;
            let short_src_val = l.create_string(source)?;
            l.raw_set(&info_table, short_src_key, short_src_val);

            let linedefined_key = l.create_string("linedefined")?;
            let linedefined_val = LuaValue::integer(chunk.linedefined as i64);
            l.raw_set(&info_table, linedefined_key, linedefined_val);

            let lastlinedefined_key = l.create_string("lastlinedefined")?;
            let lastlinedefined_val = LuaValue::integer(chunk.lastlinedefined as i64);
            l.raw_set(&info_table, lastlinedefined_key, lastlinedefined_val);

            let what_key = l.create_string("what")?;
            let what_val = l.create_string("Lua")?;
            l.raw_set(&info_table, what_key, what_val);
        }

        if what_str.contains('l') {
            // Current line (only meaningful for stack level)
            let currentline_key = l.create_string("currentline")?;
            let currentline_val = if let Some(level) = arg1.as_integer() {
                // Get PC from the specified call frame
                // Use same frame_idx calculation as above
                let call_depth = l.call_depth();
                let frame_idx = call_depth - 1 - (level as usize);
                let pc = l.get_frame_pc(frame_idx);
                // Get line number from pc
                let pc_idx = pc.saturating_sub(1) as usize;
                let line = if !chunk.line_info.is_empty() && pc_idx < chunk.line_info.len() {
                    chunk.line_info[pc_idx] as i64
                } else {
                    -1
                };
                LuaValue::integer(line)
            } else {
                // Direct function reference - no current line
                LuaValue::integer(-1)
            };
            l.raw_set(&info_table, currentline_key, currentline_val);
        }

        if what_str.contains('u') {
            // Upvalue info
            let nups_key = l.create_string("nups")?;
            let nups_val = LuaValue::integer(chunk.upvalue_count as i64);
            l.raw_set(&info_table, nups_key, nups_val);

            let nparams_key = l.create_string("nparams")?;
            let nparams_val = LuaValue::integer(chunk.param_count as i64);
            l.raw_set(&info_table, nparams_key, nparams_val);

            let isvararg_key = l.create_string("isvararg")?;
            let isvararg_val = LuaValue::boolean(chunk.is_vararg);
            l.raw_set(&info_table, isvararg_key, isvararg_val);
        }

        if what_str.contains('n') {
            // Name resolution: look at the calling frame's bytecode
            let (name_val, namewhat_val) = if let Some(level) = arg1.as_integer() {
                let call_depth = l.call_depth();
                let frame_idx = call_depth - 1 - (level as usize);
                if let Some((namewhat, name)) = getfuncname(l, frame_idx) {
                    (l.create_string(&name)?.into(), l.create_string(namewhat)?)
                } else {
                    (LuaValue::nil(), l.create_string("")?)
                }
            } else {
                (LuaValue::nil(), l.create_string("")?)
            };

            let name_key = l.create_string("name")?;
            l.raw_set(&info_table, name_key, name_val);

            let namewhat_key = l.create_string("namewhat")?;
            l.raw_set(&info_table, namewhat_key, namewhat_val);
        }

        if what_str.contains('t') {
            // Tail call info
            let istailcall_key = l.create_string("istailcall")?;
            let istailcall_val = LuaValue::boolean(false);
            l.raw_set(&info_table, istailcall_key, istailcall_val);

            // extraargs: number of __call metamethods in the call chain
            // Extract from call_status bits 8-11 (CIST_CCMT)
            let extraargs_opt = if let Some(level) = arg1.as_integer() {
                use crate::lua_vm::call_info::call_status;
                // Convert relative level to absolute frame index
                // level 0 = debug.getinfo itself
                // level 1 = caller of debug.getinfo
                // level n = call_depth - 1 - n
                let current_depth = l.call_depth();
                let frame_idx = current_depth.checked_sub(1 + level as usize);
                frame_idx
                    .and_then(|idx| l.get_frame(idx))
                    .map(|f| call_status::get_ccmt_count(f.call_status))
            } else {
                None
            };

            if let Some(extraargs) = extraargs_opt {
                let extraargs_key = l.create_string("extraargs")?;
                let extraargs_val = LuaValue::integer(extraargs as i64);
                l.raw_set(&info_table, extraargs_key, extraargs_val);
            }
        }

        if what_str.contains('f') {
            // Function itself
            let func_key = l.create_string("func")?;
            l.raw_set(&info_table, func_key, func);
        }
    } else if func.is_c_callable() {
        // C function
        if what_str.contains('S') {
            let source_key = l.create_string("source")?;
            let source_val = l.create_string("=[C]")?;
            l.raw_set(&info_table, source_key, source_val);

            let short_src_key = l.create_string("short_src")?;
            let short_src_val = l.create_string("[C]")?;
            l.raw_set(&info_table, short_src_key, short_src_val);

            let linedefined_key = l.create_string("linedefined")?;
            let linedefined_val = LuaValue::integer(-1);
            l.raw_set(&info_table, linedefined_key, linedefined_val);

            let lastlinedefined_key = l.create_string("lastlinedefined")?;
            let lastlinedefined_val = LuaValue::integer(-1);
            l.raw_set(&info_table, lastlinedefined_key, lastlinedefined_val);

            let what_key = l.create_string("what")?;
            let what_val = l.create_string("C")?;
            l.raw_set(&info_table, what_key, what_val);
        }

        if what_str.contains('n') {
            // Name resolution for C functions: look at calling frame
            let (name_val, namewhat_val) = if let Some(level) = arg1.as_integer() {
                let call_depth = l.call_depth();
                let frame_idx = call_depth - 1 - (level as usize);
                if let Some((namewhat, name)) = getfuncname(l, frame_idx) {
                    (l.create_string(&name)?.into(), l.create_string(namewhat)?)
                } else {
                    (LuaValue::nil(), l.create_string("")?)
                }
            } else {
                (LuaValue::nil(), l.create_string("")?)
            };

            let name_key = l.create_string("name")?;
            l.raw_set(&info_table, name_key, name_val);

            let namewhat_key = l.create_string("namewhat")?;
            l.raw_set(&info_table, namewhat_key, namewhat_val);
        }

        if what_str.contains('f') {
            let func_key = l.create_string("func")?;
            l.raw_set(&info_table, func_key, func);
        }

        if what_str.contains('t') {
            // Tail call info for C functions
            let istailcall_key = l.create_string("istailcall")?;
            let istailcall_val = LuaValue::boolean(false);
            l.raw_set(&info_table, istailcall_key, istailcall_val);

            // extraargs for C functions
            let extraargs_opt = if let Some(level) = arg1.as_integer() {
                use crate::lua_vm::call_info::call_status;
                // Convert relative level to absolute frame index
                let current_depth = l.call_depth();
                let frame_idx = current_depth.checked_sub(1 + level as usize);
                frame_idx
                    .and_then(|idx| l.get_frame(idx))
                    .map(|f| call_status::get_ccmt_count(f.call_status))
            } else {
                None
            };

            if let Some(extraargs) = extraargs_opt {
                let extraargs_key = l.create_string("extraargs")?;
                let extraargs_val = LuaValue::integer(extraargs as i64);
                l.raw_set(&info_table, extraargs_key, extraargs_val);
            }
        }
    }

    l.push_value(info_table)?;
    Ok(1)
}

/// debug.getmetatable(value) - Get metatable of a value (no protection)
fn debug_getmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("getmetatable() requires argument 1".to_string()))?;

    // For tables, get metatable directly
    let v = get_metatable(l, &value).unwrap_or(LuaValue::nil());
    // For other types, return nil (simplified)
    l.push_value(v)?;
    Ok(1)
}

/// debug.setmetatable(value, table) - Set metatable of a value
fn debug_setmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("setmetatable() requires argument 1".to_string()))?;

    let metatable = l.get_arg(2);

    let mt_val = match metatable {
        Some(mt) if mt.is_nil() => None,
        Some(mt) if mt.is_table() => Some(mt),
        Some(_) => return Err(l.error("setmetatable() requires a table or nil".to_string())),
        None => None,
    };

    if let Some(table) = value.as_table_mut() {
        // For tables, set metatable directly on the table
        table.set_metatable(mt_val);
        // GC write barrier: table may be BLACK, new metatable may be WHITE
        if let Some(gc_ptr) = value.as_gc_ptr() {
            l.gc_barrier_back(gc_ptr);
        }
    } else {
        // For basic types (number, string, boolean), set the global type metatable
        let kind = value.kind();
        l.vm_mut().set_basic_metatable(kind, mt_val);
    }

    // Register for finalization if __gc is present
    l.vm_mut().gc.check_finalizer(&value);

    l.push_value(value)?;
    Ok(1)
}

/// debug.gethook([thread]) - Get current hook settings
/// Stub implementation: always returns nil (no hooks set)
fn debug_gethook(_l: &mut LuaState) -> LuaResult<usize> {
    // TODO: Implement proper hook support
    // For now, return nil to indicate no hook is set
    Ok(0) // Return nothing (nil)
}

/// debug.sethook([thread,] hook, mask [, count]) - Set a debug hook
/// Stub implementation: accepts arguments but does nothing
fn debug_sethook(_l: &mut LuaState) -> LuaResult<usize> {
    // TODO: Implement proper hook support
    // For now, just accept the arguments and do nothing
    Ok(0) // Return nothing
}

/// debug.getregistry() - Return the registry table
fn debug_getregistry(l: &mut LuaState) -> LuaResult<usize> {
    let registry = l.vm_mut().registry.clone();
    l.push_value(registry)?;
    Ok(1)
}

/// debug.getlocal([thread,] f, local) - Get the name and value of a local variable
fn debug_getlocal(l: &mut LuaState) -> LuaResult<usize> {
    // Parse arguments: [thread,] level/func, local_index
    let arg1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("getlocal requires at least 2 arguments".to_string()))?;
    let arg2 = l
        .get_arg(2)
        .ok_or_else(|| l.error("getlocal requires at least 2 arguments".to_string()))?;

    // For now, we only support level (not thread or function)
    // arg1: level (stack level)
    // arg2: local_index
    let level = arg1
        .as_integer()
        .ok_or_else(|| l.error("bad argument #1 to 'getlocal' (number expected)".to_string()))?
        as usize;
    let local_index = arg2
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'getlocal' (number expected)".to_string()))?
        as usize;

    // Get the call frame at the specified level
    // Level mapping: level 0 = current function (caller of getlocal)
    //                level 1 = its caller, etc.
    let call_depth = l.call_depth();
    if level >= call_depth {
        // Level out of range, return nil
        return Ok(0);
    }

    let frame_idx = call_depth - 1 - level;

    // Get function at this level
    let frame_func = l
        .get_frame_func(frame_idx)
        .ok_or_else(|| l.error("invalid stack level".to_string()))?;

    if let Some(lua_func) = frame_func.as_lua_function() {
        let chunk = lua_func.chunk();
        // Get current PC for this frame
        let pc = l.get_frame_pc(frame_idx) as usize;
        // PC is usually 1 ahead of the currently executing instruction
        let pc = if pc > 0 { pc - 1 } else { 0 };

        // Use PC-based filtering to find the Nth active local
        // Walk through chunk.locals, counting only those active at current PC
        let mut active_count = 0;
        let mut actual_slot = None;
        for (idx, locvar) in chunk.locals.iter().enumerate() {
            if (locvar.startpc as usize) > pc {
                break;
            }
            if pc < locvar.endpc as usize {
                active_count += 1;
                if active_count == local_index {
                    actual_slot = Some((idx, &locvar.name));
                    break;
                }
            }
        }

        if let Some((_idx, name)) = actual_slot {
            // The stack slot for active locals: find which register this is
            // Active locals are stored sequentially starting at base
            // We need to count how many locals are active before this one
            // to find its register offset
            let mut reg = 0;
            let mut count = 0;
            for locvar in &chunk.locals {
                if (locvar.startpc as usize) > pc {
                    break;
                }
                if pc < locvar.endpc as usize {
                    count += 1;
                    if count == local_index {
                        break;
                    }
                    reg += 1;
                }
            }

            let base = l.get_frame_base(frame_idx);
            let value_idx = base + reg;

            let top = l.get_top();
            if value_idx < top {
                let value = l.stack_get(value_idx).unwrap_or(LuaValue::nil());
                let name_str = l.create_string(name)?;
                l.push_value(name_str)?;
                l.push_value(value)?;
                return Ok(2);
            }
        }
    }

    // No local variable found, return nil
    Ok(0)
}

/// debug.setlocal([thread,] level, local, value) - Set the value of a local variable
fn debug_setlocal(l: &mut LuaState) -> LuaResult<usize> {
    // Parse arguments: [thread,] level, local_index, value
    let level_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("setlocal requires at least 3 arguments".to_string()))?;
    let local_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("setlocal requires at least 3 arguments".to_string()))?;
    let value = l
        .get_arg(3)
        .ok_or_else(|| l.error("setlocal requires at least 3 arguments".to_string()))?;

    let level = level_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #1 to 'setlocal' (number expected)".to_string()))?
        as usize;
    let local_index = local_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'setlocal' (number expected)".to_string()))?
        as usize;

    // Get the call frame at the specified level
    let call_depth = l.call_depth();
    if level >= call_depth {
        // Level out of range, return nil
        return Ok(0);
    }

    let frame_idx = call_depth - 1 - level;

    // Get function at this level
    let frame_func = l
        .get_frame_func(frame_idx)
        .ok_or_else(|| l.error("invalid stack level".to_string()))?;

    if let Some(lua_func) = frame_func.as_lua_function() {
        let chunk = lua_func.chunk();
        let pc = l.get_frame_pc(frame_idx) as usize;
        let pc = if pc > 0 { pc - 1 } else { 0 };

        // Find the Nth active local at current PC
        let mut active_count = 0;
        let mut reg = 0;
        let mut found_name = None;
        for locvar in &chunk.locals {
            if (locvar.startpc as usize) > pc {
                break;
            }
            if pc < locvar.endpc as usize {
                active_count += 1;
                if active_count == local_index {
                    found_name = Some(&locvar.name);
                    break;
                }
                reg += 1;
            }
        }

        if let Some(name) = found_name {
            let base = l.get_frame_base(frame_idx);
            let value_idx = base + reg;

            let top = l.get_top();
            if value_idx < top {
                l.stack_set(value_idx, value)?;
                let name_str = l.create_string(name)?;
                l.push_value(name_str)?;
                return Ok(1);
            }
        }
    }

    // No local variable found, return nil
    Ok(0)
}

/// debug.getupvalue(f, up) - Get the name and value of an upvalue
fn debug_getupvalue(l: &mut LuaState) -> LuaResult<usize> {
    let func = l
        .get_arg(1)
        .ok_or_else(|| l.error("getupvalue requires 2 arguments".to_string()))?;
    let up_index_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("getupvalue requires 2 arguments".to_string()))?;

    // Check that first argument is a function
    if !func.is_function() {
        return Err(l.error("bad argument #1 to 'getupvalue' (function expected)".to_string()));
    }

    let up_index = up_index_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'getupvalue' (number expected)".to_string()))?
        as usize;

    if let Some(lua_func) = func.as_lua_function() {
        // Get upvalue from Lua function
        let upvalues = lua_func.upvalues();
        if up_index > 0 && up_index <= upvalues.len() {
            let upvalue = &upvalues[up_index - 1];

            // Get the name from chunk
            let chunk = lua_func.chunk();
            if up_index <= chunk.upvalue_descs.len() {
                // Use actual upvalue name from chunk
                let name = &chunk.upvalue_descs[up_index - 1].name;
                let name_str = l.create_string(name)?;

                // Get the value
                let value = upvalue.as_ref().data.get_value();
                l.push_value(name_str)?;
                l.push_value(value)?;
                return Ok(2);
            }
        }
    }

    // No upvalue found, return nil
    Ok(0)
}

/// debug.setupvalue(f, up, value) - Set the value of an upvalue
fn debug_setupvalue(l: &mut LuaState) -> LuaResult<usize> {
    let func = l
        .get_arg(1)
        .ok_or_else(|| l.error("setupvalue requires 3 arguments".to_string()))?;
    let up_index_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("setupvalue requires 3 arguments".to_string()))?;
    let value = l
        .get_arg(3)
        .ok_or_else(|| l.error("setupvalue requires 3 arguments".to_string()))?;

    // Check that first argument is a function
    if !func.is_function() {
        return Err(l.error("bad argument #1 to 'setupvalue' (function expected)".to_string()));
    }

    let up_index = up_index_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'setupvalue' (number expected)".to_string()))?
        as usize;

    if let Some(lua_func) = func.as_lua_function() {
        // Set upvalue in Lua function
        let upvalues = lua_func.upvalues();
        if up_index > 0 && up_index <= upvalues.len() {
            let upvalue_ptr = upvalues[up_index - 1];

            let chunk = lua_func.chunk();
            // Get the upvalue name from the chunk
            let upvalue_name = if up_index - 1 < chunk.upvalue_descs.len() {
                chunk.upvalue_descs[up_index - 1].name.clone()
            } else {
                String::new()
            };

            // Set the upvalue value (similar to SETUPVAL instruction)
            let upval_ref = upvalue_ptr.as_mut_ref();
            upval_ref.data.set_value(value);

            // GC barrier if needed
            if value.is_collectable() {
                if let Some(value_gc_ptr) = value.as_gc_ptr() {
                    l.gc_barrier(upvalue_ptr, value_gc_ptr);
                }
            }

            // Return the upvalue name
            if !upvalue_name.is_empty() {
                let name_val = l.create_string(&upvalue_name)?;
                l.push_value(name_val)?;
                return Ok(1);
            } else {
                l.push_value(LuaValue::nil())?;
                return Ok(1);
            }
        }
    }

    // No upvalue found, return nil
    Ok(0)
}

/// debug.upvalueid(f, n) - Get a unique identifier for an upvalue
fn debug_upvalueid(l: &mut LuaState) -> LuaResult<usize> {
    let func = l
        .get_arg(1)
        .ok_or_else(|| l.error("upvalueid requires 2 arguments".to_string()))?;
    let up_index_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("upvalueid requires 2 arguments".to_string()))?;

    // Check that first argument is a function
    if !func.is_function() {
        return Err(l.error("bad argument #1 to 'upvalueid' (function expected)".to_string()));
    }

    let up_index = up_index_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'upvalueid' (number expected)".to_string()))?
        as usize;

    if let Some(lua_func) = func.as_lua_function() {
        let upvalues = lua_func.upvalues();
        if up_index > 0 && up_index <= upvalues.len() {
            let upvalue = &upvalues[up_index - 1];
            // Return light userdata (pointer) like C Lua
            let ptr = upvalue.as_ptr() as *mut std::ffi::c_void;
            l.push_value(LuaValue::lightuserdata(ptr))?;
            return Ok(1);
        }
    } else if let Some(cclosure) = func.as_cclosure() {
        let upvalues = cclosure.upvalues();
        if up_index > 0 && up_index <= upvalues.len() {
            let ptr = &upvalues[up_index - 1] as *const _ as *mut std::ffi::c_void;
            l.push_value(LuaValue::lightuserdata(ptr))?;
            return Ok(1);
        }
    }

    // Invalid upvalue index, return nil
    Ok(0)
}

/// debug.upvaluejoin(f1, n1, f2, n2) - Make upvalue n1 of f1 refer to upvalue n2 of f2
fn debug_upvaluejoin(l: &mut LuaState) -> LuaResult<usize> {
    let func1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("upvaluejoin requires 4 arguments".to_string()))?;
    let n1_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("upvaluejoin requires 4 arguments".to_string()))?;
    let func2 = l
        .get_arg(3)
        .ok_or_else(|| l.error("upvaluejoin requires 4 arguments".to_string()))?;
    let n2_val = l
        .get_arg(4)
        .ok_or_else(|| l.error("upvaluejoin requires 4 arguments".to_string()))?;

    // Check that arguments are functions
    if !func1.is_function() || !func2.is_function() {
        return Err(l.error("bad argument to 'upvaluejoin' (function expected)".to_string()));
    }

    // Check that they are Lua functions (not C functions)
    if func1.is_cfunction() || func2.is_cfunction() {
        return Err(l.error("bad argument to 'upvaluejoin' (Lua function expected)".to_string()));
    }

    let n1 = n1_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'upvaluejoin' (number expected)".to_string()))?
        as usize;
    let n2 = n2_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #4 to 'upvaluejoin' (number expected)".to_string()))?
        as usize;

    // Get both Lua functions
    let lua_func1 = func1
        .as_lua_function()
        .ok_or_else(|| l.error("upvaluejoin: function 1 is not a Lua function".to_string()))?;
    let lua_func2 = func2
        .as_lua_function()
        .ok_or_else(|| l.error("upvaluejoin: function 2 is not a Lua function".to_string()))?;

    // Check upvalue indices
    let upvalues1 = lua_func1.upvalues();
    let upvalues2 = lua_func2.upvalues();
    if n1 == 0 || n1 > upvalues1.len() {
        return Err(l.error(format!("invalid upvalue index {} for function 1", n1)));
    }
    if n2 == 0 || n2 > upvalues2.len() {
        return Err(l.error(format!("invalid upvalue index {} for function 2", n2)));
    }

    // Clone the upvalue from func2
    let upvalue_to_share = upvalues2[n2 - 1].clone();

    // Replace upvalue in func1 - we need mutable access
    let lua_func1_mut = func1.as_lua_function_mut().ok_or_else(|| {
        l.error("upvaluejoin: cannot get mutable reference to function 1".to_string())
    })?;

    let upvalues1_mut = lua_func1_mut.upvalues_mut();
    upvalues1_mut[n1 - 1] = upvalue_to_share;

    Ok(0)
}

/// debug.setuservalue(udata, value [, n]) - Set user value of a userdata
fn debug_setuservalue(l: &mut LuaState) -> LuaResult<usize> {
    let udata = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'setuservalue' (userdata expected)".to_string())
    })?;

    // Must be full userdata (not light userdata)
    if udata.ttislightuserdata() {
        return Err(l.error(
            "bad argument #1 to 'setuservalue' (full userdata expected, got light userdata)"
                .to_string(),
        ));
    }

    if !udata.is_userdata() {
        let t = udata.type_name();
        return Err(l.error(format!(
            "bad argument #1 to 'setuservalue' (userdata expected, got {})",
            t
        )));
    }

    // For now, setuservalue is a no-op (user values not yet stored in LuaUserdata)
    l.push_value(udata)?;
    Ok(1)
}

/// debug.getuservalue(udata [, n]) - Get user value of a userdata
fn debug_getuservalue(l: &mut LuaState) -> LuaResult<usize> {
    let udata = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'getuservalue' (userdata expected)".to_string())
    })?;

    if !udata.is_userdata() || udata.ttislightuserdata() {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    }

    // User values not yet supported, return nil
    l.push_value(LuaValue::nil())?;
    Ok(1)
}
