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
    
    let frame_idx = vm.frames.len() - 1;
    let frame = &vm.frames[frame_idx];
    let base_ptr = frame.base_ptr;
    let top = frame.top;
    
    // For vararg functions, arguments are stored BEYOND the normal register space
    // The CALL instruction places them at base_ptr + max_stack_size
    // We need to calculate where they are based on the actual stack layout
    
    // Get max_stack_size from the function
    let max_stack_size = if let Some(func_ref) = frame.function_value.as_lua_function() {
        func_ref.borrow().chunk.max_stack_size
    } else {
        return Err(LuaError::RuntimeError("Invalid function in VARARGPREP".to_string()));
    };
    
    // Varargs were placed at base_ptr + max_stack_size by CALL
    // The number of varargs is determined by 'top' which was set to arg_count
    if top > a {
        let vararg_count = top - a;
        let vararg_start = base_ptr + max_stack_size;
        vm.frames[frame_idx].set_vararg(vararg_start, vararg_count);
    } else {
        vm.frames[frame_idx].set_vararg(base_ptr + max_stack_size, 0);
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

/// LFALSESKIP A
/// R[A] := false; pc++
pub fn exec_lfalseskip(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::boolean(false);
    
    // Skip next instruction
    vm.current_frame_mut().pc += 1;

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
#[inline(always)]
pub fn exec_loadi(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sbx = Instruction::get_sbx(instr);
    let base_ptr = vm.current_frame().base_ptr;

    unsafe {
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(sbx as i64);
    }

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
#[inline(always)]
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

/// ExtraArg: Extra argument for LOADKX and other instructions
pub fn exec_extraarg(_vm: &mut LuaVM, _instr: u32) -> Result<DispatchAction, LuaError> {
    // EXTRAARG is consumed by the preceding instruction (like LOADKX)
    // It should never be executed directly
    Err(LuaError::RuntimeError(
        "EXTRAARG should not be executed directly".to_string()
    ))
}

/// MOVE A B
/// R[A] := R[B]
#[inline(always)]
pub fn exec_move(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for direct register access
    unsafe {
        let reg_ptr = vm.register_stack.as_mut_ptr().add(base_ptr);
        *reg_ptr.add(a) = *reg_ptr.add(b);
    }

    Ok(DispatchAction::Continue)
}
