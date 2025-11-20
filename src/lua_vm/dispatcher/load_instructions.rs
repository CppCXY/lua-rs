/// Load and Move instructions
/// 
/// These instructions handle loading constants and moving values between registers.

use crate::LuaValue;
use crate::lua_vm::{LuaVM, LuaResult, LuaError, Instruction};
use super::DispatchAction;

/// VARARGPREP A
/// Prepare stack for vararg function
/// A is the number of fixed parameters
pub fn exec_varargprep(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    
    let frame = vm.current_frame_mut();
    let base_ptr = frame.base_ptr;
    let top = frame.top;
    
    // varargs start after the fixed parameters
    // If we have more values on stack than fixed params, those are varargs
    if top > a {
        let vararg_start = base_ptr + a;
        let vararg_count = top - a;
        frame.set_vararg(vararg_start, vararg_count);
    } else {
        frame.set_vararg(base_ptr + a, 0);
    }
    
    Ok(DispatchAction::Continue)
}

/// LOADNIL A B
/// R[A], R[A+1], ..., R[A+B] := nil
pub fn exec_loadnil(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    for i in 0..=b {
        vm.register_stack[base_ptr + a + i] = LuaValue::nil();
    }

    Ok(DispatchAction::Continue)
}

/// LOADFALSE A
/// R[A] := false
pub fn exec_loadfalse(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::boolean(false);

    Ok(DispatchAction::Continue)
}

/// LOADTRUE A
/// R[A] := true
pub fn exec_loadtrue(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::boolean(true);

    Ok(DispatchAction::Continue)
}

/// LOADI A sBx
/// R[A] := sBx (signed integer)
pub fn exec_loadi(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sbx = Instruction::get_sbx(instr);
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::integer(sbx as i64);

    Ok(DispatchAction::Continue)
}

/// LOADF A sBx
/// R[A] := (lua_Number)sBx
pub fn exec_loadf(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sbx = Instruction::get_sbx(instr);
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::number(sbx as f64);

    Ok(DispatchAction::Continue)
}

/// LOADK A Bx
/// R[A] := K[Bx]
pub fn exec_loadk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;

    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    let chunk = &func_ref.chunk;

    if bx >= chunk.constants.len() {
        return Err(LuaError::RuntimeError(format!(
            "Constant index out of bounds: {} >= {}",
            bx,
            chunk.constants.len()
        )));
    }

    let constant = chunk.constants[bx];
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = constant;

    Ok(DispatchAction::Continue)
}

/// LOADKX A
/// R[A] := K[extra arg]
pub fn exec_loadkx(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;

    // Next instruction contains the constant index
    let frame = vm.current_frame_mut();
    let pc = frame.pc;
    frame.pc += 1; // Skip the extra arg instruction

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;

    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    let chunk = &func_ref.chunk;

    if pc >= chunk.code.len() {
        return Err(LuaError::RuntimeError("Missing EXTRAARG for LOADKX".to_string()));
    }

    let extra_instr = chunk.code[pc];
    let bx = Instruction::get_ax(extra_instr) as usize;

    if bx >= chunk.constants.len() {
        return Err(LuaError::RuntimeError(format!(
            "Constant index out of bounds: {} >= {}",
            bx,
            chunk.constants.len()
        )));
    }

    let constant = chunk.constants[bx];
    let base_ptr = vm.current_frame().base_ptr;

    vm.register_stack[base_ptr + a] = constant;

    Ok(DispatchAction::Continue)
}

/// MOVE A B
/// R[A] := R[B]
pub fn exec_move(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let value = vm.register_stack[base_ptr + b];
    vm.register_stack[base_ptr + a] = value;

    Ok(DispatchAction::Continue)
}
