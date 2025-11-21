/// Table operations
/// 
/// These instructions handle table creation, access, and manipulation.

use crate::lua_vm::{LuaVM, LuaResult, LuaError, Instruction};
use super::DispatchAction;

/// NEWTABLE A B C k
/// R[A] := {} (size = B,C)
pub fn exec_newtable(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr);
    let c = Instruction::get_c(instr);
    let _k = Instruction::get_k(instr);

    let frame = vm.current_frame_mut();
    let base_ptr = frame.base_ptr;
    
    // NEWTABLE is always followed by an EXTRAARG instruction in Lua 5.4
    // PC has already been incremented to point to EXTRAARG
    let pc = frame.pc;
    frame.pc += 1; // Skip the EXTRAARG instruction
    
    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    let chunk = &func_ref.chunk;
    
    let extra_arg = if pc < chunk.code.len() {
        Instruction::get_ax(chunk.code[pc])
    } else {
        0
    };

    // Calculate array size hint and hash size hint
    // These are size hints for preallocation (we currently ignore them)
    let _array_size = if b > 0 { b - 1 } else { extra_arg };
    let _hash_size = c;

    // Create new table (ignore size hints for now)
    let table = vm.create_table();
    vm.register_stack[base_ptr + a] = table;

    Ok(DispatchAction::Continue)
}

/// GETTABLE A B C
/// R[A] := R[B][R[C]]
pub fn exec_gettable(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let table = vm.register_stack[base_ptr + b];
    let key = vm.register_stack[base_ptr + c];

    let value = vm.table_get_with_meta(&table, &key).unwrap_or(crate::LuaValue::nil());
    vm.register_stack[base_ptr + a] = value;

    Ok(DispatchAction::Continue)
}

/// SETTABLE A B C k
/// R[A][R[B]] := RK(C)
pub fn exec_settable(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let table = vm.register_stack[base_ptr + a];
    let key = vm.register_stack[base_ptr + b];
    
    let value = if k {
        // K[C]
        let func_ptr = frame.get_function_ptr().ok_or_else(|| {
            LuaError::RuntimeError("Not a Lua function".to_string())
        })?;
        let func = unsafe { &*func_ptr };
        func.borrow().chunk.constants.get(c).copied().ok_or_else(|| {
            LuaError::RuntimeError(format!("Invalid constant index: {}", c))
        })?
    } else {
        // R[C]
        vm.register_stack[base_ptr + c]
    };

    vm.table_set_with_meta(table, key, value)?;

    Ok(DispatchAction::Continue)
}

/// GETI A B C
/// R[A] := R[B][C]
pub fn exec_geti(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let table = vm.register_stack[base_ptr + b];
    let key = crate::LuaValue::integer(c as i64);

    let value = vm.table_get_with_meta(&table, &key).unwrap_or(crate::LuaValue::nil());
    vm.register_stack[base_ptr + a] = value;

    Ok(DispatchAction::Continue)
}

/// SETI A B C k
/// R[A][B] := RK(C)
pub fn exec_seti(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let table = vm.register_stack[base_ptr + a];
    let key = crate::LuaValue::integer(b as i64);
    
    let value = if k {
        let func_ptr = frame.get_function_ptr().ok_or_else(|| {
            LuaError::RuntimeError("Not a Lua function".to_string())
        })?;
        let func = unsafe { &*func_ptr };
        func.borrow().chunk.constants.get(c).copied().ok_or_else(|| {
            LuaError::RuntimeError(format!("Invalid constant index: {}", c))
        })?
    } else {
        vm.register_stack[base_ptr + c]
    };

    vm.table_set_with_meta(table, key, value)?;

    Ok(DispatchAction::Continue)
}

/// GETFIELD A B C
/// R[A] := R[B][K[C]:string]
pub fn exec_getfield(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let key = func.borrow().chunk.constants.get(c).copied().ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid constant index: {}", c))
    })?;

    let table = vm.register_stack[base_ptr + b];

    let value = vm.table_get_with_meta(&table, &key).unwrap_or(crate::LuaValue::nil());
    vm.register_stack[base_ptr + a] = value;

    Ok(DispatchAction::Continue)
}

/// SETFIELD A B C k
/// R[A][K[B]:string] := RK(C)
pub fn exec_setfield(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let key = func.borrow().chunk.constants.get(b).copied().ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid constant index: {}", b))
    })?;

    let table = vm.register_stack[base_ptr + a];
    
    let value = if k {
        func.borrow().chunk.constants.get(c).copied().ok_or_else(|| {
            LuaError::RuntimeError(format!("Invalid constant index: {}", c))
        })?
    } else {
        vm.register_stack[base_ptr + c]
    };

    vm.table_set_with_meta(table, key, value)?;

    Ok(DispatchAction::Continue)
}

/// GETTABUP A B C
/// R[A] := UpValue[B][K[C]:string]
pub fn exec_gettabup(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    
    let key = func_ref.chunk.constants.get(c).copied().ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid constant index: {}", c))
    })?;

    eprintln!("[exec_gettabup] A={}, B={} (upvalue), C={} (constant)", a, b, c);
    eprintln!("[exec_gettabup] Key: {:?}", key);
    eprintln!("[exec_gettabup] Function has {} upvalues", func_ref.upvalues.len());
    
    let upvalue = func_ref.upvalues.get(b).ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid upvalue index: {}", b))
    })?;

    let table = upvalue.get_value(&vm.frames, &vm.register_stack);
    eprintln!("[exec_gettabup] Table type: {:?}", table.kind());

    let value = vm.table_get_with_meta(&table, &key).unwrap_or(crate::LuaValue::nil());
    eprintln!("[exec_gettabup] Result value type: {:?}", value.kind());
    
    vm.register_stack[base_ptr + a] = value;

    Ok(DispatchAction::Continue)
}

/// SETTABUP A B C k
/// UpValue[A][K[B]:string] := RK(C)
pub fn exec_settabup(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    
    let key = func_ref.chunk.constants.get(b).copied().ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid constant index: {}", b))
    })?;

    let upvalue = func_ref.upvalues.get(a).ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid upvalue index: {}", a))
    })?;

    let table = upvalue.get_value(&vm.frames, &vm.register_stack);
    
    let value = if k {
        func_ref.chunk.constants.get(c).copied().ok_or_else(|| {
            LuaError::RuntimeError(format!("Invalid constant index: {}", c))
        })?
    } else {
        vm.register_stack[base_ptr + c]
    };

    vm.table_set_with_meta(table, key, value)?;

    Ok(DispatchAction::Continue)
}

/// SELF A B C
/// R[A+1] := R[B]; R[A] := R[B][RK(C):string]
pub fn exec_self(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let table = vm.register_stack[base_ptr + b];
    
    // Get method key from constant
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let key = func.borrow().chunk.constants.get(c).copied().ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid constant index: {}", c))
    })?;

    // R[A+1] := R[B] (self parameter)
    vm.register_stack[base_ptr + a + 1] = table;
    
    // R[A] := R[B][K[C]] (method)
    let method = vm.table_get_with_meta(&table, &key).unwrap_or(crate::LuaValue::nil());
    vm.register_stack[base_ptr + a] = method;

    Ok(DispatchAction::Continue)
}
