/// Instruction dispatcher module
/// 
/// This module handles the execution of Lua VM instructions.
/// It's separated from the main VM to allow reuse in different execution contexts:
/// - Direct VM.run() loop
/// - Function calls (CALL instruction)
/// - Coroutine resume/yield
/// - Potential Rust async integration

mod load_instructions;
// mod arithmetic_instructions;  // TODO: Uncomment when implemented
mod control_instructions;

pub use load_instructions::*;
// pub use arithmetic_instructions::*;  // TODO: Uncomment when implemented
pub use control_instructions::*;

use super::{LuaVM, LuaResult, LuaError, Instruction, OpCode};

/// Main instruction dispatcher
/// 
/// This function executes a single instruction and returns whether execution should continue.
/// It's designed to be called from multiple contexts:
/// - VM.run() main loop
/// - CALL instruction execution
/// - Coroutine yield/resume
pub fn dispatch_instruction(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let opcode = Instruction::get_opcode(instr);
    
    match opcode {
        // Load instructions
        OpCode::VarargPrep => exec_varargprep(vm, instr),
        OpCode::LoadNil => exec_loadnil(vm, instr),
        OpCode::LoadFalse => exec_loadfalse(vm, instr),
        OpCode::LoadTrue => exec_loadtrue(vm, instr),
        OpCode::LoadI => exec_loadi(vm, instr),
        OpCode::LoadF => exec_loadf(vm, instr),
        OpCode::LoadK => exec_loadk(vm, instr),
        OpCode::LoadKX => exec_loadkx(vm, instr),
        OpCode::Move => exec_move(vm, instr),
        
        // Control flow
        OpCode::Return => exec_return(vm, instr),
        
        // TODO: Add more instruction handlers
        _ => {
            Err(LuaError::RuntimeError(format!(
                "Unimplemented opcode: {:?} (0x{:02x})",
                opcode, opcode as u8
            )))
        }
    }
}

/// Action to take after dispatching an instruction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchAction {
    /// Continue executing next instruction
    Continue,
    /// Return from current function (includes return values in VM)
    Return,
    /// Yield from coroutine (yield values stored in thread)
    Yield,
    /// Call another function (caller should set up new frame)
    Call,
}
