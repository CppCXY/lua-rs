// Compiler helper functions

use crate::opcode::{OpCode, Instruction};
use crate::value::{LuaValue, LuaString};
use super::{Compiler, Local};

/// Emit an instruction and return its position
pub fn emit(c: &mut Compiler, instr: u32) -> usize {
    c.chunk.code.push(instr);
    c.chunk.code.len() - 1
}

/// Emit a jump instruction and return its position for later patching
pub fn emit_jump(c: &mut Compiler, opcode: OpCode) -> usize {
    emit(c, Instruction::encode_asbx(opcode, 0, 0))
}

/// Patch a jump instruction at the given position
pub fn patch_jump(c: &mut Compiler, pos: usize) {
    let jump = (c.chunk.code.len() - pos - 1) as i32;
    c.chunk.code[pos] = Instruction::encode_asbx(OpCode::Jmp, 0, jump);
}

/// Add a constant to the constant pool
pub fn add_constant(c: &mut Compiler, value: LuaValue) -> u32 {
    c.chunk.constants.push(value);
    (c.chunk.constants.len() - 1) as u32
}

/// Allocate a new register
pub fn alloc_register(c: &mut Compiler) -> u32 {
    let reg = c.next_register;
    c.next_register += 1;
    if c.next_register as usize > c.chunk.max_stack_size {
        c.chunk.max_stack_size = c.next_register as usize;
    }
    reg
}

/// Free a register (simple implementation)
#[allow(dead_code)]
pub fn free_register(c: &mut Compiler) {
    if c.next_register > 0 {
        c.next_register -= 1;
    }
}

/// Add a local variable
pub fn add_local(c: &mut Compiler, name: String, register: u32) {
    c.locals.push(Local {
        name,
        depth: c.scope_depth,
        register,
    });
}

/// Resolve a local variable by name
pub fn resolve_local<'a>(c: &'a Compiler, name: &str) -> Option<&'a Local> {
    c.locals.iter().rev().find(|l| l.name == name)
}

/// Begin a new scope
pub fn begin_scope(c: &mut Compiler) {
    c.scope_depth += 1;
}

/// End the current scope
pub fn end_scope(c: &mut Compiler) {
    c.scope_depth -= 1;
    c.locals.retain(|l| l.depth <= c.scope_depth);
    // Clear labels from the scope being closed
    clear_scope_labels(c);
}

/// Get a global variable
pub fn emit_get_global(c: &mut Compiler, name: &str, dest_reg: u32) {
    let const_idx = add_constant(c, LuaValue::string(LuaString::new(name.to_string())));
    emit(c, Instruction::encode_abx(OpCode::GetGlobal, dest_reg, const_idx));
}

/// Set a global variable
pub fn emit_set_global(c: &mut Compiler, name: &str, src_reg: u32) {
    let const_idx = add_constant(c, LuaValue::string(LuaString::new(name.to_string())));
    emit(c, Instruction::encode_abx(OpCode::SetGlobal, src_reg, const_idx));
}

/// Load nil into a register
pub fn emit_load_nil(c: &mut Compiler, reg: u32) {
    emit(c, Instruction::encode_abc(OpCode::LoadNil, reg, 0, 0));
}

/// Load boolean into a register
pub fn emit_load_bool(c: &mut Compiler, reg: u32, value: bool) {
    emit(c, Instruction::encode_abc(OpCode::LoadBool, reg, value as u32, 0));
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

/// Begin a new loop (for break statement support)
pub fn begin_loop(c: &mut Compiler) {
    c.loop_stack.push(super::LoopInfo {
        break_jumps: Vec::new(),
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
            emit(c, Instruction::encode_asbx(OpCode::Jmp, 0, offset));
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
    let label_pos = c.labels.iter()
        .find(|l| l.name == label_name)
        .map(|l| l.position)
        .unwrap();
    
    // Find and patch all gotos to this label
    let mut i = 0;
    while i < c.gotos.len() {
        if c.gotos[i].name == label_name {
            let goto = c.gotos.remove(i);
            let offset = label_pos as i32 - goto.jump_position as i32 - 1;
            c.chunk.code[goto.jump_position] = Instruction::encode_asbx(OpCode::Jmp, 0, offset);
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
