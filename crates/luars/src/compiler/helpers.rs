// Compiler helper functions

use emmylua_parser::{LuaExpr, LuaLiteralToken};

use super::{Compiler, Local, ScopeChain};
use crate::lua_value::LuaValue;
use crate::lua_vm::{Instruction, OpCode};
use std::cell::RefCell;
use std::rc::Rc;

/// Create a string value using VM's string pool
pub fn create_string_value(c: &mut Compiler, s: &str) -> LuaValue {
    unsafe { (*c.vm_ptr).create_string(s) }
}

/// Emit an instruction and return its position
/// Automatically fills line_info with last_line
pub fn emit(c: &mut Compiler, instr: u32) -> usize {
    c.chunk.code.push(instr);
    // Fill line_info - use last_line or 0 if not set
    c.chunk.line_info.push(c.last_line);
    c.chunk.code.len() - 1
}

/// Emit a jump instruction and return its position for later patching
pub fn emit_jump(c: &mut Compiler, opcode: OpCode) -> usize {
    // JMP uses sJ format, not AsBx
    emit(c, Instruction::create_sj(opcode, 0))
}

/// Patch a jump instruction at the given position
pub fn patch_jump(c: &mut Compiler, pos: usize) {
    let jump = (c.chunk.code.len() - pos - 1) as i32;
    // JMP uses sJ format
    c.chunk.code[pos] = Instruction::create_sj(OpCode::Jmp, jump);
}

/// Add a constant to the constant pool (without deduplication)
pub fn add_constant(c: &mut Compiler, value: LuaValue) -> u32 {
    c.chunk.constants.push(value);
    (c.chunk.constants.len() - 1) as u32
}

/// Add a constant with deduplication (Lua 5.4 style)
/// Returns the index in the constant table
pub fn add_constant_dedup(c: &mut Compiler, value: LuaValue) -> u32 {
    // Search for existing constant
    for (i, existing) in c.chunk.constants.iter().enumerate() {
        if values_equal(existing, &value) {
            return i as u32;
        }
    }
    // Not found, add new constant
    add_constant(c, value)
}

/// Helper: check if two LuaValues are equal for constant deduplication
fn values_equal(a: &LuaValue, b: &LuaValue) -> bool {
    // Use LuaValue's type checking methods
    if a.is_nil() && b.is_nil() {
        return true;
    }
    if let (Some(a_bool), Some(b_bool)) = (a.as_bool(), b.as_bool()) {
        return a_bool == b_bool;
    }
    if let (Some(a_int), Some(b_int)) = (a.as_integer(), b.as_integer()) {
        return a_int == b_int;
    }
    if let (Some(a_num), Some(b_num)) = (a.as_float(), b.as_float()) {
        return a_num == b_num;
    }
    // For strings, compare raw primary/secondary words to avoid unsafe
    if a.is_string() && b.is_string() {
        return a.primary == b.primary && a.secondary == b.secondary;
    }
    false
}

/// Try to add a constant to K table (for RK instructions)
/// Returns Some(k_index) if successful, None if too many constants
/// In Lua 5.4, K indices are limited to MAXARG_B (255)
pub fn try_add_constant_k(c: &mut Compiler, value: LuaValue) -> Option<u32> {
    let idx = add_constant_dedup(c, value);
    if idx <= Instruction::MAX_B {
        Some(idx)
    } else {
        None // Too many constants, must use register
    }
}

/// Allocate a new register
/// Lua equivalent: luaK_reserveregs(fs, 1)
#[track_caller]
pub fn alloc_register(c: &mut Compiler) -> u32 {
    let reg = c.freereg;
    c.freereg += 1;
    // Track peak freereg for max_stack_size
    if c.freereg > c.peak_freereg {
        c.peak_freereg = c.freereg;
    }
    if c.freereg as usize > c.chunk.max_stack_size {
        c.chunk.max_stack_size = c.freereg as usize;
    }
    reg
}

/// Reserve N consecutive registers
/// Lua equivalent: luaK_reserveregs(fs, n)
#[allow(dead_code)]
pub fn reserve_registers(c: &mut Compiler, n: u32) {
    c.freereg += n;
    if c.freereg as usize > c.chunk.max_stack_size {
        c.chunk.max_stack_size = c.freereg as usize;
    }
}

/// Ensure a specific register is available (update max_stack_size if needed)
/// This should be called when using a register directly without alloc_register
pub fn ensure_register(c: &mut Compiler, reg: u32) {
    let min_stack = (reg + 1) as usize;
    if min_stack > c.chunk.max_stack_size {
        c.chunk.max_stack_size = min_stack;
    }
}

/// Get a register for result: use dest if provided, otherwise allocate new
/// This is the CORRECT way to handle dest parameter - always ensures max_stack_size is updated
/// and freereg is protected so subsequent allocations don't overwrite dest
#[inline]
pub fn get_result_reg(c: &mut Compiler, dest: Option<u32>) -> u32 {
    match dest {
        Some(reg) => {
            ensure_register(c, reg);
            // CRITICAL: Also update freereg to protect this register from being overwritten
            // by subsequent allocations within the same expression
            if c.freereg <= reg {
                c.freereg = reg + 1;
            }
            reg
        }
        None => alloc_register(c),
    }
}

/// Free a register (only if >= nactvar)
/// Lua equivalent: freereg(fs, reg)
#[allow(dead_code)]
pub fn free_register(c: &mut Compiler, reg: u32) {
    // Only free if register is beyond the active local variables
    // This matches Lua's: if (reg >= luaY_nvarstack(fs)) fs->freereg--;
    if reg >= nvarstack(c) && reg == c.freereg - 1 {
        c.freereg -= 1;
    }
}

/// Free two registers in proper order
/// Lua equivalent: freeregs(fs, r1, r2)
#[allow(dead_code)]
pub fn free_registers(c: &mut Compiler, r1: u32, r2: u32) {
    if r1 > r2 {
        free_register(c, r1);
        free_register(c, r2);
    } else {
        free_register(c, r2);
        free_register(c, r1);
    }
}

/// Reset freereg to number of active local variables
/// Lua equivalent: fs->freereg = luaY_nvarstack(fs)
pub fn reset_freereg(c: &mut Compiler) {
    c.freereg = nvarstack(c);
}

/// Get the number of registers used by active local variables
/// Lua equivalent: luaY_nvarstack(fs)
pub fn nvarstack(c: &Compiler) -> u32 {
    // Count non-const locals in current scope
    // For simplicity, we use nactvar as the count
    c.nactvar as u32
}

/// Add a local variable to the current scope
pub fn add_local(c: &mut Compiler, name: String, register: u32) {
    add_local_with_attrs(c, name, register, false, false);
}



/// Add a new local variable with <const> and <close> attributes
pub fn add_local_with_attrs(
    c: &mut Compiler,
    name: String,
    register: u32,
    is_const: bool,
    is_to_be_closed: bool,
) {
    let local = Local {
        name: name.clone(),
        depth: c.scope_depth,
        register,
        is_const,
        is_to_be_closed,
        needs_close: false,
    };
    c.scope_chain.borrow_mut().locals.push(local);

    // Add to chunk's locals list for debugging/introspection
    c.chunk.locals.push(name);

    // Increment nactvar for non-const locals
    if !is_const {
        c.nactvar += 1;
    }

    // Emit TBC instruction for to-be-closed variables
    // NOTE: <close> variables use TBC instruction for cleanup, NOT the needclose flag!
    // needclose is ONLY for upvalues (captured locals), not for <close> variables
    if is_to_be_closed {
        emit(c, Instruction::encode_abc(OpCode::Tbc, register, 0, 0));
    }
}

/// Resolve a local variable by name (searches from innermost to outermost scope)
/// Now uses scope_chain directly
pub fn resolve_local<'a>(c: &'a Compiler, name: &str) -> Option<Local> {
    // Search in current scope_chain's locals
    let scope = c.scope_chain.borrow();
    scope.locals.iter().rev().find(|l| l.name == name).cloned()
}

// add_upvalue function removed - logic inlined into resolve_upvalue_from_chain

/// Resolve an upvalue by searching parent scopes through the scope chain
/// This is called when a variable is not found in local scope
/// Recursively searches through all ancestor scopes
pub fn resolve_upvalue_from_chain(c: &mut Compiler, name: &str) -> Option<usize> {
    // Check if already in current upvalues
    {
        let scope = c.scope_chain.borrow();
        if let Some((idx, _)) = scope
            .upvalues
            .iter()
            .enumerate()
            .find(|(_, uv)| uv.name == name)
        {
            return Some(idx);
        }
    }

    // Get parent scope (clone to avoid borrow issues)
    let parent = c.scope_chain.borrow().parent.clone()?;

    // Resolve from parent scope - this returns info about where the variable was found
    let (is_local, index) = resolve_in_parent_scope(&parent, name)?;

    // Add upvalue to current scope
    // Simply append in the order of first reference (Lua 5.4 behavior)
    let upvalue_index = {
        let mut scope = c.scope_chain.borrow_mut();
        scope.upvalues.push(super::Upvalue {
            name: name.to_string(),
            is_local,
            index,
        });
        scope.upvalues.len() - 1
    };
    
    // CRITICAL: If capturing parent's local variable, mark PARENT's block.upval=true
    // (对齐lparser.c的markupval: mark parent block's upval flag)
    // This will propagate to needclose when parent's block is closed
    if is_local {
        if let Some(prev_ptr) = c.prev {
            unsafe {
                if let Some(ref mut block) = (*prev_ptr).block {
                    block.upval = true;
                }
            }
        }
    }

    Some(upvalue_index)
}

/// Recursively resolve variable in parent scope chain
/// Returns (is_local, index) where:
/// - is_local=true means found in direct parent's locals (index = register)
/// - is_local=false means found in ancestor's upvalue chain (index = upvalue index in parent)
fn resolve_in_parent_scope(scope: &Rc<RefCell<ScopeChain>>, name: &str) -> Option<(bool, u32)> {
    // First, search in this scope's locals
    {
        let mut scope_ref = scope.borrow_mut();
        if let Some(local) = scope_ref.locals.iter_mut().rev().find(|l| l.name == name) {
            let register = local.register;
            // Mark this local as captured by a closure - needs CLOSE on scope exit
            local.needs_close = true;
            // Found as local - return (true, register)
            return Some((true, register));
        }
    }

    // Check if already in this scope's upvalues
    {
        let scope_ref = scope.borrow();
        if let Some((idx, _)) = scope_ref
            .upvalues
            .iter()
            .enumerate()
            .find(|(_, uv)| uv.name == name)
        {
            // Found in this scope's existing upvalues - return (false, upvalue_index)
            return Some((false, idx as u32));
        }
    }

    // Not found in this scope - search in grandparent
    let grandparent = scope.borrow().parent.clone()?;

    // Recursively resolve from grandparent
    let (gp_is_local, gp_index) = resolve_in_parent_scope(&grandparent, name)?;

    // Add upvalue to this scope (intermediate scope between caller and where variable was found)
    let upvalue_idx = {
        let mut scope_mut = scope.borrow_mut();
        // Check if already in upvalues
        if let Some((idx, _)) = scope_mut
            .upvalues
            .iter()
            .enumerate()
            .find(|(_, uv)| uv.name == name)
        {
            idx as u32
        } else {
            // Add new upvalue - always false because we're capturing from ancestor
            scope_mut.upvalues.push(super::Upvalue {
                name: name.to_string(),
                is_local: gp_is_local,
                index: gp_index,
            });
            (scope_mut.upvalues.len() - 1) as u32
        }
    };

    // Return (false, upvalue_idx) because caller needs to capture from our upvalue
    Some((false, upvalue_idx))
}

/// Begin a new scope
pub fn begin_scope(c: &mut Compiler) {
    c.scope_depth += 1;
}

/// Enter a new block (对齐lparser.c的enterblock)
pub(crate) fn enterblock(c: &mut Compiler, isloop: bool) {
    let bl = super::BlockCnt {
        previous: c.block.take(),
        first_label: c.labels.len(),
        first_goto: c.gotos.len(),
        nactvar: c.nactvar,
        upval: false,
        isloop,
        insidetbc: c.block.as_ref().map_or(false, |b| b.insidetbc),
    };
    c.block = Some(Box::new(bl));
    
    // Verify freereg == nactvar (对齐lua_assert)
    debug_assert_eq!(c.freereg as usize, c.nactvar);
}

/// Leave the current block (对齐lparser.c的leaveblock)
pub(crate) fn leaveblock(c: &mut Compiler) {
    let bl = c.block.take().expect("leaveblock called without matching enterblock");
    let stklevel = bl.nactvar as u32; // level outside the block
    
    // Remove block locals (removevars)
    remove_vars_to_level(c, bl.nactvar);
    
    // Verify we're back to level on entry
    debug_assert_eq!(bl.nactvar, c.nactvar);
    
    // Handle loop: fix pending breaks
    let mut hasclose = false;
    if bl.isloop {
        // Create "break" label and resolve break jumps
        hasclose = create_label_internal(c, "break", 0);
    }
    
    // Emit CLOSE if needed
    if !hasclose && bl.previous.is_some() && bl.upval {
        emit(c, Instruction::encode_abc(OpCode::Close, stklevel, 0, 0));
    }
    
    // CRITICAL: If block has upvalues, mark function's needclose flag
    // (对齐Official Lua: block.upval propagates to fs->needclose)
    // This ensures RETURN instructions will have k flag set
    if bl.upval {
        c.needclose = true;
    }
    
    // Free registers
    c.freereg = stklevel;
    
    // Remove local labels
    c.labels.truncate(bl.first_label);
    
    // Restore previous block
    c.block = bl.previous;
    
    // Move gotos out if was nested block
    // TODO: implement movegotosout logic
}

/// Remove variables to a specific level (对齐lparser.c的removevars)
fn remove_vars_to_level(c: &mut Compiler, level: usize) {
    c.nactvar = level;
    c.scope_chain.borrow_mut().locals.truncate(level);
}

/// Create a label internally (returns whether close instruction was added)
fn create_label_internal(c: &mut Compiler, name: &str, _line: u32) -> bool {
    // Resolve pending gotos to this label
    let mut needs_close = false;
    let label_pc = c.chunk.code.len();
    
    for i in (0..c.gotos.len()).rev() {
        if c.gotos[i].name == name {
            // Patch the jump
            let jump_pos = c.gotos[i].jump_position;
            let jump_offset = (label_pc as i32) - (jump_pos as i32) - 1;
            c.chunk.code[jump_pos] = Instruction::create_sj(OpCode::Jmp, jump_offset);
            
            // Remove resolved goto
            c.gotos.remove(i);
            
            // Check if needs close (simplified - official checks more)
            needs_close = true;
        }
    }
    
    // Add label
    c.labels.push(super::Label {
        name: name.to_string(),
        position: label_pc,
        scope_depth: c.scope_depth,
    });
    
    // Emit CLOSE if needed
    if needs_close {
        let stklevel = c.nactvar as u32;
        emit(c, Instruction::encode_abc(OpCode::Close, stklevel, 0, 0));
        true
    } else {
        false
    }
}

/// End the current scope
pub fn end_scope(c: &mut Compiler) {
    // Before closing the scope, emit CLOSE instruction for to-be-closed variables
    // Find the minimum register of all to-be-closed variables in the current scope
    let mut min_tbc_reg: Option<u32> = None;
    let mut removed_count = 0usize;
    {
        let scope = c.scope_chain.borrow();
        for local in scope.locals.iter().rev() {
            if local.depth > c.scope_depth {
                break; // Only check current scope
            }
            if local.depth == c.scope_depth {
                if local.is_to_be_closed {
                    min_tbc_reg = Some(match min_tbc_reg {
                        None => local.register,
                        Some(min_reg) => min_reg.min(local.register),
                    });
                }
                if !local.is_const {
                    removed_count += 1;
                }
            }
        }
    }

    // Emit CLOSE instruction if there are to-be-closed variables
    if let Some(reg) = min_tbc_reg {
        emit(c, Instruction::encode_abc(OpCode::Close, reg, 0, 0));
    }

    c.scope_depth -= 1;

    // Decrease nactvar by number of removed non-const locals
    c.nactvar = c.nactvar.saturating_sub(removed_count);

    c.scope_chain
        .borrow_mut()
        .locals
        .retain(|l| l.depth <= c.scope_depth);

    // Reset freereg after removing locals
    reset_freereg(c);

    // Clear labels from the scope being closed
    clear_scope_labels(c);
}

/// Get a global variable (Lua 5.4 uses _ENV upvalue)
pub fn emit_get_global(c: &mut Compiler, name: &str, dest_reg: u32) {
    // Ensure _ENV upvalue exists
    ensure_env_upvalue(c);

    // Find _ENV's actual index in upvalues
    let env_index = {
        let scope = c.scope_chain.borrow();
        scope
            .upvalues
            .iter()
            .position(|uv| uv.name == "_ENV")
            .expect("_ENV upvalue should exist after ensure_env_upvalue")
    };

    let lua_str = create_string_value(c, name);
    let const_idx = add_constant_dedup(c, lua_str);
    // GetTabUp: R(A) := UpValue[B][K(C)]
    // B is _ENV's upvalue index, C is constant index, k=1
    emit(
        c,
        Instruction::create_abck(
            OpCode::GetTabUp,
            dest_reg,
            env_index as u32,
            const_idx,
            true,
        ),
    );
}

/// Set a global variable (Lua 5.4 uses _ENV upvalue)
pub fn emit_set_global(c: &mut Compiler, name: &str, src_reg: u32) {
    // Ensure _ENV upvalue exists
    ensure_env_upvalue(c);

    // Find _ENV's actual index in upvalues
    let env_index = {
        let scope = c.scope_chain.borrow();
        scope
            .upvalues
            .iter()
            .position(|uv| uv.name == "_ENV")
            .expect("_ENV upvalue should exist after ensure_env_upvalue")
    };

    let lua_str = create_string_value(c, name);
    let const_idx = add_constant_dedup(c, lua_str);
    // SetTabUp: UpValue[A][K(B)] := RK(C)
    // A is _ENV's upvalue index, B is constant index for key, C is source register
    // k=false means C is a register index (not constant)
    emit(
        c,
        Instruction::create_abck(
            OpCode::SetTabUp,
            env_index as u32,
            const_idx,
            src_reg,
            false,
        ),
    );
}

/// Ensure _ENV is in upvalue[0]
fn ensure_env_upvalue(c: &mut Compiler) {
    let scope = c.scope_chain.borrow();

    // Check if _ENV exists anywhere in upvalues
    if scope.upvalues.iter().any(|uv| uv.name == "_ENV") {
        // _ENV already exists - no need to add it again
        return;
    }

    // _ENV doesn't exist - need to resolve it from parent
    drop(scope);

    // Try to resolve _ENV from parent scope chain
    // This will add _ENV to upvalues in the order of first reference (Lua 5.4 behavior)
    if let Some(_env_idx) = resolve_upvalue_from_chain(c, "_ENV") {
        // Successfully added _ENV to upvalues
        // No reordering needed - Lua 5.4 uses natural reference order
    } else {
        // Can't resolve _ENV from parent - this means we're in top-level chunk
        // Top-level chunk should have _ENV as upvalue[0] from VM initialization
        // We don't need to add it here
    }
}

/// Emit LoadK/LoadKX instruction (Lua 5.4 style)
/// Loads a constant from the constant table into a register
pub fn emit_loadk(c: &mut Compiler, dest: u32, const_idx: u32) -> usize {
    if const_idx <= Instruction::MAX_BX {
        emit(c, Instruction::encode_abx(OpCode::LoadK, dest, const_idx))
    } else {
        // Use LoadKX + ExtraArg for large constant indices (> 131071)
        let pos = emit(c, Instruction::encode_abx(OpCode::LoadKX, dest, 0));
        emit(c, Instruction::create_ax(OpCode::ExtraArg, const_idx));
        pos
    }
}

/// Emit LoadI instruction for small integers (Lua 5.4)
/// LoadI can encode integers directly in sBx field (-65536 to 65535)
/// Returns Some(pos) if successful, None if value too large
pub fn emit_loadi(c: &mut Compiler, dest: u32, value: i64) -> Option<usize> {
    if value >= i32::MIN as i64 && value <= i32::MAX as i64 {
        let sbx = value as i32;
        // Check if fits in sBx field
        if sbx >= -(Instruction::OFFSET_SBX) && sbx <= Instruction::OFFSET_SBX {
            return Some(emit(c, Instruction::encode_asbx(OpCode::LoadI, dest, sbx)));
        }
    }
    None // Value too large, must use LoadK
}

/// Emit LoadF instruction for floats (Lua 5.4)
/// LoadF encodes small floats in sBx field (integer-representable floats only)
/// Returns Some(pos) if successful, None if must use LoadK
pub fn emit_loadf(c: &mut Compiler, dest: u32, value: f64) -> Option<usize> {
    // For simplicity, only handle integer-representable floats
    if value.fract() == 0.0 {
        let int_val = value as i32;
        if int_val as f64 == value {
            if int_val >= -(Instruction::OFFSET_SBX) && int_val <= Instruction::OFFSET_SBX {
                return Some(emit(
                    c,
                    Instruction::encode_asbx(OpCode::LoadF, dest, int_val),
                ));
            }
        }
    }
    None // Complex float, must use LoadK
}

/// Load nil into a register
pub fn emit_load_nil(c: &mut Compiler, reg: u32) {
    emit(c, Instruction::encode_abc(OpCode::LoadNil, reg, 0, 0));
}

/// Load boolean into a register (Lua 5.4 uses LoadTrue/LoadFalse)
pub fn emit_load_bool(c: &mut Compiler, reg: u32, value: bool) {
    if value {
        emit(c, Instruction::encode_abc(OpCode::LoadTrue, reg, 0, 0));
    } else {
        emit(c, Instruction::encode_abc(OpCode::LoadFalse, reg, 0, 0));
    }
}

/// Load constant into a register
pub fn emit_load_constant(c: &mut Compiler, reg: u32, const_idx: u32) {
    emit(c, Instruction::encode_abx(OpCode::LoadK, reg, const_idx));
}

/// Move value from one register to another
pub fn emit_move(c: &mut Compiler, dest: u32, src: u32) {
    if dest != src {
        emit(c, Instruction::encode_abc(OpCode::Move, dest, src, 0));
    }
}

/// Try to compile expression as a constant, returns Some(const_idx) if successful
pub fn try_expr_as_constant(c: &mut Compiler, expr: &emmylua_parser::LuaExpr) -> Option<u32> {
    // Only handle literal expressions that can be constants
    if let LuaExpr::LiteralExpr(lit_expr) = expr {
        if let Some(literal_token) = lit_expr.get_literal() {
            match literal_token {
                LuaLiteralToken::Nil(_) => {
                    return try_add_constant_k(c, LuaValue::nil());
                }
                LuaLiteralToken::Bool(b) => {
                    return try_add_constant_k(c, LuaValue::boolean(b.is_true()));
                }
                LuaLiteralToken::Number(num) => {
                    let value = if num.is_float() {
                        LuaValue::float(num.get_float_value())
                    } else {
                        LuaValue::integer(num.get_int_value())
                    };
                    return try_add_constant_k(c, value);
                }
                LuaLiteralToken::String(s) => {
                    let lua_str = create_string_value(c, &s.get_value());
                    return try_add_constant_k(c, lua_str);
                }
                _ => {}
            }
        }
    }
    None
}

/// Begin a new loop (for break statement support)
pub fn begin_loop(c: &mut Compiler) {
    begin_loop_with_register(c, c.freereg);
}

/// Begin a new loop with a specific first register for CLOSE
pub fn begin_loop_with_register(c: &mut Compiler, first_reg: u32) {
    c.loop_stack.push(super::LoopInfo {
        break_jumps: Vec::new(),
        scope_depth: c.scope_depth,
        first_local_register: first_reg,
    });
}

/// End current loop and patch all break statements
pub fn end_loop(c: &mut Compiler) {
    if let Some(loop_info) = c.loop_stack.pop() {
        // Patch all break jumps to current position
        for jump_pos in loop_info.break_jumps {
            patch_jump(c, jump_pos);
        }
    }
}

/// Emit a break statement (jump to end of current loop)
pub fn emit_break(c: &mut Compiler) -> Result<(), String> {
    if c.loop_stack.is_empty() {
        return Err("break statement outside loop".to_string());
    }

    // Before breaking, check if we need to close any captured upvalues
    // This is needed when break jumps past the CLOSE instruction that would
    // normally be executed at the end of each iteration
    let loop_info = c.loop_stack.last().unwrap();
    let loop_scope_depth = loop_info.scope_depth;
    let first_reg = loop_info.first_local_register;
    
    // Find minimum register of captured locals in the loop scope
    let mut min_close_reg: Option<u32> = None;
    {
        let scope = c.scope_chain.borrow();
        for local in scope.locals.iter().rev() {
            if local.depth < loop_scope_depth {
                break; // Only check loop scope and nested scopes
            }
            if local.needs_close && local.register >= first_reg {
                min_close_reg = Some(match min_close_reg {
                    None => local.register,
                    Some(min_reg) => min_reg.min(local.register),
                });
            }
        }
    }
    
    // Emit CLOSE if needed
    if let Some(reg) = min_close_reg {
        emit(c, Instruction::encode_abc(OpCode::Close, reg, 0, 0));
    }

    let jump_pos = emit_jump(c, OpCode::Jmp);
    c.loop_stack.last_mut().unwrap().break_jumps.push(jump_pos);
    Ok(())
}

/// Define a label at current position
pub fn define_label(c: &mut Compiler, name: String) -> Result<(), String> {
    // Check if label already exists in current scope
    for label in &c.labels {
        if label.name == name && label.scope_depth == c.scope_depth {
            return Err(format!("label '{}' already defined", name));
        }
    }

    let position = c.chunk.code.len();
    c.labels.push(super::Label {
        name: name.clone(),
        position,
        scope_depth: c.scope_depth,
    });

    // Try to resolve any pending gotos to this label
    resolve_pending_gotos(c, &name);

    Ok(())
}

/// Emit a goto statement
pub fn emit_goto(c: &mut Compiler, label_name: String) -> Result<(), String> {
    // Check if label is already defined
    for label in &c.labels {
        if label.name == label_name {
            // Label found - emit direct jump
            let current_pos = c.chunk.code.len();
            let offset = label.position as i32 - current_pos as i32 - 1;
            emit(c, Instruction::create_sj(OpCode::Jmp, offset));
            return Ok(());
        }
    }

    // Label not yet defined - add to pending gotos
    let jump_pos = emit_jump(c, OpCode::Jmp);
    c.gotos.push(super::GotoInfo {
        name: label_name,
        jump_position: jump_pos,
        scope_depth: c.scope_depth,
    });

    Ok(())
}

/// Resolve pending gotos for a newly defined label
fn resolve_pending_gotos(c: &mut Compiler, label_name: &str) {
    let label_pos = c
        .labels
        .iter()
        .find(|l| l.name == label_name)
        .map(|l| l.position)
        .unwrap();

    // Find and patch all gotos to this label
    let mut i = 0;
    while i < c.gotos.len() {
        if c.gotos[i].name == label_name {
            let goto = c.gotos.remove(i);
            let offset = label_pos as i32 - goto.jump_position as i32 - 1;
            c.chunk.code[goto.jump_position] = Instruction::create_sj(OpCode::Jmp, offset);
        } else {
            i += 1;
        }
    }
}

/// Check for unresolved gotos (call at end of compilation)
pub fn check_unresolved_gotos(c: &Compiler) -> Result<(), String> {
    if !c.gotos.is_empty() {
        let names: Vec<_> = c.gotos.iter().map(|g| g.name.as_str()).collect();
        return Err(format!("undefined label(s): {}", names.join(", ")));
    }
    Ok(())
}

/// Clear labels when leaving a scope
pub fn clear_scope_labels(c: &mut Compiler) {
    c.labels.retain(|l| l.scope_depth < c.scope_depth);
}

/// Check if an expression is a vararg (...) literal
pub fn is_vararg_expr(expr: &LuaExpr) -> bool {
    if let LuaExpr::LiteralExpr(lit) = expr {
        matches!(lit.get_literal(), Some(LuaLiteralToken::Dots(_)))
    } else {
        false
    }
}

/// Result of parsing a Lua number literal
#[derive(Debug, Clone, Copy)]
pub enum ParsedNumber {
    /// Successfully parsed as integer
    Int(i64),
    /// Number is too large for i64, use float instead
    Float(f64),
}

/// Parse a Lua integer literal from text, handling hex numbers that overflow i64
/// Lua treats 0xFFFFFFFFFFFFFFFF as -1 (two's complement interpretation)
/// For decimal numbers that overflow i64 range, returns Float instead
pub fn parse_lua_int(text: &str) -> ParsedNumber {
    let text = text.trim();
    if text.starts_with("0x") || text.starts_with("0X") {
        // Hex number - parse as u64 first, then reinterpret as i64
        // This handles the case like 0xFFFFFFFFFFFFFFFF which should be -1
        let hex_part = &text[2..];
        // Remove any trailing decimal part (e.g., 0xFF.0)
        let hex_part = hex_part.split('.').next().unwrap_or(hex_part);
        if let Ok(val) = u64::from_str_radix(hex_part, 16) {
            return ParsedNumber::Int(val as i64); // Reinterpret bits as signed
        }
    }
    // Decimal case: parse as i64 only, if overflow use float
    // (Unlike hex, decimal numbers should NOT be reinterpreted as two's complement)
    if let Ok(val) = text.parse::<i64>() {
        return ParsedNumber::Int(val);
    }
    // Decimal number is too large for i64, parse as float
    if let Ok(val) = text.parse::<f64>() {
        return ParsedNumber::Float(val);
    }
    // Default fallback
    ParsedNumber::Int(0)
}

/// Lua left shift: x << n (returns 0 if |n| >= 64)
/// Negative n means right shift
#[inline(always)]
pub fn lua_shl(l: i64, r: i64) -> i64 {
    if r >= 64 || r <= -64 {
        0
    } else if r >= 0 {
        (l as u64).wrapping_shl(r as u32) as i64
    } else {
        // Negative shift means right shift (logical)
        (l as u64).wrapping_shr((-r) as u32) as i64
    }
}

/// Lua right shift: x >> n (logical shift, returns 0 if |n| >= 64)
/// Negative n means left shift
#[inline(always)]
pub fn lua_shr(l: i64, r: i64) -> i64 {
    if r >= 64 || r <= -64 {
        0
    } else if r >= 0 {
        (l as u64).wrapping_shr(r as u32) as i64
    } else {
        // Negative shift means left shift
        (l as u64).wrapping_shl((-r) as u32) as i64
    }
}
/// Finish function compilation (对齐lcode.c的luaK_finish)
/// Converts RETURN0/RETURN1 to RETURN when needed and sets k/C flags
pub(crate) fn finish_function(c: &mut Compiler) {
    let needclose = c.needclose;
    let is_vararg = c.chunk.is_vararg;
    let param_count = c.chunk.param_count;
    
    for i in 0..c.chunk.code.len() {
        let instr = c.chunk.code[i];
        let opcode = Instruction::get_opcode(instr);
        
        match opcode {
            OpCode::Return0 | OpCode::Return1 => {
                if needclose || is_vararg {
                    // Convert RETURN0/RETURN1 to RETURN
                    let a = Instruction::get_a(instr);
                    let b = if opcode == OpCode::Return0 { 1 } else { 2 }; // nret + 1
                    let c_val = if is_vararg { (param_count + 1) as u32 } else { 0 };
                    let new_instr = Instruction::create_abck(OpCode::Return, a, b, c_val, needclose);
                    
                    c.chunk.code[i] = new_instr;
                }
            }
            OpCode::Return | OpCode::TailCall => {
                if needclose || is_vararg {
                    let mut new_instr = instr;
                    
                    // Set k flag if needs to close upvalues
                    if needclose {
                        Instruction::set_k(&mut new_instr, true);
                    }
                    
                    // Set C flag if vararg (C = numparams + 1)
                    if is_vararg {
                        Instruction::set_c(&mut new_instr, (param_count + 1) as u32);
                    }
                    
                    c.chunk.code[i] = new_instr;
                }
            }
            _ => {}
        }
    }
}
