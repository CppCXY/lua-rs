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

use super::{Instruction, LuaError, LuaResult, LuaVM, OpCode};
use crate::LuaValue;
use crate::lua_value::{TAG_INTEGER, TYPE_MASK};

/// Ultra-optimized main execution loop
///
/// Key optimizations (like Lua C):
/// 1. Local variables: pc, code_ptr, base_ptr cached locally (like Lua C's luaV_execute)
/// 2. Hot path instructions inlined directly in match
/// 3. State reload only when frame changes (CALL/RETURN)
///
/// Returns: Ok(LuaValue) on success, Err on runtime error
#[inline(never)] // Don't inline this - it's the main loop, let it stay in cache
pub fn luavm_execute(vm: &mut LuaVM) -> LuaResult<LuaValue> {
    // Safety check: must have at least one frame to execute
    if vm.frame_count == 0 {
        return Err(LuaError::Exit);
    }

    // Initialize frame pointer - Box ensures pointer stability across Vec reallocs
    let mut frame_ptr = vm.current_frame_ptr();
    
    // Like Lua C: cache hot variables as locals (register allocated)
    // This avoids dereferencing frame_ptr on each instruction
    let mut pc: usize;
    let mut code_ptr: *const u32;
    let mut base_ptr: usize;
    
    // Initial load from frame
    unsafe {
        pc = (*frame_ptr).pc;
        code_ptr = (*frame_ptr).code_ptr;
        base_ptr = (*frame_ptr).base_ptr;
    }

    'mainloop: loop {
        // Fetch instruction using local pc (like Lua C's vmfetch)
        let instr = unsafe { *code_ptr.add(pc) };
        pc += 1;

        let opcode = Instruction::get_opcode(instr);

        match opcode {
            // ============ HOT PATH: Inline for maximum speed ============
            
            // MOVE - directly inline with cached base_ptr
            OpCode::Move => {
                let a = Instruction::get_a(instr) as usize;
                let b = Instruction::get_b(instr) as usize;
                unsafe {
                    let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr);
                    *reg_base.add(a) = *reg_base.add(b);
                }
                continue 'mainloop;
            }
            
            // LOADI - directly inline with cached base_ptr
            OpCode::LoadI => {
                let a = Instruction::get_a(instr) as usize;
                let sbx = Instruction::get_sbx(instr);
                unsafe {
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(sbx as i64);
                }
                continue 'mainloop;
            }
            
            // ============ Other Load Instructions ============
            OpCode::LoadNil => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_loadnil(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadFalse => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_loadfalse(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadTrue => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_loadtrue(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LFalseSkip => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_lfalseskip(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::LoadF => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_loadf(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_loadk(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::LoadKX => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_loadkx(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::VarargPrep => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_varargprep(vm, instr, frame_ptr);
                continue 'mainloop;
            }

            // ============ Arithmetic (hot path inline) ============
            OpCode::Add => {
                let a = Instruction::get_a(instr) as usize;
                let b = Instruction::get_b(instr) as usize;
                let c = Instruction::get_c(instr) as usize;
                unsafe {
                    let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr);
                    let left = *reg_base.add(b);
                    let right = *reg_base.add(c);
                    let combined_tags = (left.primary | right.primary) & TYPE_MASK;
                    if combined_tags == TAG_INTEGER {
                        let result = (left.secondary as i64).wrapping_add(right.secondary as i64);
                        *reg_base.add(a) = LuaValue { primary: TAG_INTEGER, secondary: result as u64 };
                        pc += 1; // Skip MMBIN
                        continue 'mainloop;
                    }
                }
                // Slow path: floats or metamethods
                unsafe { (*frame_ptr).pc = pc; }
                exec_add(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Sub => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_sub(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Mul => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_mul(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::AddI => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_addi(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Div => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_div(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::IDiv => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_idiv(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Mod => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_mod(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Pow => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_pow(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }

            // Arithmetic with constants
            OpCode::AddK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_addk(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::SubK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_subk(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::MulK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_mulk(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::ModK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_modk(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::PowK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_powk(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::DivK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_divk(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::IDivK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_idivk(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }

            // ============ Bitwise (never fail for integers) ============
            OpCode::BAnd => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_band(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::BOr => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_bor(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::BXor => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_bxor(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Shl => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_shl(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Shr => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_shr(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::BAndK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_bandk(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::BOrK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_bork(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::BXorK => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_bxork(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::ShrI => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_shri(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::ShlI => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_shli(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::BNot => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_bnot(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ Unary operations ============
            OpCode::Not => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_not(vm, instr, frame_ptr);
                continue 'mainloop;
            }

            // ============ Metamethod stubs (skip, handled by previous instruction) ============
            OpCode::MmBin => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_mmbin(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::MmBinI => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_mmbini(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::MmBinK => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_mmbink(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ Comparisons (may skip next instruction) ============
            OpCode::LtI => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_lti(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::LeI => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_lei(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::GtI => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_gti(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::GeI => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_gei(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::EqI => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_eqi(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::EqK => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_eqk(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }

            // ============ Control Flow (inline for speed) ============
            OpCode::Jmp => {
                let sj = Instruction::get_sj(instr);
                pc = (pc as i32 + sj) as usize;
                continue 'mainloop;
            }
            OpCode::Test => {
                let a = Instruction::get_a(instr) as usize;
                let k = Instruction::get_k(instr);
                unsafe {
                    let val = *vm.register_stack.as_ptr().add(base_ptr + a);
                    let is_truthy = val.is_truthy();
                    // If (not value) == k, skip next instruction
                    if !is_truthy == k {
                        pc += 1; // Skip next instruction
                    }
                }
                continue 'mainloop;
            }
            OpCode::TestSet => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_testset(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }

            // ============ Loop Instructions (hot path inline) ============
            OpCode::ForPrep => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_forprep(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::ForLoop => {
                let a = Instruction::get_a(instr) as usize;
                let bx = Instruction::get_bx(instr) as usize;
                unsafe {
                    let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr + a);
                    let step = *reg_base.add(2);
                    
                    // Integer loop (vast majority of cases)
                    if step.primary == TAG_INTEGER {
                        let count = (*reg_base.add(1)).secondary;
                        if count > 0 {
                            let idx = (*reg_base).secondary as i64;
                            let step_i = step.secondary as i64;
                            let new_idx = idx.wrapping_add(step_i);
                            (*reg_base.add(1)).secondary = count - 1;
                            (*reg_base).secondary = new_idx as u64;
                            (*reg_base.add(3)).secondary = new_idx as u64;
                            pc -= bx;
                        }
                        continue 'mainloop;
                    }
                    // Float loop - use existing function
                    (*frame_ptr).pc = pc;
                    let _ = exec_forloop(vm, instr, frame_ptr);
                    pc = (*frame_ptr).pc;
                }
                continue 'mainloop;
            }
            OpCode::TForPrep => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_tforprep(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::TForLoop => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_tforloop(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }

            // ============ Upvalue operations ============
            OpCode::GetUpval => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_getupval(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::SetUpval => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_setupval(vm, instr, frame_ptr);
                continue 'mainloop;
            }

            // ============ Extra arg (no-op) ============
            OpCode::ExtraArg => {
                continue 'mainloop;
            }

            // ============ Return Instructions (reload state after frame change) ============
            OpCode::Return0 => {
                unsafe { (*frame_ptr).pc = pc; }
                match exec_return0(vm, instr, &mut frame_ptr) {
                    Ok(()) => {
                        // Reload state from new frame
                        unsafe {
                            pc = (*frame_ptr).pc;
                            code_ptr = (*frame_ptr).code_ptr;
                            base_ptr = (*frame_ptr).base_ptr;
                        }
                        continue 'mainloop;
                    }
                    Err(LuaError::Exit) => return Err(LuaError::Exit),
                    Err(e) => return Err(e),
                }
            }
            OpCode::Return1 => {
                unsafe { (*frame_ptr).pc = pc; }
                match exec_return1(vm, instr, &mut frame_ptr) {
                    Ok(()) => {
                        unsafe {
                            pc = (*frame_ptr).pc;
                            code_ptr = (*frame_ptr).code_ptr;
                            base_ptr = (*frame_ptr).base_ptr;
                        }
                        continue 'mainloop;
                    }
                    Err(LuaError::Exit) => return Err(LuaError::Exit),
                    Err(e) => return Err(e),
                }
            }
            OpCode::Return => {
                unsafe { (*frame_ptr).pc = pc; }
                match exec_return(vm, instr, &mut frame_ptr) {
                    Ok(()) => {
                        unsafe {
                            pc = (*frame_ptr).pc;
                            code_ptr = (*frame_ptr).code_ptr;
                            base_ptr = (*frame_ptr).base_ptr;
                        }
                        continue 'mainloop;
                    }
                    Err(LuaError::Exit) => return Err(LuaError::Exit),
                    Err(e) => return Err(e),
                }
            }

            // ============ Function calls (reload state after frame change) ============
            OpCode::Call => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_call(vm, instr, &mut frame_ptr) {
                    return Err(e);
                }
                // Reload state from new frame
                unsafe {
                    pc = (*frame_ptr).pc;
                    code_ptr = (*frame_ptr).code_ptr;
                    base_ptr = (*frame_ptr).base_ptr;
                }
                continue 'mainloop;
            }
            OpCode::TailCall => {
                unsafe { (*frame_ptr).pc = pc; }
                match exec_tailcall(vm, instr, &mut frame_ptr) {
                    Ok(()) => {
                        unsafe {
                            pc = (*frame_ptr).pc;
                            code_ptr = (*frame_ptr).code_ptr;
                            base_ptr = (*frame_ptr).base_ptr;
                        }
                        continue 'mainloop;
                    }
                    Err(LuaError::Exit) => return Err(LuaError::Exit),
                    Err(e) => return Err(e),
                }
            }

            // ============ Table operations (can trigger metamethods) ============
            OpCode::NewTable => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_newtable(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::GetTable => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_gettable(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetTable => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_settable(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetI => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_geti(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetI => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_seti(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetField => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_getfield(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetField => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_setfield(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetTabUp => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_gettabup(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetTabUp => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_settabup(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Self_ => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_self(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ Operations that can trigger metamethods ============
            OpCode::Unm => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_unm(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Len => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_len(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Concat => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_concat(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Eq => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_eq(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Lt => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_lt(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Le => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_le(vm, instr, frame_ptr) {
                    return Err(e);
                }
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }

            // ============ TForCall ============
            OpCode::TForCall => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_tforcall(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ Closure and special ============
            OpCode::Closure => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_closure(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Vararg => {
                unsafe { (*frame_ptr).pc = pc; }
                if let Err(e) = exec_vararg(vm, instr, frame_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetList => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_setlist(vm, instr, frame_ptr);
                unsafe { pc = (*frame_ptr).pc; }
                continue 'mainloop;
            }
            OpCode::Close => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_close(vm, instr, frame_ptr);
                continue 'mainloop;
            }
            OpCode::Tbc => {
                unsafe { (*frame_ptr).pc = pc; }
                exec_tbc(vm, instr, frame_ptr);
                continue 'mainloop;
            }
        }
    }
}
