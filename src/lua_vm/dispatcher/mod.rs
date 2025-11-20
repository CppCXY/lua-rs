/// Instruction dispatcher module
/// 
/// This module handles the execution of Lua VM instructions.
/// It's separated from the main VM to allow reuse in different execution contexts:
/// - Direct VM.run() loop
/// - Function calls (CALL instruction)
/// - Coroutine resume/yield
/// - Potential Rust async integration

mod load_instructions;
mod arithmetic_instructions;
mod control_instructions;
mod table_instructions;
mod upvalue_instructions;
mod loop_instructions;

pub use load_instructions::*;
pub use arithmetic_instructions::*;
pub use control_instructions::*;
pub use table_instructions::*;
pub use upvalue_instructions::*;
pub use loop_instructions::*;

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
        OpCode::LFalseSkip => exec_lfalseskip(vm, instr),
        OpCode::LoadTrue => exec_loadtrue(vm, instr),
        OpCode::LoadI => exec_loadi(vm, instr),
        OpCode::LoadF => exec_loadf(vm, instr),
        OpCode::LoadK => exec_loadk(vm, instr),
        OpCode::LoadKX => exec_loadkx(vm, instr),
        OpCode::Move => exec_move(vm, instr),
        
        // Arithmetic instructions (register-register)
        OpCode::Add => exec_add(vm, instr),
        OpCode::Sub => exec_sub(vm, instr),
        OpCode::Mul => exec_mul(vm, instr),
        OpCode::Div => exec_div(vm, instr),
        OpCode::IDiv => exec_idiv(vm, instr),
        OpCode::Mod => exec_mod(vm, instr),
        OpCode::Pow => exec_pow(vm, instr),
        
        // Unary operations
        OpCode::Unm => exec_unm(vm, instr),
        
        // Arithmetic with immediate/constant
        OpCode::AddI => exec_addi(vm, instr),
        OpCode::AddK => exec_addk(vm, instr),
        OpCode::SubK => exec_subk(vm, instr),
        OpCode::MulK => exec_mulk(vm, instr),
        OpCode::ModK => exec_modk(vm, instr),
        OpCode::PowK => exec_powk(vm, instr),
        OpCode::DivK => exec_divk(vm, instr),
        OpCode::IDivK => exec_idivk(vm, instr),
        
        // Bitwise operations (register-register)
        OpCode::BAnd => exec_band(vm, instr),
        OpCode::BOr => exec_bor(vm, instr),
        OpCode::BXor => exec_bxor(vm, instr),
        OpCode::Shl => exec_shl(vm, instr),
        OpCode::Shr => exec_shr(vm, instr),
        
        // Metamethod binary operations
        OpCode::MmBin => exec_mmbin(vm, instr),
        OpCode::MmBinI => exec_mmbini(vm, instr),
        OpCode::MmBinK => exec_mmbink(vm, instr),
        
        // Bitwise operations (with constant)
        OpCode::BAndK => exec_bandk(vm, instr),
        OpCode::BOrK => exec_bork(vm, instr),
        OpCode::BXorK => exec_bxork(vm, instr),
        OpCode::ShrI => exec_shri(vm, instr),
        OpCode::ShlI => exec_shli(vm, instr),
        
        // Unary operations
        OpCode::BNot => exec_bnot(vm, instr),
        OpCode::Not => exec_not(vm, instr),
        OpCode::Len => exec_len(vm, instr),
        
        // Comparison instructions
        OpCode::Eq => exec_eq(vm, instr),
        OpCode::Lt => exec_lt(vm, instr),
        OpCode::Le => exec_le(vm, instr),
        OpCode::EqK => exec_eqk(vm, instr),
        OpCode::EqI => exec_eqi(vm, instr),
        OpCode::LtI => exec_lti(vm, instr),
        OpCode::LeI => exec_lei(vm, instr),
        OpCode::GtI => exec_gti(vm, instr),
        OpCode::GeI => exec_gei(vm, instr),
        
        // Jump and test instructions
        OpCode::Jmp => exec_jmp(vm, instr),
        OpCode::Test => exec_test(vm, instr),
        OpCode::TestSet => exec_testset(vm, instr),
        
        // Table operations
        OpCode::NewTable => exec_newtable(vm, instr),
        OpCode::GetTable => exec_gettable(vm, instr),
        OpCode::SetTable => exec_settable(vm, instr),
        OpCode::GetI => exec_geti(vm, instr),
        OpCode::SetI => exec_seti(vm, instr),
        OpCode::GetField => exec_getfield(vm, instr),
        OpCode::SetField => exec_setfield(vm, instr),
        OpCode::GetTabUp => exec_gettabup(vm, instr),
        OpCode::SetTabUp => exec_settabup(vm, instr),
        OpCode::Self_ => exec_self(vm, instr),
        
        // Upvalue operations
        OpCode::GetUpval => exec_getupval(vm, instr),
        OpCode::SetUpval => exec_setupval(vm, instr),
        OpCode::Close => exec_close(vm, instr),
        
        // Closure and special operations
        OpCode::Closure => exec_closure(vm, instr),
        OpCode::Vararg => exec_vararg(vm, instr),
        OpCode::Concat => exec_concat(vm, instr),
        OpCode::SetList => exec_setlist(vm, instr),
        OpCode::Tbc => exec_tbc(vm, instr),
        
        // Loop operations
        OpCode::ForPrep => exec_forprep(vm, instr),
        OpCode::ForLoop => exec_forloop(vm, instr),
        OpCode::TForPrep => exec_tforprep(vm, instr),
        OpCode::TForCall => exec_tforcall(vm, instr),
        OpCode::TForLoop => exec_tforloop(vm, instr),
        
        // Call and return
        OpCode::Call => exec_call(vm, instr),
        OpCode::TailCall => exec_tailcall(vm, instr),
        OpCode::Return => exec_return(vm, instr),
        OpCode::Return0 => exec_return0(vm, instr),
        OpCode::Return1 => exec_return1(vm, instr),
        
        // Extra argument
        OpCode::ExtraArg => exec_extraarg(vm, instr),
        
        // All instructions implemented!
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
