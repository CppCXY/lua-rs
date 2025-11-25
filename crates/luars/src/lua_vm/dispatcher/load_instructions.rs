/// Load and Move instructions
///
/// These instructions handle loading constants and moving values between registers.
use crate::LuaValue;
use crate::lua_vm::{Instruction, LuaCallFrame, LuaResult, LuaVM};

/// VARARGPREP A
/// Prepare stack for vararg function
/// A is the number of fixed parameters
pub fn exec_varargprep(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;

    let frame_idx = vm.frames.len() - 1;
    let frame = &vm.frames[frame_idx];
    let base_ptr = frame.base_ptr;
    let top = frame.top;

    // Get max_stack_size from the function
    let max_stack_size = if let Some(func_ref) = frame.function_value.as_lua_function() {
        func_ref.borrow().chunk.max_stack_size
    } else {
        return Err(vm.error("Invalid function in VARARGPREP".to_string()));
    };

    // Arguments were placed starting at base_ptr by CALL instruction
    // Fixed parameters are at base_ptr + 0 to base_ptr + a - 1
    // Extra arguments (varargs) are at base_ptr + a to base_ptr + top - 1
    // We need to move the varargs to base_ptr + max_stack_size

    if top > a {
        let vararg_count = top - a;
        let vararg_dest = base_ptr + max_stack_size;

        // Ensure we have enough space for the varargs
        vm.ensure_stack_capacity(vararg_dest + vararg_count);

        // Move varargs from base_ptr + a to base_ptr + max_stack_size
        for i in 0..vararg_count {
            vm.register_stack[vararg_dest + i] = vm.register_stack[base_ptr + a + i].clone();
        }

        vm.frames[frame_idx].set_vararg(vararg_dest, vararg_count);
    } else {
        vm.frames[frame_idx].set_vararg(base_ptr + max_stack_size, 0);
    }

    Ok(())
}

/// LOADNIL A B
/// R[A], R[A+1], ..., R[A+B] := nil
pub fn exec_loadnil(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    for i in 0..=b {
        vm.register_stack[base_ptr + a + i] = LuaValue::nil();
    }

    Ok(())
}

/// LOADFALSE A
/// R[A] := false
pub fn exec_loadfalse(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::boolean(false);

    Ok(())
}

/// LFALSESKIP A
/// R[A] := false; pc++
pub fn exec_lfalseskip(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::boolean(false);

    // Skip next instruction
    vm.current_frame_mut().pc += 1;

    Ok(())
}

/// LOADTRUE A
/// R[A] := true
pub fn exec_loadtrue(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::boolean(true);

    Ok(())
}

/// LOADI A sBx
/// R[A] := sBx (signed integer)
#[inline(always)]
pub fn exec_loadi(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sbx = Instruction::get_sbx(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(sbx as i64);
    }

    Ok(())
}

/// LOADF A sBx
/// R[A] := (lua_Number)sBx
pub fn exec_loadf(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sbx = Instruction::get_sbx(instr);
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::number(sbx as f64);

    Ok(())
}

/// LOADK A Bx
/// R[A] := K[Bx]
#[inline(always)]
pub fn exec_loadk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let bx = Instruction::get_bx(instr) as usize;

    let frame = vm.current_frame();
    let Some(func_ref) = frame.get_lua_function() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    let len = func_ref.borrow().chunk.constants.len();
    if bx >= len {
        return Err(vm.error(format!("Constant index out of bounds: {} >= {}", bx, len)));
    }

    let constant = func_ref.borrow().chunk.constants[bx];
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = constant;

    Ok(())
}

/// LOADKX A
/// R[A] := K[extra arg]
pub fn exec_loadkx(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;

    // Next instruction contains the constant index
    let frame = vm.current_frame_mut();
    let pc = frame.pc;
    frame.pc += 1; // Skip the extra arg instruction

    let Some(func_ref) = frame.get_lua_function() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    let code_len = func_ref.borrow().chunk.code.len();

    if pc >= code_len {
        return Err(vm.error("Missing EXTRAARG for LOADKX".to_string()));
    }

    let extra_instr = func_ref.borrow().chunk.code[pc];
    let bx = Instruction::get_ax(extra_instr) as usize;
    let const_len = func_ref.borrow().chunk.constants.len();
    if bx >= const_len {
        return Err(vm.error(format!(
            "Constant index out of bounds: {} >= {}",
            bx, const_len
        )));
    }

    let constant = func_ref.borrow().chunk.constants[bx];
    let base_ptr = vm.current_frame().base_ptr;

    vm.register_stack[base_ptr + a] = constant;

    Ok(())
}

/// ExtraArg: Extra argument for LOADKX and other instructions
pub fn exec_extraarg(vm: &mut LuaVM, _instr: u32) -> LuaResult<()> {
    // EXTRAARG is consumed by the preceding instruction (like LOADKX)
    // It should never be executed directly
    Err(vm.error("EXTRAARG should not be executed directly".to_string()))
}

/// MOVE A B
/// R[A] := R[B]
#[inline(always)]
pub fn exec_move(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
        let reg_ptr = vm.register_stack.as_mut_ptr().add(base_ptr);
        *reg_ptr.add(a) = *reg_ptr.add(b);
    }

    Ok(())
}
