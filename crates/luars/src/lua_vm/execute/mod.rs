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
use crate::lua_value::TAG_INTEGER;
use crate::lua_vm::LuaCallFrame;

/// Save current pc to frame (like Lua C's savepc macro)
/// Called before operations that may call Lua functions (CALL, metamethods, etc.)
macro_rules! savepc {
    ($frame_ptr:expr, $pc:expr) => {
        unsafe {
            (*$frame_ptr).pc = $pc;
        }
    };
}

/// Update pc, code_ptr and base_ptr from frame (like Lua C's updatestate)
/// Used after CALL/RETURN instructions when frame changes
#[inline(always)]
unsafe fn updatestate(
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    code_ptr: &mut *const u32,
    base_ptr: &mut usize,
) {
    unsafe {
        *pc = (*frame_ptr).pc;
        *code_ptr = (*frame_ptr).code_ptr;
        *base_ptr = (*frame_ptr).base_ptr;
    }
}

/// Ultra-optimized main execution loop
///
/// Key optimizations (like Lua C):
/// 1. Local variables: pc, code_ptr, base_ptr cached locally (like Lua C's luaV_execute)
/// 2. Hot path instructions inlined directly in match
/// 3. State reload only when frame changes (CALL/RETURN)
/// 4. Pass mutable references to avoid frequent frame_ptr writes
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
            // ============ HOT PATH: Inline simple instructions (< 10 lines) ============

            // MOVE - R[A] := R[B]
            OpCode::Move => {
                let a = Instruction::get_a(instr) as usize;
                let b = Instruction::get_b(instr) as usize;
                unsafe {
                    let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr);
                    *reg_base.add(a) = *reg_base.add(b);
                }
                continue 'mainloop;
            }

            // LOADI - R[A] := sBx
            OpCode::LoadI => {
                let a = Instruction::get_a(instr) as usize;
                let sbx = Instruction::get_sbx(instr);
                unsafe {
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) =
                        LuaValue::integer(sbx as i64);
                }
                continue 'mainloop;
            }

            // LOADF - R[A] := (float)sBx
            OpCode::LoadF => {
                let a = Instruction::get_a(instr) as usize;
                let sbx = Instruction::get_sbx(instr);
                unsafe {
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) =
                        LuaValue::number(sbx as f64);
                }
                continue 'mainloop;
            }

            // LOADTRUE - R[A] := true
            OpCode::LoadTrue => {
                let a = Instruction::get_a(instr) as usize;
                unsafe {
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::boolean(true);
                }
                continue 'mainloop;
            }

            // LOADFALSE - R[A] := false
            OpCode::LoadFalse => {
                let a = Instruction::get_a(instr) as usize;
                unsafe {
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::boolean(false);
                }
                continue 'mainloop;
            }

            // LFALSESKIP - R[A] := false; pc++
            OpCode::LFalseSkip => {
                let a = Instruction::get_a(instr) as usize;
                unsafe {
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::boolean(false);
                }
                pc += 1;
                continue 'mainloop;
            }

            // LOADNIL - R[A], ..., R[A+B] := nil
            OpCode::LoadNil => {
                let a = Instruction::get_a(instr) as usize;
                let b = Instruction::get_b(instr) as usize;
                unsafe {
                    let nil_val = LuaValue::nil();
                    let reg_ptr = vm.register_stack.as_mut_ptr().add(base_ptr);
                    for i in 0..=b {
                        *reg_ptr.add(a + i) = nil_val;
                    }
                }
                continue 'mainloop;
            }

            // LOADK - R[A] := K[Bx]
            OpCode::LoadK => {
                let a = Instruction::get_a(instr) as usize;
                let bx = Instruction::get_bx(instr) as usize;
                unsafe {
                    let constant = *(*frame_ptr).constants_ptr.add(bx);
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) = constant;
                }
                continue 'mainloop;
            }

            // LOADKX - R[A] := K[extra arg]; pc++
            OpCode::LoadKX => {
                let a = Instruction::get_a(instr) as usize;
                unsafe {
                    let extra_instr = *code_ptr.add(pc);
                    pc += 1;
                    let bx = Instruction::get_ax(extra_instr) as usize;
                    let constant = *(*frame_ptr).constants_ptr.add(bx);
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) = constant;
                }
                continue 'mainloop;
            }

            // VARARGPREP - complex, call function
            OpCode::VarargPrep => {
                exec_varargprep(vm, instr, frame_ptr, &mut base_ptr);
                continue 'mainloop;
            }

            // ============ Arithmetic operations ============
            OpCode::Add => {
                exec_add(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::Sub => {
                exec_sub(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::Mul => {
                exec_mul(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::AddI => {
                exec_addi(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::Div => {
                exec_div(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::IDiv => {
                exec_idiv(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::Mod => {
                exec_mod(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::Pow => {
                exec_pow(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }

            // Arithmetic with constants
            OpCode::AddK => {
                exec_addk(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::SubK => {
                exec_subk(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::MulK => {
                exec_mulk(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::ModK => {
                exec_modk(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::PowK => {
                exec_powk(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::DivK => {
                exec_divk(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::IDivK => {
                exec_idivk(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }

            // ============ Bitwise (inline simple ones) ============
            OpCode::BAnd => {
                exec_band(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::BOr => {
                exec_bor(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::BXor => {
                exec_bxor(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::Shl => {
                exec_shl(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::Shr => {
                exec_shr(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::BAndK => {
                exec_bandk(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::BOrK => {
                exec_bork(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::BXorK => {
                exec_bxork(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::ShrI => {
                exec_shri(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::ShlI => {
                exec_shli(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::BNot => {
                if let Err(e) = exec_bnot(vm, instr, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ Unary operations (inline NOT) ============
            OpCode::Not => {
                let a = Instruction::get_a(instr) as usize;
                let b = Instruction::get_b(instr) as usize;
                unsafe {
                    let value = *vm.register_stack.as_ptr().add(base_ptr + b);
                    let is_falsy = !value.is_truthy();
                    *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::boolean(is_falsy);
                }
                continue 'mainloop;
            }

            // ============ Metamethod stubs (save pc before calling) ============
            OpCode::MmBin => {
                savepc!(frame_ptr, pc);
                if let Err(e) = exec_mmbin(vm, instr, code_ptr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::MmBinI => {
                savepc!(frame_ptr, pc);
                if let Err(e) = exec_mmbini(vm, instr, code_ptr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::MmBinK => {
                savepc!(frame_ptr, pc);
                let constants_ptr = unsafe { (*frame_ptr).constants_ptr };
                if let Err(e) = exec_mmbink(vm, instr, code_ptr, constants_ptr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ Comparisons (inline simple ones) ============
            OpCode::LtI => {
                if let Err(e) = exec_lti(vm, instr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::LeI => {
                if let Err(e) = exec_lei(vm, instr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GtI => {
                if let Err(e) = exec_gti(vm, instr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GeI => {
                if let Err(e) = exec_gei(vm, instr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::EqI => {
                exec_eqi(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::EqK => {
                if let Err(e) = exec_eqk(vm, instr, frame_ptr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ Control Flow (inline JMP and TEST) ============
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
                    if !is_truthy == k {
                        pc += 1;
                    }
                }
                continue 'mainloop;
            }
            OpCode::TestSet => {
                exec_testset(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }

            // ============ Loop Instructions (inline FORLOOP) ============
            OpCode::ForPrep => {
                if let Err(e) = exec_forprep(vm, instr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::ForLoop => {
                let a = Instruction::get_a(instr) as usize;
                let bx = Instruction::get_bx(instr) as usize;
                unsafe {
                    let reg_base = vm.register_stack.as_mut_ptr().add(base_ptr + a);
                    let step = *reg_base.add(2);

                    // Integer loop (like Lua C)
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
                    // Float loop
                    if let Err(e) = exec_forloop_float(vm, reg_base, bx, &mut pc) {
                        return Err(e);
                    }
                }
                continue 'mainloop;
            }
            OpCode::TForPrep => {
                exec_tforprep(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::TForLoop => {
                exec_tforloop(vm, instr, &mut pc, base_ptr);
                continue 'mainloop;
            }

            // ============ Upvalue operations (inline simple ones) ============
            OpCode::GetUpval => {
                exec_getupval(vm, instr, frame_ptr, base_ptr);
                continue 'mainloop;
            }
            OpCode::SetUpval => {
                exec_setupval(vm, instr, frame_ptr, base_ptr);
                continue 'mainloop;
            }

            // ============ Extra arg (no-op) ============
            OpCode::ExtraArg => {
                continue 'mainloop;
            }

            // ============ Return Instructions (update state after frame change) ============
            OpCode::Return0 => {
                match exec_return0(vm, instr, &mut frame_ptr) {
                    Ok(()) => {
                        // Reload state from new frame (like Lua C)
                        unsafe {
                            updatestate(frame_ptr, &mut pc, &mut code_ptr, &mut base_ptr);
                        }
                        continue 'mainloop;
                    }
                    Err(LuaError::Exit) => return Err(LuaError::Exit),
                    Err(e) => return Err(e),
                }
            }
            OpCode::Return1 => match exec_return1(vm, instr, &mut frame_ptr) {
                Ok(()) => {
                    unsafe {
                        updatestate(frame_ptr, &mut pc, &mut code_ptr, &mut base_ptr);
                    }
                    continue 'mainloop;
                }
                Err(LuaError::Exit) => return Err(LuaError::Exit),
                Err(e) => return Err(e),
            },
            OpCode::Return => match exec_return(vm, instr, &mut frame_ptr) {
                Ok(()) => {
                    unsafe {
                        updatestate(frame_ptr, &mut pc, &mut code_ptr, &mut base_ptr);
                    }
                    continue 'mainloop;
                }
                Err(LuaError::Exit) => return Err(LuaError::Exit),
                Err(e) => return Err(e),
            },

            // ============ Function calls (update state after frame change) ============
            OpCode::Call => {
                // Save current pc to frame before call (so return knows where to resume)
                savepc!(frame_ptr, pc);
                if let Err(e) = exec_call(vm, instr, &mut frame_ptr) {
                    return Err(e);
                }
                // Reload state from new frame
                unsafe {
                    updatestate(frame_ptr, &mut pc, &mut code_ptr, &mut base_ptr);
                }
                continue 'mainloop;
            }
            OpCode::TailCall => {
                // Save current pc before tail call
                savepc!(frame_ptr, pc);
                match exec_tailcall(vm, instr, &mut frame_ptr) {
                    Ok(()) => {
                        unsafe {
                            updatestate(frame_ptr, &mut pc, &mut code_ptr, &mut base_ptr);
                        }
                        continue 'mainloop;
                    }
                    Err(LuaError::Exit) => return Err(LuaError::Exit),
                    Err(e) => return Err(e),
                }
            }

            // ============ Table operations ============
            OpCode::NewTable => {
                exec_newtable(vm, instr, frame_ptr, &mut pc, base_ptr);
                continue 'mainloop;
            }
            OpCode::GetTable => {
                if let Err(e) = exec_gettable(vm, instr, frame_ptr, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetTable => {
                if let Err(e) = exec_settable(vm, instr, frame_ptr, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetI => {
                if let Err(e) = exec_geti(vm, instr, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetI => {
                if let Err(e) = exec_seti(vm, instr, frame_ptr, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetField => {
                if let Err(e) = exec_getfield(vm, instr, frame_ptr, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetField => {
                if let Err(e) = exec_setfield(vm, instr, frame_ptr, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::GetTabUp => {
                if let Err(e) = exec_gettabup(vm, instr, frame_ptr, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetTabUp => {
                if let Err(e) = exec_settabup(vm, instr, frame_ptr, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Self_ => {
                if let Err(e) = exec_self(vm, instr, frame_ptr, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ Operations that can trigger metamethods ============
            OpCode::Unm => {
                if let Err(e) = exec_unm(vm, instr, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Len => {
                if let Err(e) = exec_len(vm, instr, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Concat => {
                if let Err(e) = exec_concat(vm, instr, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Eq => {
                if let Err(e) = exec_eq(vm, instr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Lt => {
                if let Err(e) = exec_lt(vm, instr, &mut pc, &mut base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Le => {
                if let Err(e) = exec_le(vm, instr, &mut pc, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }

            // ============ TForCall ============
            OpCode::TForCall => {
                savepc!(frame_ptr, pc);
                match exec_tforcall(vm, instr, &mut frame_ptr, base_ptr) {
                    Ok(true) => {
                        // Lua function called, need to update state
                        unsafe {
                            updatestate(frame_ptr, &mut pc, &mut code_ptr, &mut base_ptr);
                        }
                    }
                    Ok(false) => {
                        // C function called, no state change needed
                    }
                    Err(e) => return Err(e),
                }
                continue 'mainloop;
            }

            // ============ Closure and special ============
            OpCode::Closure => {
                if let Err(e) = exec_closure(vm, instr, frame_ptr, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::Vararg => {
                if let Err(e) = exec_vararg(vm, instr, frame_ptr, base_ptr) {
                    return Err(e);
                }
                continue 'mainloop;
            }
            OpCode::SetList => {
                exec_setlist(vm, instr, frame_ptr, base_ptr);
                continue 'mainloop;
            }
            OpCode::Close => {
                exec_close(vm, instr, base_ptr);
                continue 'mainloop;
            }
            OpCode::Tbc => {
                exec_tbc(vm, instr, base_ptr);
                continue 'mainloop;
            }
        }
    }
}
