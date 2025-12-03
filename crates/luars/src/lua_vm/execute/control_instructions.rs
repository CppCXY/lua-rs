/// Control flow instructions
///
/// These instructions handle function calls, returns, jumps, and coroutine operations.
use crate::LuaValue;
use crate::lua_value::LuaValueKind;
use crate::lua_vm::{Instruction, LuaCallFrame, LuaError, LuaResult, LuaVM};

/// RETURN A B C k
/// return R[A], ... ,R[A+B-2]
pub fn exec_return(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr_ptr: &mut *mut LuaCallFrame,
) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    // Get frame info BEFORE popping
    let base_ptr = unsafe { (**frame_ptr_ptr).base_ptr } as usize;
    let result_reg = unsafe { (**frame_ptr_ptr).get_result_reg() };
    let num_results = unsafe { (**frame_ptr_ptr).get_num_results() };
    let top = unsafe { (**frame_ptr_ptr).top } as usize;

    // Calculate return count
    let return_count = if b == 0 { top.saturating_sub(a) } else { b - 1 };

    // Close upvalues before popping the frame
    if !vm.open_upvalues.is_empty() {
        vm.close_upvalues_from(base_ptr);
    }

    // Handle upvalue closing (k bit) - BEFORE popping frame
    if k {
        let close_from = base_ptr + a;
        vm.close_upvalues_from(close_from);
        vm.close_to_be_closed(close_from)?;
    }

    // Calculate caller info BEFORE pop
    let has_caller = vm.frame_count > 1;
    let (caller_ptr, caller_is_lua) = if has_caller {
        let ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 2) };
        let is_lua = unsafe { (*ptr).is_lua() };
        (ptr, is_lua)
    } else {
        (std::ptr::null_mut(), false)
    };

    // Pop frame - decrement counter
    vm.frame_count -= 1;

    // Check if caller is a Lua function
    if has_caller && caller_is_lua {
        // Update frame_ptr to caller
        *frame_ptr_ptr = caller_ptr;

        let caller_base = unsafe { (*caller_ptr).base_ptr } as usize;
        let dest_base = caller_base + result_reg;

        // Ensure destination has enough space
        let dest_end = dest_base
            + return_count.max(if num_results == usize::MAX {
                return_count
            } else {
                num_results
            });
        if vm.register_stack.len() < dest_end {
            vm.ensure_stack_capacity(dest_end);
            vm.register_stack.resize(dest_end, LuaValue::nil());
        }

        unsafe {
            let reg_ptr = vm.register_stack.as_mut_ptr();

            if num_results == usize::MAX {
                // Return all values
                if return_count > 0 {
                    let src_start = base_ptr + a;
                    let src_end = src_start + return_count;
                    let dst_start = dest_base;
                    let dst_end = dst_start + return_count;

                    if (src_start < dst_end) && (dst_start < src_end) {
                        std::ptr::copy(
                            reg_ptr.add(src_start),
                            reg_ptr.add(dst_start),
                            return_count,
                        );
                    } else {
                        std::ptr::copy_nonoverlapping(
                            reg_ptr.add(src_start),
                            reg_ptr.add(dst_start),
                            return_count,
                        );
                    }
                }
                (*caller_ptr).top = (result_reg + return_count) as u32;
            } else {
                // Fixed number of return values
                let nil_val = LuaValue::nil();
                for i in 0..num_results {
                    let val = if i < return_count {
                        *reg_ptr.add(base_ptr + a + i)
                    } else {
                        nil_val
                    };
                    *reg_ptr.add(dest_base + i) = val;
                }
                (*caller_ptr).top = (result_reg + num_results) as u32;
            }
        }

        Ok(())
    } else if has_caller {
        // Caller is C function - write return values to return_values
        *frame_ptr_ptr = caller_ptr;
        vm.return_values.clear();
        for i in 0..return_count {
            if base_ptr + a + i < vm.register_stack.len() {
                vm.return_values.push(vm.register_stack[base_ptr + a + i]);
            }
        }
        Err(LuaError::Exit)
    } else {
        // No caller - exit VM
        vm.return_values.clear();
        for i in 0..return_count {
            if base_ptr + a + i < vm.register_stack.len() {
                vm.return_values.push(vm.register_stack[base_ptr + a + i]);
            }
        }
        Err(LuaError::Exit)
    }
}

// ============ Jump Instructions ============

/// JMP sJ
/// pc += sJ
#[allow(dead_code)]
#[inline(always)]
pub fn exec_jmp(instr: u32, pc: &mut usize) {
    let sj = Instruction::get_sj(instr);

    // PC already incremented by dispatcher, so we add offset directly
    *pc = (*pc as i32 + sj) as usize;
}

// ============ Test Instructions ============

/// TEST A k
/// if (not R[A] == k) then pc++
/// ULTRA-OPTIMIZED: Direct type tag check, single branch
#[allow(dead_code)]
#[inline(always)]
pub fn exec_test(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = Instruction::get_a(instr) as usize;
    let k = Instruction::get_k(instr);

    unsafe {
        // OPTIMIZATION: Direct unsafe access and type tag comparison
        let value = *vm.register_stack.as_ptr().add(base_ptr + a);

        // OPTIMIZATION: Fast truthiness check using type tags
        // nil = TAG_NIL, false = VALUE_FALSE
        // Only nil and false are falsy
        use crate::lua_value::{TAG_NIL, VALUE_FALSE};
        let is_truthy = value.primary != TAG_NIL && value.primary != VALUE_FALSE;

        // If (not value) == k, skip next instruction
        if !is_truthy == k {
            *pc += 1;
        }
    }
}

/// TESTSET A B k
/// if (not R[B] == k) then R[A] := R[B] else pc++
/// ULTRA-OPTIMIZED: Direct type tag check, single branch
#[inline(always)]
pub fn exec_testset(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    unsafe {
        // OPTIMIZATION: Direct unsafe access
        let reg_ptr = vm.register_stack.as_ptr().add(base_ptr);
        let value = *reg_ptr.add(b);

        // OPTIMIZATION: Fast truthiness check
        use crate::lua_value::{TAG_NIL, VALUE_FALSE};
        let is_truthy = value.primary != TAG_NIL && value.primary != VALUE_FALSE;

        // If (is_truthy == k), assign R[A] = R[B], otherwise skip next instruction
        if is_truthy == k {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = value;
        } else {
            *pc += 1;
        }
    }
}

// ============ Comparison Instructions ============

/// EQ A B k
/// if ((R[A] == R[B]) ~= k) then pc++
/// ULTRA-OPTIMIZED: Fast path for common types (integers, floats, strings)
#[inline(always)]
pub fn exec_eq(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    // OPTIMIZATION: Use unsafe for unchecked register access
    let (left, right) = unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        (*reg_base.add(a), *reg_base.add(b))
    };

    // OPTIMIZATION: Fast path - check if primary fields are identical (same type and value/id)
    use crate::lua_value::{TAG_FLOAT, TAG_INTEGER, TAG_STRING, TYPE_MASK};
    let left_tag = left.primary & TYPE_MASK;
    let right_tag = right.primary & TYPE_MASK;

    let mut is_equal = if left.primary == right.primary && left.secondary == right.secondary {
        // Identical values (same type, same bits)
        true
    } else if left_tag == TAG_INTEGER && right_tag == TAG_INTEGER {
        // Both integers but different values
        false
    } else if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
        // Both floats, compare values
        f64::from_bits(left.secondary) == f64::from_bits(right.secondary)
    } else if left_tag == TAG_STRING && right_tag == TAG_STRING {
        // Different string IDs means different strings (strings are interned)
        false
    } else if left_tag == TAG_INTEGER && right_tag == TAG_FLOAT {
        // Mixed integer/float comparison
        (left.secondary as i64) as f64 == f64::from_bits(right.secondary)
    } else if left_tag == TAG_FLOAT && right_tag == TAG_INTEGER {
        // Mixed float/integer comparison
        f64::from_bits(left.secondary) == (right.secondary as i64) as f64
    } else {
        // Slow path: check full equality (handles tables, etc.)
        left == right
    };

    // If not equal by value, try __eq metamethod
    // IMPORTANT: Both operands must have the SAME __eq metamethod (Lua 5.4 spec)
    if !is_equal && (left.is_table() || right.is_table()) {
        // Use pre-cached __eq StringId
        let mm_key = LuaValue::string(vm.object_pool.tm_eq);

        let left_mt = vm.table_get_metatable(&left);
        let right_mt = vm.table_get_metatable(&right);

        if let (Some(lmt), Some(rmt)) = (left_mt, right_mt) {
            let left_mm = vm.table_get_with_meta(&lmt, &mm_key);
            let right_mm = vm.table_get_with_meta(&rmt, &mm_key);

            // Both must have __eq and they must be the same function
            if let (Some(lmm), Some(rmm)) = (left_mm, right_mm) {
                if !lmm.is_nil() && lmm == rmm {
                    if let Some(result) = vm.call_metamethod(&lmm, &[left, right])? {
                        is_equal = !result.is_nil() && result.as_bool().unwrap_or(true);
                    }
                }
            }
        }
    }

    // If (left == right) != k, skip next instruction
    if is_equal != k {
        *pc += 1;
    }

    Ok(())
}

/// LT A B k
/// if ((R[A] < R[B]) ~= k) then pc++
/// ULTRA-OPTIMIZED: Direct integer fast path like Lua C
#[inline(always)]
pub fn exec_lt(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: &mut usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    unsafe {
        let reg_base = vm.register_stack.as_ptr().add(*base_ptr);
        let left = *reg_base.add(a);
        let right = *reg_base.add(b);

        use crate::lua_value::{TAG_FLOAT, TAG_INTEGER, TYPE_MASK};
        let left_tag = left.primary & TYPE_MASK;
        let right_tag = right.primary & TYPE_MASK;

        // Fast path: both integers (most common case in loops)
        if left_tag == TAG_INTEGER && right_tag == TAG_INTEGER {
            let is_less = (left.secondary as i64) < (right.secondary as i64);
            if is_less != k {
                *pc += 1;
            }
            return Ok(());
        }

        // Fast path: both floats
        if left_tag == TAG_FLOAT && right_tag == TAG_FLOAT {
            let is_less = f64::from_bits(left.secondary) < f64::from_bits(right.secondary);
            if is_less != k {
                *pc += 1;
            }
            return Ok(());
        }

        // Mixed numeric types
        if (left_tag == TAG_INTEGER || left_tag == TAG_FLOAT)
            && (right_tag == TAG_INTEGER || right_tag == TAG_FLOAT)
        {
            let left_f = if left_tag == TAG_INTEGER {
                (left.secondary as i64) as f64
            } else {
                f64::from_bits(left.secondary)
            };
            let right_f = if right_tag == TAG_INTEGER {
                (right.secondary as i64) as f64
            } else {
                f64::from_bits(right.secondary)
            };
            let is_less = left_f < right_f;
            if is_less != k {
                *pc += 1;
            }
            return Ok(());
        }

        // String comparison
        use crate::lua_value::TAG_STRING;
        if left_tag == TAG_STRING && right_tag == TAG_STRING {
            let is_less = left < right;
            if is_less != k {
                *pc += 1;
            }
            return Ok(());
        }

        // Slow path: metamethod
        exec_lt_metamethod(vm, left, right, k, pc)
    }
}

/// Slow path for LT metamethod lookup
#[cold]
#[inline(never)]
fn exec_lt_metamethod(
    vm: &mut LuaVM,
    left: crate::LuaValue,
    right: crate::LuaValue,
    k: bool,
    pc: &mut usize,
) -> LuaResult<()> {
    // Use pre-cached __lt StringId
    let mm_key = LuaValue::string(vm.object_pool.tm_lt);
    let mut found_metamethod = false;

    if let Some(mt) = vm.table_get_metatable(&left) {
        if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key) {
            if !metamethod.is_nil() {
                if let Some(result) = vm.call_metamethod(&metamethod, &[left, right])? {
                    let is_less_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                    if is_less_result != k {
                        *pc += 1;
                    }
                    return Ok(());
                }
                found_metamethod = true;
            }
        }
    }

    if !found_metamethod {
        if let Some(mt) = vm.table_get_metatable(&right) {
            if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key) {
                if !metamethod.is_nil() {
                    if let Some(result) = vm.call_metamethod(&metamethod, &[left, right])? {
                        let is_less_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                        if is_less_result != k {
                            *pc += 1;
                        }
                        return Ok(());
                    }
                    found_metamethod = true;
                }
            }
        }
    }

    if !found_metamethod {
        return Err(vm.error(format!(
            "attempt to compare {} with {}",
            left.type_name(),
            right.type_name()
        )));
    }
    Ok(())
}

/// LE A B k
/// if ((R[A] <= R[B]) ~= k) then pc++
/// ULTRA-OPTIMIZED: Use combined_tags for fast path like LT
#[inline(always)]
pub fn exec_le(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    // OPTIMIZATION: Use unsafe for unchecked register access
    let (left, right) = unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        (*reg_base.add(a), *reg_base.add(b))
    };

    // OPTIMIZATION: Direct type tag comparison with combined_tags (like LT)
    use crate::lua_value::TYPE_MASK;
    let left_tag = left.primary & TYPE_MASK;
    let right_tag = right.primary & TYPE_MASK;

    // Combined type check for fast paths (single branch!)
    // Note: Shift TAG values right by 48 bits to get small values (0-15) for combining
    let left_tag_small = left_tag >> 48;
    let right_tag_small = right_tag >> 48;
    let combined_tags = (left_tag_small << 4) | right_tag_small;

    // Small tag values after >> 48: TAG_INTEGER=3, TAG_FLOAT=4, TAG_STRING=5
    const INT_INT: u64 = (3 << 4) | 3; // 0x33
    const FLOAT_FLOAT: u64 = (4 << 4) | 4; // 0x44
    const INT_FLOAT: u64 = (3 << 4) | 4; // 0x34
    const FLOAT_INT: u64 = (4 << 4) | 3; // 0x43
    const STRING_STRING: u64 = (5 << 4) | 5; // 0x55

    let is_less_or_equal = if combined_tags == INT_INT {
        // Fast integer path - single branch!
        (left.secondary as i64) <= (right.secondary as i64)
    } else if combined_tags == FLOAT_FLOAT {
        // Fast float path
        f64::from_bits(left.secondary) <= f64::from_bits(right.secondary)
    } else if combined_tags == INT_FLOAT {
        // Mixed: integer <= float
        ((left.secondary as i64) as f64) <= f64::from_bits(right.secondary)
    } else if combined_tags == FLOAT_INT {
        // Mixed: float <= integer
        f64::from_bits(left.secondary) <= ((right.secondary as i64) as f64)
    } else if combined_tags == STRING_STRING {
        // String comparison
        left <= right
    } else {
        // Try __le metamethod first - use pre-cached StringId
        let mm_key_le = LuaValue::string(vm.object_pool.tm_le);
        let mut found_metamethod = false;

        if let Some(mt) = vm.table_get_metatable(&left) {
            if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key_le) {
                if !metamethod.is_nil() {
                    if let Some(result) = vm.call_metamethod(&metamethod, &[left, right])? {
                        let is_le_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                        if is_le_result != k {
                            *pc += 1;
                        }
                        return Ok(());
                    }
                    found_metamethod = true;
                }
            }
        }

        if !found_metamethod {
            if let Some(mt) = vm.table_get_metatable(&right) {
                if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key_le) {
                    if !metamethod.is_nil() {
                        if let Some(result) = vm.call_metamethod(&metamethod, &[left, right])? {
                            let is_le_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                            if is_le_result != k {
                                *pc += 1;
                            }
                            return Ok(());
                        }
                        found_metamethod = true;
                    }
                }
            }
        }

        // If __le not found, try __lt and compute !(b < a)
        if !found_metamethod {
            // Use pre-cached __lt StringId
            let mm_key_lt = LuaValue::string(vm.object_pool.tm_lt);

            if let Some(mt) = vm.table_get_metatable(&right) {
                if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key_lt) {
                    if !metamethod.is_nil() {
                        if let Some(result) = vm.call_metamethod(&metamethod, &[right, left])? {
                            let is_gt_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                            let is_le_result = !is_gt_result; // a <= b is !(b < a)
                            if is_le_result != k {
                                *pc += 1;
                            }
                            return Ok(());
                        }
                        found_metamethod = true;
                    }
                }
            }

            if !found_metamethod {
                if let Some(mt) = vm.table_get_metatable(&left) {
                    if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key_lt) {
                        if !metamethod.is_nil() {
                            if let Some(result) = vm.call_metamethod(&metamethod, &[right, left])? {
                                let is_gt_result =
                                    !result.is_nil() && result.as_bool().unwrap_or(true);
                                let is_le_result = !is_gt_result;
                                if is_le_result != k {
                                    *pc += 1;
                                }
                                return Ok(());
                            }
                            found_metamethod = true;
                        }
                    }
                }
            }
        }

        if !found_metamethod {
            return Err(vm.error(format!(
                "attempt to compare {} with {}",
                left.type_name(),
                right.type_name()
            )));
        }
        return Ok(());
    };

    if is_less_or_equal != k {
        *pc += 1;
    }

    Ok(())
}

/// EQK A B k
/// if ((R[A] == K[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_eqk(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    pc: &mut usize,
    base_ptr: usize,
) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let func_id = unsafe { (*frame_ptr).get_function_id() };
    // Get function using new ID-based API
    let Some(func_id) = func_id else {
        return Err(vm.error("Not a Lua function".to_string()));
    };
    let Some(func_ref) = vm.object_pool.get_function(func_id) else {
        return Err(vm.error("Invalid function ID".to_string()));
    };
    let Some(constant) = func_ref.chunk.constants.get(b).copied() else {
        return Err(vm.error(format!("Invalid constant index: {}", b)));
    };

    let left = unsafe { *vm.register_stack.as_ptr().add(base_ptr + a) };

    let is_equal = left == constant;

    if is_equal != k {
        *pc += 1;
    }

    Ok(())
}

/// EQI A sB k
/// if ((R[A] == sB) ~= k) then pc++
#[inline(always)]
pub fn exec_eqi(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + a);

        use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
        let is_equal = if (left.primary & TYPE_MASK) == TAG_INTEGER {
            (left.secondary as i64) == (sb as i64)
        } else if let Some(l) = left.as_number() {
            l == sb as f64
        } else {
            false
        };

        if is_equal != k {
            *pc += 1;
        }
    }
}

/// LTI A sB k
/// if ((R[A] < sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lti(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + a);

        // OPTIMIZATION: Direct type tag comparison
        use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
        let is_less = if (left.primary & TYPE_MASK) == TAG_INTEGER {
            // Fast integer path
            (left.secondary as i64) < (sb as i64)
        } else if let Some(l) = left.as_number() {
            l < sb as f64
        } else {
            return Err(vm.error(format!(
                "attempt to compare {} with number",
                left.type_name()
            )));
        };

        if is_less != k {
            *pc += 1;
        }
    }

    Ok(())
}

/// LEI A sB k
/// if ((R[A] <= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lei(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + a);

        use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
        let is_less_equal = if (left.primary & TYPE_MASK) == TAG_INTEGER {
            (left.secondary as i64) <= (sb as i64)
        } else if let Some(l) = left.as_number() {
            l <= sb as f64
        } else {
            return Err(vm.error(format!(
                "attempt to compare {} with number",
                left.type_name()
            )));
        };

        if is_less_equal != k {
            *pc += 1;
        }
    }

    Ok(())
}

/// GTI A sB k
/// if ((R[A] > sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gti(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + a);

        use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
        let is_greater = if (left.primary & TYPE_MASK) == TAG_INTEGER {
            (left.secondary as i64) > (sb as i64)
        } else if let Some(l) = left.as_number() {
            l > sb as f64
        } else {
            return Err(vm.error(format!(
                "attempt to compare {} with number",
                left.type_name()
            )));
        };

        if is_greater != k {
            *pc += 1;
        }
    }

    Ok(())
}

/// GEI A sB k
/// if ((R[A] >= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gei(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let left = *vm.register_stack.as_ptr().add(base_ptr + a);

        use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
        let is_greater_equal = if (left.primary & TYPE_MASK) == TAG_INTEGER {
            (left.secondary as i64) >= (sb as i64)
        } else if let Some(l) = left.as_number() {
            l >= sb as f64
        } else {
            return Err(vm.error(format!(
                "attempt to compare {} with number",
                left.type_name()
            )));
        };

        if is_greater_equal != k {
            *pc += 1;
        }
    }

    Ok(())
}

// ============ Call Instructions ============

/// CALL A B C
/// R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
/// ULTRA-OPTIMIZED: Minimize overhead for the common case (Lua function, no metamethod)
/// Returns Ok(true) if frame changed (Lua function) and updatestate is needed
/// Returns Ok(false) if frame unchanged (C function) and no updatestate needed
#[inline(always)]
pub fn exec_call(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr_ptr: &mut *mut LuaCallFrame,
) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    // OPTIMIZATION: Use passed frame_ptr directly - avoid Vec lookup!
    let (base, func) = unsafe {
        let base = (**frame_ptr_ptr).base_ptr as usize;
        let func = *vm.register_stack.get_unchecked(base + a);
        (base, func)
    };

    // OPTIMIZATION: Fast path for Lua functions (most common case)
    // Check type tag directly without full pattern match
    use crate::lua_value::TAG_FUNCTION;
    if (func.primary & crate::lua_value::TYPE_MASK) == TAG_FUNCTION {
        exec_call_lua_function(
            vm,
            func,
            a,
            b,
            c,
            base,
            false,
            LuaValue::nil(),
            frame_ptr_ptr,
        )?;
        return Ok(()); // Frame changed, need updatestate
    }

    // Check for CFunction
    if func.is_cfunction() {
        exec_call_cfunction(
            vm,
            func,
            a,
            b,
            c,
            base,
            false,
            LuaValue::nil(),
            frame_ptr_ptr,
        )?;
        return Ok(()); // Frame unchanged, no updatestate needed
    }

    // Slow path: Check for __call metamethod
    if func.kind() == LuaValueKind::Table {
        let metatable_opt = func.as_table_id().and_then(|table_id| {
            vm.object_pool
                .get_table(table_id)
                .and_then(|t| t.get_metatable())
        });

        if let Some(metatable) = metatable_opt {
            // Use pre-cached __call StringId
            let call_key = LuaValue::string(vm.object_pool.tm_call);
            if let Some(call_func) = vm.table_get_with_meta(&metatable, &call_key) {
                if call_func.is_callable() {
                    if call_func.is_cfunction() {
                        exec_call_cfunction(
                            vm,
                            call_func,
                            a,
                            b,
                            c,
                            base,
                            true,
                            func,
                            frame_ptr_ptr,
                        )?;
                        return Ok(()); // C function, no updatestate
                    } else {
                        exec_call_lua_function(
                            vm,
                            call_func,
                            a,
                            b,
                            c,
                            base,
                            true,
                            func,
                            frame_ptr_ptr,
                        )?;
                        return Ok(()); // Lua function, need updatestate
                    }
                }
            }
        }
    }

    Err(vm.error(format!("attempt to call a {} value", func.type_name())))
}

/// Fast path for Lua function calls
/// OPTIMIZED: Minimize branches and memory operations for common case
#[inline(always)]
fn exec_call_lua_function(
    vm: &mut LuaVM,
    func: LuaValue,
    a: usize,
    b: usize,
    c: usize,
    caller_base: usize,
    use_call_metamethod: bool,
    call_metamethod_self: LuaValue,
    frame_ptr_ptr: &mut *mut LuaCallFrame, // Use passed frame_ptr!
) -> LuaResult<()> {
    // Safepoint GC check: run GC at function call boundaries
    // This is much cheaper than checking on every table operation
    if vm.gc_debt_local > 1024 * 1024 {
        vm.check_gc_slow_pub();
    }

    // Get function ID - FAST PATH: assume valid function
    let func_id = unsafe { func.as_function_id().unwrap_unchecked() };

    // Extract chunk info from ObjectPool - use unchecked for hot path
    let func_ref = unsafe { vm.object_pool.get_function_unchecked(func_id) };

    let (max_stack_size, num_params, is_vararg, code_ptr, constants_ptr) = (
        func_ref.chunk.max_stack_size,
        func_ref.chunk.param_count,
        func_ref.chunk.is_vararg,
        func_ref.chunk.code.as_ptr(),
        func_ref.chunk.constants.as_ptr(),
    );

    // Calculate argument count - use frame_ptr directly!
    let arg_count = if b == 0 {
        unsafe { (**frame_ptr_ptr).top.saturating_sub((a + 1) as u32) as usize }
    } else {
        unsafe {
            (**frame_ptr_ptr).top = (a + b) as u32;
        }
        b - 1
    };

    let return_count = if c == 0 { usize::MAX } else { c - 1 };

    // Zero-copy: new frame base = R[A+1]
    let new_base = caller_base + a + 1;

    // FAST PATH: No metamethod, no vararg (most common case)
    if !use_call_metamethod && !is_vararg {
        // Simple case: just ensure capacity and push frame
        let required_capacity = new_base + max_stack_size;

        // Ensure capacity - single branch
        if vm.register_stack.len() < required_capacity {
            vm.register_stack.resize(required_capacity, LuaValue::nil());
        }

        // LIKE LUA C: Only fill missing arguments (arg_count < num_params)
        // NOT all stack slots! This is the key optimization.
        // For add(a,b) called with add(1,5): arg_count=2, num_params=2 → no fill!
        if arg_count < num_params {
            unsafe {
                let reg_ptr = vm.register_stack.as_mut_ptr().add(new_base);
                let nil_val = LuaValue::nil();
                for i in arg_count..num_params {
                    *reg_ptr.add(i) = nil_val;
                }
            }
        }

        // Create and push new frame - inline nresults calculation
        let nresults = if c == 0 { -1i16 } else { (c - 1) as i16 };
        let new_frame = LuaCallFrame::new_lua_function(
            func_id,
            code_ptr,
            constants_ptr,
            new_base,
            arg_count, // top = number of arguments
            a,         // result_reg
            nresults,
        );

        *frame_ptr_ptr = vm.push_frame(new_frame);
        return Ok(());
    }

    // SLOW PATH: Handle metamethod or vararg
    let actual_arg_count = if use_call_metamethod {
        arg_count + 1
    } else {
        arg_count
    };
    let actual_stack_size = max_stack_size.max(actual_arg_count);
    let total_stack_size = if is_vararg && actual_arg_count > 0 {
        actual_stack_size + actual_arg_count
    } else {
        actual_stack_size
    };

    // Ensure stack capacity
    let required_capacity = new_base + total_stack_size;
    if vm.register_stack.len() < required_capacity {
        vm.ensure_stack_capacity(required_capacity);
        vm.register_stack.resize(required_capacity, LuaValue::nil());
    }

    // Initialize registers
    unsafe {
        let reg_ptr = vm.register_stack.as_mut_ptr();
        let nil_val = LuaValue::nil();

        // Initialize local variable slots beyond arguments
        for i in actual_arg_count..actual_stack_size {
            *reg_ptr.add(new_base + i) = nil_val;
        }

        // Vararg extra space
        if is_vararg && actual_arg_count > 0 {
            for i in actual_stack_size..total_stack_size {
                *reg_ptr.add(new_base + i) = nil_val;
            }
        }

        // __call metamethod: shift arguments and insert self
        if use_call_metamethod && arg_count > 0 {
            for i in (0..arg_count).rev() {
                *reg_ptr.add(new_base + 1 + i) = *reg_ptr.add(new_base + i);
            }
            *reg_ptr.add(new_base) = call_metamethod_self;
        } else if use_call_metamethod {
            *reg_ptr.add(new_base) = call_metamethod_self;
        }
    }

    // Create and push new frame
    let nresults = if return_count == usize::MAX {
        -1i16
    } else {
        return_count as i16
    };
    let new_frame = LuaCallFrame::new_lua_function(
        func_id,
        code_ptr,
        constants_ptr,
        new_base,
        actual_arg_count, // top = number of arguments
        a,                // result_reg
        nresults,
    );

    *frame_ptr_ptr = vm.push_frame(new_frame);
    Ok(())
}

/// Fast path for C function calls
/// Note: Must update frame_ptr_ptr after return because C functions may recursively call Lua
#[inline(always)]
fn exec_call_cfunction(
    vm: &mut LuaVM,
    func: LuaValue,
    a: usize,
    b: usize,
    c: usize,
    base: usize,
    use_call_metamethod: bool,
    call_metamethod_self: LuaValue,
    frame_ptr_ptr: &mut *mut LuaCallFrame,
) -> LuaResult<()> {
    let cfunc = unsafe { func.as_cfunction().unwrap_unchecked() };

    // Calculate argument count
    let arg_count = if b == 0 {
        let frame = vm.current_frame();
        if frame.top as usize > a + 1 {
            (frame.top as usize) - (a + 1)
        } else {
            0
        }
    } else {
        vm.current_frame_mut().top = (a + b) as u32;
        b - 1
    };

    let return_count = if c == 0 { usize::MAX } else { c - 1 };

    let call_base = base + a;

    // Handle __call metamethod
    if use_call_metamethod {
        if arg_count > 0 {
            for i in (0..arg_count).rev() {
                vm.register_stack[call_base + 2 + i] = vm.register_stack[call_base + 1 + i];
            }
        }
        vm.register_stack[call_base + 1] = call_metamethod_self;
        vm.register_stack[call_base] = func;
    }

    let actual_arg_count = if use_call_metamethod {
        arg_count + 1
    } else {
        arg_count
    };

    // Ensure stack capacity
    let required_top = call_base + actual_arg_count + 1 + 20;
    vm.ensure_stack_capacity(required_top);

    // Push C function frame
    let temp_frame = LuaCallFrame::new_c_function(call_base, actual_arg_count + 1);

    vm.push_frame(temp_frame);

    // Call C function
    let result = match cfunc(vm) {
        Ok(r) => r,
        Err(LuaError::Yield) => {
            vm.pop_frame_discard();
            // Update frame_ptr after pop (safely check for empty frames)
            if vm.frame_count > 0 {
                *frame_ptr_ptr = vm.current_frame_ptr();
            }
            return Err(LuaError::Yield);
        }
        Err(e) => {
            vm.pop_frame_discard();
            // Update frame_ptr after pop (safely check for empty frames)
            if vm.frame_count > 0 {
                *frame_ptr_ptr = vm.current_frame_ptr();
            }
            return Err(e);
        }
    };
    vm.pop_frame_discard();
    // CRITICAL: Update frame_ptr after C function call returns
    // The C function may have called Lua code which could have reallocated the frames Vec
    if vm.frame_count > 0 {
        *frame_ptr_ptr = vm.current_frame_ptr();
    }

    // OPTIMIZED: Copy return values without heap allocation for common cases
    let result_len = result.len();
    let num_returns = if return_count == usize::MAX {
        result_len
    } else {
        return_count.min(result_len)
    };

    // Fast path: copy inline values directly (no Vec allocation)
    if result.overflow.is_none() {
        // Values are stored inline
        if num_returns > 0 {
            vm.register_stack[call_base] = result.inline[0];
        }
        if num_returns > 1 {
            vm.register_stack[call_base + 1] = result.inline[1];
        }
    } else {
        // Slow path: overflow to Vec
        let values = result.all_values();
        if num_returns > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    values.as_ptr(),
                    vm.register_stack.as_mut_ptr().add(call_base),
                    num_returns,
                );
            }
        }
    }

    // Fill remaining with nil
    if return_count != usize::MAX {
        for i in num_returns..return_count {
            vm.register_stack[call_base + i] = crate::LuaValue::nil();
        }
    }

    vm.current_frame_mut().top = (a + num_returns) as u32;
    Ok(())
}

/// TAILCALL A B C k
/// return R[A](R[A+1], ... ,R[A+B-1])
pub fn exec_tailcall(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr_ptr: &mut *mut LuaCallFrame,
) -> LuaResult<()> {
    // TAILCALL A B C: return R[A](R[A+1], ..., R[A+B-1])
    // Reuse current frame (tail call optimization)
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    // Extract all frame information we'll need BEFORE taking mutable references
    let (base, return_count, result_reg, _pc) = {
        let frame = &vm.frames[vm.frame_count - 1];
        (
            frame.base_ptr as usize,
            frame.get_num_results(),
            frame.get_result_reg(),
            frame.pc,
        )
    };

    // Get function from R[A]
    let func = vm.register_stack[base + a];

    // Determine argument count
    let arg_count = if b == 0 {
        // Use all values from R[A+1] to top
        // IMPORTANT: frame.top is RELATIVE to frame.base_ptr
        let frame = vm.current_frame();
        let args_start_rel = a + 1; // Relative to base
        if (frame.top as usize) > args_start_rel {
            (frame.top as usize) - args_start_rel
        } else {
            0 // No arguments
        }
    } else {
        b - 1
    };

    // Copy arguments to temporary buffer
    let mut args = Vec::with_capacity(arg_count);
    for i in 0..arg_count {
        args.push(vm.register_stack[base + a + 1 + i]);
    }

    // Check if function is Lua or C function
    match func.kind() {
        LuaValueKind::Function => {
            // Lua function: tail call optimization
            let Some(func_id) = func.as_function_id() else {
                return Err(vm.error("not a function".to_string()));
            };
            let Some(func_ref) = vm.object_pool.get_function(func_id) else {
                return Err(vm.error("Invalid function ID".to_string()));
            };
            let max_stack_size = func_ref.chunk.max_stack_size;
            let code_ptr = func_ref.chunk.code.as_ptr();
            let constants_ptr = func_ref.chunk.constants.as_ptr();

            // CRITICAL: Before popping the frame, close all upvalues that point to it
            // This ensures that any closures created in this frame can still access
            // the captured values after the frame is destroyed
            vm.close_upvalues_from(base);

            // Pop current frame (tail call optimization)
            let old_base = base; // Already extracted
            // return_count already extracted
            vm.pop_frame_discard();

            vm.ensure_stack_capacity(old_base + max_stack_size);

            // Copy arguments to frame base
            for (i, arg) in args.iter().enumerate() {
                vm.register_stack[old_base + i] = *arg;
            }

            // Create new frame at same location
            let nresults = if return_count == usize::MAX {
                -1i16
            } else {
                return_count as i16
            };
            let new_frame = LuaCallFrame::new_lua_function(
                func_id,
                code_ptr,
                constants_ptr,
                old_base,
                arg_count,  // top = number of arguments passed
                result_reg, // result_reg from the CALLER (not 0!)
                nresults,
            );

            *frame_ptr_ptr = vm.push_frame(new_frame);

            Ok(())
        }
        LuaValueKind::CFunction => {
            // C function: cannot use tail call optimization
            // Convert to regular call

            // CRITICAL: return_count, result_reg, caller_base already extracted
            // These tell us where to write results in the CALLER's frame
            let args_len = args.len();

            // IMPORTANT: For TAILCALL, we need the PARENT frame's base
            // The current frame IS the tail-calling function
            // When we pop it, we return to the parent
            // result_reg is relative to parent's base

            let Some(c_func) = func.as_cfunction() else {
                return Err(vm.error("not a c function".to_string()));
            };

            // CRITICAL FIX: Use current frame's base for C function call
            // This avoids growing register_stack infinitely on repeated tail calls
            // The current frame's stack space is being reused since this is a tail call
            let call_base = base;
            vm.ensure_stack_capacity(call_base + args_len + 1);

            vm.register_stack[call_base] = func;
            for (i, arg) in args.iter().enumerate() {
                vm.register_stack[call_base + 1 + i] = *arg;
            }

            // Create temporary C function frame
            let temp_frame = LuaCallFrame::new_c_function(call_base, args_len + 1);

            // Push temp frame and call C function
            vm.push_frame(temp_frame);
            // Note: We don't update frame_ptr_ptr here since this is a temporary frame
            let result = c_func(vm)?;
            vm.pop_frame_discard(); // Pop temp frame

            // NOW pop the tail-calling function's frame
            vm.pop_frame_discard();

            // Write return values to PARENT frame
            // CRITICAL: result_reg is relative to PARENT's base_ptr!
            if !vm.frames_is_empty() {
                // Update frame_ptr to point to parent frame
                *frame_ptr_ptr = vm.current_frame_ptr();

                let parent_base = vm.current_frame().base_ptr as usize;
                let vals = result.all_values();
                let count = if return_count == usize::MAX {
                    vals.len()
                } else {
                    vals.len().min(return_count)
                };

                // Write return values
                for i in 0..count {
                    vm.register_stack[parent_base + result_reg + i] = vals[i];
                }

                // Fill remaining expected values with nil
                if return_count != usize::MAX {
                    for i in count..return_count {
                        vm.register_stack[parent_base + result_reg + i] = LuaValue::nil();
                    }
                }

                // CRITICAL: Update parent frame's top to reflect the number of return values
                // This is essential for variable returns (return_count == usize::MAX)
                vm.current_frame_mut().top = (result_reg + count) as u32;
            }

            Ok(())
        }
        _ => Err(vm.error(format!("attempt to call a {} value", func.type_name()))),
    }
}

/// RETURN0
/// return (no values)
/// OPTIMIZED: Use frame_ptr directly, calculate caller ptr before pop
#[inline(always)]
pub fn exec_return0(
    vm: &mut LuaVM,
    _instr: u32,
    frame_ptr_ptr: &mut *mut LuaCallFrame,
) -> LuaResult<()> {
    // FAST PATH: Use passed frame_ptr directly - get all info BEFORE popping
    let (base_ptr, result_reg, num_results) = unsafe {
        (
            (**frame_ptr_ptr).base_ptr as usize,
            (**frame_ptr_ptr).get_result_reg(),
            (**frame_ptr_ptr).get_num_results(),
        )
    };

    // Only close upvalues if there are any
    if !vm.open_upvalues.is_empty() {
        vm.close_upvalues_from(base_ptr);
    }

    // OPTIMIZED: Calculate caller frame pointer and check if Lua BEFORE pop
    let has_caller = vm.frame_count > 1;
    let (caller_ptr, caller_is_lua) = if has_caller {
        let ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 2) };
        let is_lua = unsafe { (*ptr).is_lua() };
        (ptr, is_lua)
    } else {
        (std::ptr::null_mut(), false)
    };

    // Pop frame - just decrement counter
    vm.frame_count -= 1;

    // FAST PATH: Lua caller (most common case - Lua calling Lua)
    if has_caller && caller_is_lua {
        // Update frame_ptr (already computed)
        *frame_ptr_ptr = caller_ptr;

        // Get caller's base_ptr
        let caller_base = unsafe { (*caller_ptr).base_ptr } as usize;

        // Fill expected return values with nil
        if num_results != usize::MAX && num_results > 0 {
            let dest_base = caller_base + result_reg;
            unsafe {
                let reg_ptr = vm.register_stack.as_mut_ptr();
                let nil_val = LuaValue::nil();
                for i in 0..num_results {
                    *reg_ptr.add(dest_base + i) = nil_val;
                }
            }
        }

        // Update caller's top
        unsafe {
            (*caller_ptr).top = result_reg as u32;
        }
        Ok(())
    } else if has_caller {
        // C function caller (pcall/xpcall/metamethods via call_function_internal)
        // Write to return_values for call_function_internal to read
        *frame_ptr_ptr = caller_ptr;
        vm.return_values.clear();
        Err(LuaError::Exit)
    } else {
        // No caller - exit VM, clear return_values (empty return)
        vm.return_values.clear();
        Err(LuaError::Exit)
    }
}

/// RETURN1 A
/// return R[A]
/// OPTIMIZED: Ultra-fast path for single-value return (most common case)
#[inline(always)]
pub fn exec_return1(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr_ptr: &mut *mut LuaCallFrame,
) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;

    // FAST PATH: Use passed frame_ptr directly - get all info we need
    let (base_ptr, result_reg) = unsafe {
        (
            (**frame_ptr_ptr).base_ptr as usize,
            (**frame_ptr_ptr).get_result_reg(),
        )
    };

    // Get return value BEFORE any other operations
    let return_value = unsafe { *vm.register_stack.get_unchecked(base_ptr + a) };

    // Only close upvalues if there are any open (rare for simple functions)
    if !vm.open_upvalues.is_empty() {
        vm.close_upvalues_from(base_ptr);
    }

    // OPTIMIZED: Calculate caller frame pointer and check if Lua BEFORE pop
    let has_caller = vm.frame_count > 1;
    let (caller_ptr, caller_is_lua) = if has_caller {
        let ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 2) };
        let is_lua = unsafe { (*ptr).is_lua() };
        (ptr, is_lua)
    } else {
        (std::ptr::null_mut(), false)
    };

    // Pop frame - just decrement counter
    vm.frame_count -= 1;

    // FAST PATH: Lua caller (most common - Lua calling Lua)
    if has_caller && caller_is_lua {
        // Update frame_ptr to caller (already computed above)
        *frame_ptr_ptr = caller_ptr;

        // Get caller's base_ptr and write return value directly
        let caller_base = unsafe { (*caller_ptr).base_ptr } as usize;
        unsafe {
            *vm.register_stack
                .get_unchecked_mut(caller_base + result_reg) = return_value;
            // Update top
            (*caller_ptr).top = (result_reg + 1) as u32;
        }

        Ok(())
    } else if has_caller {
        // C function caller (pcall/xpcall/metamethods via call_function_internal)
        // Write to return_values for call_function_internal to read
        *frame_ptr_ptr = caller_ptr;
        vm.return_values.clear();
        vm.return_values.push(return_value);
        Err(LuaError::Exit)
    } else {
        // No caller - exit VM (only happens at script end)
        vm.return_values.clear();
        vm.return_values.push(return_value);
        Err(LuaError::Exit)
    }
}
