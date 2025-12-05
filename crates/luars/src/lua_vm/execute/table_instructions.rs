use crate::lua_value::{LuaValue, tm_flags};
/// Table operations
///
/// These instructions handle table creation, access, and manipulation.
use crate::lua_vm::{LuaCallFrame, LuaResult, LuaVM};
use crate::{get_a, get_ax, get_b, get_c, get_k};

/// NEWTABLE A B C k
/// R[A] := {} (size = B,C)
/// B = log2(hash_size) + 1 (0 means no hash part)
/// C = array_size % 256
/// k = 1 means EXTRAARG follows with array_size / 256
/// OPTIMIZED: Fast path for common empty/small table case
#[inline(always)]
pub fn exec_newtable(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) {
    let a = get_a!(instr);
    let b = get_b!(instr); // log2(hash_size) + 1
    let c = get_c!(instr); // array_size % 256
    let k = get_k!(instr); // true if EXTRAARG has high bits of array_size

    // Decode hash size: if b > 0, hash_size = 2^(b-1)
    let hash_size = if b > 0 { 1usize << (b - 1) } else { 0 };

    let func_value = unsafe {
        *pc += 1; // Skip EXTRAARG
        (*frame_ptr).as_function_value()
    };

    // Calculate array size - C is low bits, EXTRAARG has high bits when k=1
    let array_size = if k {
        // Need to read EXTRAARG for large arrays
        let prev_pc = *pc - 1; // We already incremented pc
        // Use new ID-based API to get function and read EXTRAARG
        if let Some(func_id) = func_value.as_function_id() {
            if let Some(func_ref) = vm.object_pool.get_function(func_id) {
                if let Some(chunk) = func_ref.chunk() {
                    if prev_pc < chunk.code.len() {
                        let extra = get_ax!(chunk.code[prev_pc]);
                        extra * 256 + c // MAXARG_C + 1 = 256
                    } else {
                        c
                    }
                } else {
                    c
                }
            } else {
                c
            }
        } else {
            c
        }
    } else {
        c
    };

    // Create new table with size hints (array_size, hash_size)
    let table = vm.create_table(array_size, hash_size);

    // Store in register - use unchecked for speed
    unsafe {
        *vm.register_stack.get_unchecked_mut(base_ptr + a) = table;
    }

    // GC checkpoint - inline fast path to avoid function call overhead
    // Only call slow path when debt exceeds 1MB threshold
    const GC_THRESHOLD: isize = 1024 * 1024;
    if vm.gc_debt_local > GC_THRESHOLD {
        vm.check_gc_slow_pub();
    }
}

/// GETTABLE A B C
/// R[A] := R[B][R[C]]
/// OPTIMIZED: Fast path for integer keys and tables without metatable
#[inline(always)]
pub fn exec_gettable(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    // Read values using unchecked access
    let (table_value, key_value) = unsafe {
        let table = *vm.register_stack.get_unchecked(*base_ptr + b);
        let key = *vm.register_stack.get_unchecked(*base_ptr + c);
        (table, key)
    };

    // FAST PATH: Direct table access for common case
    if let Some(table_id) = table_value.as_table_id() {
        // SAFETY: table_id is valid because it came from as_table_id()
        let lua_table = unsafe { vm.object_pool.get_table_unchecked(table_id) };

        // Check key type to choose optimal path
        // Integer keys may be in array part, other keys are in hash part
        let result = if let Some(i) = key_value.as_integer() {
            // Integer key: try array first, then hash
            lua_table
                .get_int(i)
                .or_else(|| lua_table.get_from_hash(&key_value))
        } else {
            // Non-integer key: direct hash lookup (most common for string keys)
            lua_table.get_from_hash(&key_value)
        };

        if let Some(val) = result {
            if !val.is_nil() {
                unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = val };
                return Ok(());
            }
        }

        // Key not found - check metatable for __index
        let metatable = lua_table.get_metatable();
        if metatable.is_none() {
            // No metatable - just return nil
            unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = LuaValue::nil() };
            return Ok(());
        }

        // FAST PATH: fasttm optimization - check if __index is known to be absent
        if let Some(mt_val) = metatable
            && let Some(mt_id) = mt_val.as_table_id()
        {
            let mt_table = unsafe { vm.object_pool.get_table_unchecked(mt_id) };
            if mt_table.tm_absent(tm_flags::TM_INDEX) {
                // __index is known to be absent - skip slow path
                unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = LuaValue::nil() };
                return Ok(());
            }
        }
    }

    // Slow path: Use metamethod handling
    let value = vm
        .table_get_with_meta(&table_value, &key_value)
        .unwrap_or(LuaValue::nil());

    // Re-read base_ptr after metamethod call
    *base_ptr = unsafe { (*frame_ptr).base_ptr } as usize;
    vm.register_stack[*base_ptr + a] = value;

    Ok(())
}

/// SETTABLE A B C k
/// R[A][R[B]] := RK(C)
/// OPTIMIZED: Fast path for integer keys and tables without metatable
#[inline(always)]
pub fn exec_settable(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);
    let k = get_k!(instr);

    // Read all values using unchecked access
    let (table_value, key_value, set_value) = unsafe {
        let table = *vm.register_stack.get_unchecked(*base_ptr + a);
        let key = *vm.register_stack.get_unchecked(*base_ptr + b);

        let value = if k {
            // Get constant via cached pointer
            *(*frame_ptr).constants_ptr.add(c)
        } else {
            *vm.register_stack.get_unchecked(*base_ptr + c)
        };

        (table, key, value)
    };

    // FAST PATH: Direct table access for common case (no metatable)
    if let Some(table_id) = table_value.as_table_id() {
        // SAFETY: table_id is valid because it came from as_table_id()
        let lua_table = unsafe { vm.object_pool.get_table_unchecked(table_id) };

        // Quick check: no metatable means no __newindex to worry about
        if lua_table.get_metatable().is_none() {
            let lua_table = unsafe { vm.object_pool.get_table_mut_unchecked(table_id) };

            // Try integer key fast path
            if let Some(i) = key_value.as_integer() {
                lua_table.set_int(i, set_value);
            } else {
                lua_table.raw_set(key_value, set_value);
            }

            // GC write barrier: if table is old and value is young, mark table as touched
            vm.gc_barrier_back_table(table_id, &set_value);
            return Ok(());
        }
    }

    // Slow path: use full metamethod handling
    vm.table_set_with_meta(table_value, key_value, set_value)?;

    Ok(())
}

/// GETI A B C
/// R[A] := R[B][C:integer]
/// OPTIMIZED: Direct integer access using get_int_full() without creating LuaValue key
#[inline(always)]
pub fn exec_geti(vm: &mut LuaVM, instr: u32, base_ptr: usize) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr) as i64; // C is unsigned integer index

    let table = unsafe { *vm.register_stack.get_unchecked(base_ptr + b) };

    // FAST PATH: Direct integer access for tables using unchecked access
    if let Some(table_id) = table.as_table_id() {
        // SAFETY: table_id is valid because it came from as_table_id()
        let lua_table = unsafe { vm.object_pool.get_table_unchecked(table_id) };

        // Use get_int_full to check both array and hash parts
        // This is necessary because integer keys may be stored in hash if array wasn't pre-allocated
        if let Some(val) = lua_table.get_int_full(c) {
            unsafe { *vm.register_stack.get_unchecked_mut(base_ptr + a) = val };
            return Ok(());
        }

        // Key not found - check for metatable and fasttm
        let metatable = lua_table.get_metatable();
        if metatable.is_none() {
            // No metatable - return nil directly
            unsafe { *vm.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::nil() };
            return Ok(());
        }

        // FAST PATH: fasttm optimization - check if __index is known to be absent
        if let Some(mt_val) = metatable
            && let Some(mt_id) = mt_val.as_table_id()
        {
            let mt_table = unsafe { vm.object_pool.get_table_unchecked(mt_id) };
            if mt_table.tm_absent(tm_flags::TM_INDEX) {
                // __index is known to be absent - skip slow path
                unsafe { *vm.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::nil() };
                return Ok(());
            }
        }
    }

    // Slow path: Use metamethod handling
    let key = LuaValue::integer(c);
    let value = vm
        .table_get_with_meta(&table, &key)
        .unwrap_or(LuaValue::nil());
    vm.register_stack[base_ptr + a] = value;

    Ok(())
}

/// SETI A B C
/// R[A][B] := RK(C)
/// OPTIMIZED: Direct integer key access using set_int()
#[inline(always)]
pub fn exec_seti(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr) as i64; // B is unsigned integer key
    let c = get_c!(instr);
    let k = get_k!(instr);

    // CRITICAL: Read all values BEFORE any metamethod calls
    let (table_value, set_value) = unsafe {
        let table = *vm.register_stack.get_unchecked(*base_ptr + a);

        let value = if k {
            // Get constant via cached pointer for speed
            *(*frame_ptr).constants_ptr.add(c)
        } else {
            *vm.register_stack.get_unchecked(*base_ptr + c)
        };

        (table, value)
    };

    // FAST PATH: Direct table access without metamethod check
    if let Some(table_id) = table_value.as_table_id() {
        // SAFETY: table_id is valid because it came from as_table_id()
        let lua_table = unsafe { vm.object_pool.get_table_unchecked(table_id) };

        // Quick check: no metatable means no __newindex to worry about
        if lua_table.get_metatable().is_none() {
            // Ultra-fast path: direct integer set
            let lua_table = unsafe { vm.object_pool.get_table_mut_unchecked(table_id) };
            lua_table.set_int(b, set_value);

            // GC write barrier
            vm.gc_barrier_back_table(table_id, &set_value);
            return Ok(());
        }
    }

    // Slow path: use full metamethod handling
    let key_value = crate::LuaValue::integer(b);
    vm.table_set_with_meta(table_value, key_value, set_value)?;

    Ok(())
}

/// GETFIELD A B C
/// R[A] := R[B][K[C]:string]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
pub fn exec_getfield(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    // FAST PATH: Direct constant access via cached pointer (like GETTABUP)
    let key_value = unsafe { *(*frame_ptr).constants_ptr.add(c) };
    let table_value = unsafe { *vm.register_stack.get_unchecked(*base_ptr + b) };

    // FAST PATH: Direct hash access for tables without metatable
    if let Some(table_id) = table_value.as_table_id() {
        // OPTIMIZED: Use unchecked table access
        let table_ref = unsafe { vm.object_pool.get_table_unchecked(table_id) };

        // GETFIELD always uses string keys - direct hash lookup
        if let Some(val) = table_ref.get_from_hash(&key_value) {
            if !val.is_nil() {
                unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = val };
                return Ok(());
            }
        }

        // Check for metatable and fasttm optimization
        let metatable = table_ref.get_metatable();
        if metatable.is_none() {
            // No metatable - return nil directly
            unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = LuaValue::nil() };
            return Ok(());
        }

        // FAST PATH: fasttm optimization - check if __index is known to be absent
        if let Some(mt_val) = metatable
            && let Some(mt_id) = mt_val.as_table_id()
        {
            let mt_table = unsafe { vm.object_pool.get_table_unchecked(mt_id) };
            if mt_table.tm_absent(tm_flags::TM_INDEX) {
                // __index is known to be absent - skip slow path
                unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = LuaValue::nil() };
                return Ok(());
            }
        }
    }

    // Slow path: Use metamethod handling
    let value = vm
        .table_get_with_meta(&table_value, &key_value)
        .unwrap_or(LuaValue::nil());

    // IMPORTANT: Re-read base_ptr after metamethod call in case frames changed
    *base_ptr = unsafe { (*frame_ptr).base_ptr } as usize;
    unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = value };

    Ok(())
}

/// SETFIELD A B C k
/// R[A][K[B]:string] := RK(C)
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
pub fn exec_setfield(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);
    let k = get_k!(instr);

    // FAST PATH: Direct access via cached pointers
    let (table_value, key_value, set_value) = unsafe {
        let table = *vm.register_stack.get_unchecked(*base_ptr + a);
        let key = *(*frame_ptr).constants_ptr.add(b);
        let value = if k {
            *(*frame_ptr).constants_ptr.add(c)
        } else {
            *vm.register_stack.get_unchecked(*base_ptr + c)
        };
        (table, key, value)
    };

    // FAST PATH: Direct table access without metamethod check for common case
    if let Some(table_id) = table_value.as_table_id() {
        // OPTIMIZED: Use unchecked access
        let table_ref = unsafe { vm.object_pool.get_table_unchecked(table_id) };

        // Quick check: no metatable means no __newindex to worry about
        if table_ref.get_metatable().is_none() {
            let table_ref = unsafe { vm.object_pool.get_table_mut_unchecked(table_id) };
            // Ultra-fast path: direct set without any metamethod checks
            table_ref.raw_set(key_value, set_value);
            // GC write barrier
            vm.gc_barrier_back_table(table_id, &set_value);
            return Ok(());
        }
    }

    // Slow path: use full metamethod handling
    vm.table_set_with_meta(table_value, key_value, set_value)?;

    Ok(())
}

/// GETTABUP A B C
/// R[A] := UpValue[B][K[C]:string]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
/// Note: Key is always a string from constant table per Lua 5.4 spec
#[inline(always)]
pub fn exec_gettabup(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    // FAST PATH: Direct constant access via cached pointer
    let key_value = unsafe { *(*frame_ptr).constants_ptr.add(c) };

    // OPTIMIZED: Use cached upvalues_ptr for direct access (avoids function lookup)
    let upvalue_id = unsafe { *(*frame_ptr).upvalues_ptr.add(b) };

    // OPTIMIZED: Use unchecked upvalue read
    let table_value = unsafe { vm.read_upvalue_unchecked(upvalue_id) };

    // FAST PATH: Direct hash access for tables without metatable
    if let Some(table_id) = table_value.as_table_id() {
        // OPTIMIZED: Use unchecked table access
        let table_ref = unsafe { vm.object_pool.get_table_unchecked(table_id) };

        // Key is always string from constants, skip integer check - use direct hash lookup
        if let Some(val) = table_ref.get_from_hash(&key_value) {
            if !val.is_nil() {
                unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = val };
                return Ok(());
            }
        }

        // Check if no metatable - can return nil directly
        if table_ref.get_metatable().is_none() {
            unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = LuaValue::nil() };
            return Ok(());
        }
    }

    // Slow path: Use metamethod handling
    let value = vm
        .table_get_with_meta(&table_value, &key_value)
        .unwrap_or(LuaValue::nil());

    unsafe { *vm.register_stack.get_unchecked_mut(*base_ptr + a) = value };

    Ok(())
}

/// SETTABUP A B C k
/// UpValue[A][K[B]:string] := RK(C)
/// OPTIMIZED: Uses cached constants_ptr and upvalues_ptr for direct access
#[inline(always)]
pub fn exec_settabup(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);
    let k = get_k!(instr);

    // FAST PATH: Direct constant access via cached pointer
    let key_value = unsafe { *(*frame_ptr).constants_ptr.add(b) };

    // Get set_value - either from constant or register
    let set_value = if k {
        unsafe { *(*frame_ptr).constants_ptr.add(c) }
    } else {
        vm.register_stack[*base_ptr + c]
    };

    // OPTIMIZED: Use cached upvalues_ptr for direct access
    let upvalue_id = unsafe { *(*frame_ptr).upvalues_ptr.add(a) };

    // Get upvalue value
    let table_value = vm.read_upvalue(upvalue_id);

    // FAST PATH: Direct table access without metamethod check for common case
    if let Some(table_id) = table_value.as_table_id() {
        if let Some(table_ref) = vm.object_pool.get_table_mut(table_id) {
            // Quick check: no metatable means no __newindex to worry about
            if table_ref.get_metatable().is_none() {
                // Ultra-fast path: direct set without any metamethod checks
                table_ref.raw_set(key_value.clone(), set_value.clone());

                // GC write barrier
                vm.gc_barrier_back_table(table_id, &set_value);
                return Ok(());
            }
        }
    }

    // Slow path: use full metamethod handling
    vm.table_set_with_meta(table_value, key_value, set_value)?;

    Ok(())
}

/// SELF A B C
/// R[A+1] := R[B]; R[A] := R[B][RK(C):string]
#[inline(always)]
pub fn exec_self(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) -> LuaResult<()> {
    let a = get_a!(instr);
    let b = get_b!(instr);
    let c = get_c!(instr);

    let func_id = unsafe { (*frame_ptr).get_function_id() };
    let table = vm.register_stack[*base_ptr + b];

    // Get method key from constant using new API
    let Some(func_id) = func_id else {
        return Err(vm.error("Not a Lua function".to_string()));
    };
    let Some(func_ref) = vm.object_pool.get_function(func_id) else {
        return Err(vm.error("Invalid function ID".to_string()));
    };
    let Some(chunk) = func_ref.chunk() else {
        return Err(vm.error("Not a Lua function".to_string()));
    };
    let Some(key) = chunk.constants.get(c).copied() else {
        return Err(vm.error(format!("Invalid constant index: {}", c)));
    };

    // R[A+1] := R[B] (self parameter)
    vm.register_stack[*base_ptr + a + 1] = table;

    // R[A] := R[B][K[C]] (method)
    // Support both tables and userdata
    let method = if table.is_userdata() {
        vm.userdata_get(&table, &key)
            .unwrap_or(crate::LuaValue::nil())
    } else {
        vm.table_get_with_meta(&table, &key)
            .unwrap_or(crate::LuaValue::nil())
    };
    vm.register_stack[*base_ptr + a] = method;

    Ok(())
}
