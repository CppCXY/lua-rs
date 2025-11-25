use crate::lua_value::LuaValue;
/// Table operations
///
/// These instructions handle table creation, access, and manipulation.
use crate::lua_vm::{Instruction, LuaResult, LuaVM};

/// NEWTABLE A B C k
/// R[A] := {} (size = B,C)
#[inline]
pub fn exec_newtable(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr);
    let c = Instruction::get_c(instr);

    let frame = vm.current_frame_mut();
    let base_ptr = frame.base_ptr;

    // NEWTABLE is always followed by an EXTRAARG instruction in Lua 5.4
    // PC has already been incremented to point to EXTRAARG
    let pc = frame.pc;
    frame.pc += 1; // Skip the EXTRAARG instruction

    // OPTIMIZATION: Inline EXTRAARG reading to avoid function pointer overhead
    let extra_arg = unsafe {
        let func_ptr = frame.get_function_ptr().unwrap_unchecked();
        let func = &*func_ptr;
        let func_ref = func.borrow();
        let chunk = &func_ref.chunk;

        if pc < chunk.code.len() {
            Instruction::get_ax(chunk.code[pc])
        } else {
            0
        }
    };

    // Calculate array size hint and hash size hint
    let array_size = if b > 0 {
        (b - 1) as usize
    } else {
        extra_arg as usize
    };
    let hash_size = c as usize;

    // Create new table with size hints
    let table = vm.create_table(array_size, hash_size);
    vm.register_stack[base_ptr + a] = table;

    // GC checkpoint: table now safely stored in register
    vm.check_gc();

    Ok(())
}

/// GETTABLE A B C
/// R[A] := R[B][R[C]]
#[inline(always)]
pub fn exec_gettable(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    // CRITICAL: Read values BEFORE metamethod calls
    let (table_value, key_value) = {
        let frame = vm.current_frame();
        let base_ptr = frame.base_ptr;
        let table = vm.register_stack[base_ptr + b];
        let key = vm.register_stack[base_ptr + c];
        (table, key)
    };

    // Use table_get_with_meta to support __index metamethod
    let value = vm
        .table_get_with_meta(&table_value, &key_value)
        .unwrap_or(LuaValue::nil());

    // Re-read base_ptr after metamethod call
    let new_base_ptr = vm.current_frame().base_ptr;
    vm.register_stack[new_base_ptr + a] = value;

    Ok(())
}

/// SETTABLE A B C k
/// R[A][R[B]] := RK(C)
#[inline(always)]
pub fn exec_settable(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    // CRITICAL: Read all values BEFORE any metamethod calls
    // because metamethods can modify the register stack
    let (table_value, key_value, set_value) = {
        let frame = vm.current_frame();
        let base_ptr = frame.base_ptr;

        let table = vm.register_stack[base_ptr + a];
        let key = vm.register_stack[base_ptr + b];

        let value = if k {
            // OPTIMIZATION: Get constant directly
            let Some(func_ref) = frame.get_lua_function() else {
                return Err(vm.error("Not a Lua function".to_string()));
            };

            let Some(constant) = func_ref.borrow().chunk.constants.get(c).copied() else {
                return Err(vm.error(format!("Invalid constant index: {}", c)));
            };

            constant
        } else {
            vm.register_stack[base_ptr + c]
        };

        (table, key, value)
    };

    vm.table_set_with_meta(table_value, key_value, set_value)?;

    Ok(())
}

/// GETI A B C
/// R[A] := R[B][C:integer]
#[inline(always)]
pub fn exec_geti(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let table = vm.register_stack[base_ptr + b];

    // FAST PATH: Direct access for tables without metatable
    if let Some(ptr) = table.as_table_ptr() {
        let lua_table = unsafe { &*ptr };
        let borrowed = lua_table.borrow();
        let key = LuaValue::integer(c as i64);
        
        // Try raw_get which checks both array and hash parts
        if let Some(val) = borrowed.raw_get(&key) {
            if !val.is_nil() {
                vm.register_stack[base_ptr + a] = val;
                return Ok(());
            }
        }
        
        // Key not found - check if no metatable to skip metamethod handling
        if borrowed.get_metatable().is_none() {
            vm.register_stack[base_ptr + a] = LuaValue::nil();
            return Ok(());
        }
    }

    // Slow path: Use metamethod handling
    let key = LuaValue::integer(c as i64);
    let value = vm
        .table_get_with_meta(&table, &key)
        .unwrap_or(LuaValue::nil());
    vm.register_stack[base_ptr + a] = value;

    Ok(())
}

/// SETI A B C k
/// R[A][B] := RK(C)
#[inline(always)]
pub fn exec_seti(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_sb(instr);
    let c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    // CRITICAL: Read all values BEFORE any metamethod calls
    // because metamethods can modify the register stack
    let (table_value, key_value, set_value) = {
        let frame = vm.current_frame();
        let base_ptr = frame.base_ptr;

        let table = vm.register_stack[base_ptr + a];
        let key = crate::LuaValue::integer(b as i64);

        let value = if k {
            // OPTIMIZATION: Get constant directly
            let Some(func_ref) = frame.get_lua_function() else {
                return Err(vm.error("Not a Lua function".to_string()));
            };

            let Some(constant) = func_ref.borrow().chunk.constants.get(c).copied() else {
                return Err(vm.error(format!("Invalid constant index: {}", c)));
            };

            constant
        } else {
            vm.register_stack[base_ptr + c]
        };

        (table, key, value)
    };

    // FAST PATH: Direct table access without metamethod check for common case
    if let Some(ptr) = table_value.as_table_ptr() {
        let lua_table = unsafe { &*ptr };
        
        // Quick check: no metatable means no __newindex to worry about
        let has_metatable = lua_table.borrow().get_metatable().is_some();
        
        if !has_metatable {
            // Ultra-fast path: direct set without any metamethod checks
            lua_table.borrow_mut().raw_set(key_value.clone(), set_value.clone());
            
            // GC barrier - only for collectable values (like Lua)
            if crate::gc::GC::is_collectable(&set_value) {
                if let Some(table_id) = table_value.as_table_id() {
                    vm.gc.barrier_forward(crate::gc::GcObjectType::Table, table_id.0);
                    vm.gc.barrier_back(&set_value);
                }
            }
            return Ok(());
        }
    }

    // Slow path: use full metamethod handling
    vm.table_set_with_meta(table_value, key_value, set_value)?;

    Ok(())
}

/// GETFIELD A B C
/// R[A] := R[B][K[C]:string]
#[inline(always)]
pub fn exec_getfield(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let Some(func_ptr) = frame.get_function_ptr() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    // SAFETY: func_ptr is valid for the duration of this block
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();

    let Some(&key_value) = func_ref.chunk.constants.get(c) else {
        drop(func_ref);
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let table_value = vm.register_stack[base_ptr + b];
    drop(func_ref);

    // FAST PATH: Direct hash access for tables without metatable
    if let Some(ptr) = table_value.as_table_ptr() {
        let lua_table = unsafe { &*ptr };
        let borrowed = lua_table.borrow();
        
        // Use optimized hash-only lookup (GETFIELD always uses string keys, never integers)
        if let Some(val) = borrowed.get_from_hash(&key_value) {
            if !val.is_nil() {
                vm.register_stack[base_ptr + a] = val;
                return Ok(());
            }
        }
        
        // Check if no metatable - can return nil directly
        if borrowed.get_metatable().is_none() {
            vm.register_stack[base_ptr + a] = LuaValue::nil();
            return Ok(());
        }
    }

    // Slow path: Use metamethod handling
    let value = vm
        .table_get_with_meta(&table_value, &key_value)
        .unwrap_or(LuaValue::nil());

    // IMPORTANT: Re-read base_ptr after metamethod call in case frames changed
    let new_base_ptr = vm.current_frame().base_ptr;
    vm.register_stack[new_base_ptr + a] = value;

    Ok(())
}

/// SETFIELD A B C k
/// R[A][K[B]:string] := RK(C)
#[inline(always)]
pub fn exec_setfield(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    // CRITICAL: Read all values BEFORE any metamethod calls
    // because metamethods can modify the register stack
    let (table_value, key_value, set_value) = {
        let frame = vm.current_frame();
        let base_ptr = frame.base_ptr;

        let Some(func_ptr) = frame.get_function_ptr() else {
            return Err(vm.error("Not a Lua function".to_string()));
        };

        // SAFETY: func_ptr is valid for the duration of this block
        let func = unsafe { &*func_ptr };
        let func_ref = func.borrow();

        let Some(&key) = func_ref.chunk.constants.get(b) else {
            drop(func_ref);
            return Err(vm.error(format!("Invalid constant index: {}", b)));
        };

        let table = vm.register_stack[base_ptr + a];

        let value = if k {
            let Some(&constant) = func_ref.chunk.constants.get(c) else {
                drop(func_ref);
                return Err(vm.error(format!("Invalid constant index: {}", c)));
            };
            constant
        } else {
            vm.register_stack[base_ptr + c]
        };

        (table, key, value)
    };

    // FAST PATH: Direct table access without metamethod check for common case
    if let Some(ptr) = table_value.as_table_ptr() {
        let lua_table = unsafe { &*ptr };
        
        // Quick check: no metatable means no __newindex to worry about
        let has_metatable = lua_table.borrow().get_metatable().is_some();
        
        if !has_metatable {
            // Ultra-fast path: direct set without any metamethod checks
            lua_table.borrow_mut().raw_set(key_value.clone(), set_value.clone());
            
            // GC barrier - only for collectable values
            if crate::gc::GC::is_collectable(&set_value) {
                if let Some(table_id) = table_value.as_table_id() {
                    vm.gc.barrier_forward(crate::gc::GcObjectType::Table, table_id.0);
                    vm.gc.barrier_back(&set_value);
                }
            }
            return Ok(());
        }
    }

    // Slow path: use full metamethod handling
    vm.table_set_with_meta(table_value, key_value, set_value)?;

    Ok(())
}

/// GETTABUP A B C
/// R[A] := UpValue[B][K[C]:string]
#[inline(always)]
pub fn exec_gettabup(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let Some(func_ptr) = frame.get_function_ptr() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    // SAFETY: func_ptr is valid for the duration of this instruction
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();

    let Some(&key_value) = func_ref.chunk.constants.get(c) else {
        drop(func_ref);
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    let Some(upvalue) = func_ref.upvalues.get(b) else {
        drop(func_ref);
        return Err(vm.error(format!("Invalid upvalue index: {}", b)));
    };

    // FAST PATH: Try to get closed upvalue directly (common case for _ENV)
    let table_value = if let Some(val) = upvalue.try_get_closed() {
        val
    } else {
        // Slow path: open upvalue, need full lookup
        upvalue.get_value(&vm.frames, &vm.register_stack)
    };

    drop(func_ref);

    // FAST PATH: Direct hash access for tables without metatable
    if let Some(ptr) = table_value.as_table_ptr() {
        let lua_table = unsafe { &*ptr };
        let borrowed = lua_table.borrow();
        
        // Try direct access
        if let Some(val) = borrowed.raw_get(&key_value) {
            if !val.is_nil() {
                vm.register_stack[base_ptr + a] = val;
                return Ok(());
            }
        }
        
        // Check if no metatable - can return nil directly
        if borrowed.get_metatable().is_none() {
            vm.register_stack[base_ptr + a] = LuaValue::nil();
            return Ok(());
        }
    }

    // Slow path: Use metamethod handling
    let value = vm
        .table_get_with_meta(&table_value, &key_value)
        .unwrap_or(LuaValue::nil());

    vm.register_stack[base_ptr + a] = value;

    Ok(())
}

/// SETTABUP A B C k
/// UpValue[A][K[B]:string] := RK(C)
#[inline(always)]
pub fn exec_settabup(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let Some(func_ptr) = frame.get_function_ptr() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    // SAFETY: func_ptr is valid for the duration of this instruction
    let func = unsafe { &*func_ptr };
    let func_ref = func.borrow();
    
    let Some(&key_value) = func_ref.chunk.constants.get(b) else {
        drop(func_ref);
        return Err(vm.error(format!("Invalid constant index: {}", b)));
    };

    let Some(upvalue) = func_ref.upvalues.get(a) else {
        drop(func_ref);
        return Err(vm.error(format!("Invalid upvalue index: {}", a)));
    };

    // FAST PATH: Try to get closed upvalue directly (common case for _ENV)
    let table_value = if let Some(val) = upvalue.try_get_closed() {
        val
    } else {
        // Slow path: open upvalue, need full lookup
        upvalue.get_value(&vm.frames, &vm.register_stack)
    };

    let set_value = if k {
        let Some(&constant) = func_ref.chunk.constants.get(c) else {
            drop(func_ref);
            return Err(vm.error(format!("Invalid constant index: {}", c)));
        };
        constant
    } else {
        vm.register_stack[base_ptr + c]
    };

    // Release borrow before table operations
    drop(func_ref);

    // FAST PATH: Direct table access without metamethod check for common case
    if let Some(ptr) = table_value.as_table_ptr() {
        let lua_table = unsafe { &*ptr };
        
        // Quick check: no metatable means no __newindex to worry about
        let has_metatable = lua_table.borrow().get_metatable().is_some();
        
        if !has_metatable {
            // Ultra-fast path: direct set without any metamethod checks
            lua_table.borrow_mut().raw_set(key_value, set_value);
            
            // GC barrier - only for collectable values
            if crate::gc::GC::is_collectable(&set_value) {
                if let Some(table_id) = table_value.as_table_id() {
                    vm.gc.barrier_forward(crate::gc::GcObjectType::Table, table_id.0);
                    vm.gc.barrier_back(&set_value);
                }
            }
            return Ok(());
        }
    }

    // Slow path: use full metamethod handling
    vm.table_set_with_meta(table_value, key_value, set_value)?;

    Ok(())
}

/// SELF A B C
/// R[A+1] := R[B]; R[A] := R[B][RK(C):string]
pub fn exec_self(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let table = vm.register_stack[base_ptr + b];

    // Get method key from constant
    let Some(func_ref) = frame.get_lua_function() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };

    let Some(key) = func_ref.borrow().chunk.constants.get(c).copied() else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    // R[A+1] := R[B] (self parameter)
    vm.register_stack[base_ptr + a + 1] = table;

    // R[A] := R[B][K[C]] (method)
    let method = vm
        .table_get_with_meta(&table, &key)
        .unwrap_or(crate::LuaValue::nil());
    vm.register_stack[base_ptr + a] = method;

    Ok(())
}
