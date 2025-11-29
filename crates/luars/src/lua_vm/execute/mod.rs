/// Instruction dispatcher module
///
/// This module handles the execution of Lua VM instructions.
/// All instructions are inlined to eliminate function call overhead.

mod arithmetic_instructions;
mod control_instructions;
mod load_instructions;
mod loop_instructions;
mod table_instructions;
mod upvalue_instructions;

pub use arithmetic_instructions::*;
pub use control_instructions::*;
pub use load_instructions::*;
pub use loop_instructions::*;
pub use table_instructions::*;
pub use upvalue_instructions::*;

use crate::LuaValue;
use super::{Instruction, LuaError, LuaResult, LuaVM, OpCode};

/// Ultra-optimized main execution loop
/// 
/// Key optimizations:
/// 1. Instructions that never fail don't return Result - just continue
/// 2. frame_ptr is passed by mutable reference so CALL/RETURN can update it
/// 3. Minimal branching on the fast path
/// 
/// Returns: Ok(LuaValue) on success, Err on runtime error
#[inline(never)]  // Don't inline this - it's the main loop, let it stay in cache
pub fn luavm_execute(vm: &mut LuaVM) -> LuaResult<LuaValue> {
    // Safety check: must have at least one frame to execute
    if vm.frame_count == 0 {
        return Err(LuaError::Exit);
    }
    
    // Initialize frame pointer - will be updated by CALL/RETURN
    let mut frame_ptr = unsafe { 
        vm.frames.as_mut_ptr().add(vm.frame_count - 1) 
    };
    
    'mainloop: loop {
        // Fetch and decode instruction
        let instr = unsafe { (*frame_ptr).code_ptr.add((*frame_ptr).pc).read() };
        unsafe { (*frame_ptr).pc += 1; }
        
        let opcode = Instruction::get_opcode(instr);
        
        match opcode {
            // ============ Load Instructions (never fail) ============
            OpCode::Move => {
                exec_move(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadI => {
                exec_loadi(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadNil => {
                exec_loadnil(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadFalse => {
                exec_loadfalse(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadTrue => {
                exec_loadtrue(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LFalseSkip => {
                exec_lfalseskip(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadF => {
                exec_loadf(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadK => {
                exec_loadk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadKX => {
                exec_loadkx(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::VarargPrep => {
                exec_varargprep(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            
            // ============ Arithmetic (integer fast path never fails) ============
            OpCode::Add => {
                exec_add(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Sub => {
                exec_sub(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Mul => {
                exec_mul(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::AddI => {
                exec_addi(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Div => {
                exec_div(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::IDiv => {
                exec_idiv(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Mod => {
                exec_mod(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Pow => {
                exec_pow(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            
            // Arithmetic with constants
            OpCode::AddK => {
                exec_addk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::SubK => {
                exec_subk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::MulK => {
                exec_mulk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::ModK => {
                exec_modk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::PowK => {
                exec_powk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::DivK => {
                exec_divk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::IDivK => {
                exec_idivk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            
            // ============ Bitwise (never fail for integers) ============
            OpCode::BAnd => {
                exec_band(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::BOr => {
                exec_bor(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::BXor => {
                exec_bxor(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Shl => {
                exec_shl(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Shr => {
                exec_shr(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::BAndK => {
                exec_bandk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::BOrK => {
                exec_bork(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::BXorK => {
                exec_bxork(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::ShrI => {
                exec_shri(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::ShlI => {
                exec_shli(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::BNot => {
                if let Err(e) = exec_bnot(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            
            // ============ Unary operations ============
            OpCode::Not => {
                exec_not(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            
            // ============ Metamethod stubs (skip, handled by previous instruction) ============
            OpCode::MmBin => {
                if let Err(e) = exec_mmbin(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::MmBinI => {
                if let Err(e) = exec_mmbini(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::MmBinK => {
                if let Err(e) = exec_mmbink(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            
            // ============ Comparisons (never fail for basic types) ============
            OpCode::LtI => {
                exec_lti(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LeI => {
                exec_lei(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::GtI => {
                exec_gti(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::GeI => {
                exec_gei(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::EqI => {
                exec_eqi(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::EqK => {
                if let Err(e) = exec_eqk(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            
            // ============ Control Flow (never fail) ============
            OpCode::Jmp => {
                exec_jmp(instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Test => {
                exec_test(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::TestSet => {
                exec_testset(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            
            // ============ Loop Instructions (never fail for integer loops) ============
            OpCode::ForPrep => {
                if let Err(e) = exec_forprep(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::ForLoop => {
                if let Err(e) = exec_forloop(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::TForPrep => {
                exec_tforprep(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::TForLoop => {
                exec_tforloop(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            
            // ============ Upvalue operations ============
            OpCode::GetUpval => {
                exec_getupval(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::SetUpval => {
                exec_setupval(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            
            // ============ Extra arg (no-op) ============
            OpCode::ExtraArg => {
                continue 'mainloop;
            }
            
            // ============ Return Instructions (special handling - can Exit) ============
            OpCode::Return0 => {
                match exec_return0(vm, instr, &mut frame_ptr) {
                    Ok(()) => continue 'mainloop,
                    Err(LuaError::Exit) => return Ok(LuaValue::nil()),
                    Err(e) => return Err(e),
                }
            }
            OpCode::Return1 => {
                match exec_return1(vm, instr, &mut frame_ptr) {
                    Ok(()) => continue 'mainloop,
                    Err(LuaError::Exit) => return Ok(LuaValue::nil()),
                    Err(e) => return Err(e),
                }
            }
            OpCode::Return => {
                match exec_return(vm, instr, &mut frame_ptr) {
                    Ok(()) => continue 'mainloop,
                    Err(LuaError::Exit) => return Ok(LuaValue::nil()),
                    Err(e) => return Err(e),
                }
            }
            
            // ============ Function calls (update frame_ptr) ============
            OpCode::Call => {
                if let Err(e) = exec_call(vm, instr, &mut frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::TailCall => {
                match exec_tailcall(vm, instr, &mut frame_ptr) {
                    Ok(()) => continue 'mainloop,
                    Err(LuaError::Exit) => return Ok(LuaValue::nil()),
                    Err(e) => return Err(e),
                }
            }
            
            // ============ Table operations (can trigger metamethods) ============
            OpCode::NewTable => {
                exec_newtable(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::GetTable => {
                if let Err(e) = exec_gettable(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetTable => {
                if let Err(e) = exec_settable(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetI => {
                if let Err(e) = exec_geti(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetI => {
                if let Err(e) = exec_seti(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetField => {
                if let Err(e) = exec_getfield(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetField => {
                if let Err(e) = exec_setfield(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetTabUp => {
                if let Err(e) = exec_gettabup(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetTabUp => {
                if let Err(e) = exec_settabup(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Self_ => {
                if let Err(e) = exec_self(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            
            // ============ Operations that can trigger metamethods ============
            OpCode::Unm => {
                if let Err(e) = exec_unm(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Len => {
                if let Err(e) = exec_len(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Concat => {
                if let Err(e) = exec_concat(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Eq => {
                if let Err(e) = exec_eq(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Lt => {
                if let Err(e) = exec_lt(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Le => {
                if let Err(e) = exec_le(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            
            // ============ TForCall ============
            OpCode::TForCall => {
                if let Err(e) = exec_tforcall(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            
            // ============ Closure and special ============
            OpCode::Closure => {
                if let Err(e) = exec_closure(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Vararg => {
                if let Err(e) = exec_vararg(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetList => {
                exec_setlist(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Close => {
                exec_close(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Tbc => {
                exec_tbc(vm, instr, frame_ptr);
                continue 'mainloop;
            }
        }
    }
}
