/// Control flow instructions
///
/// These instructions handle function calls, returns, jumps, and coroutine operations.
use crate::LuaValue;
use crate::lua_value::LuaValueKind;
use crate::lua_vm::{Instruction, LuaCallFrame, LuaError, LuaResult, LuaVM};

/// RETURN A B C k
/// return R[A], ... ,R[A+B-2]
#[inline(always)]
pub fn exec_return(vm: &mut LuaVM, instr: u32, frame_ptr_ptr: &mut *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    // let _c = Instruction::get_c(instr) as usize;
    let k = Instruction::get_k(instr);

    // Close upvalues before popping the frame
    let base_ptr = unsafe { (**frame_ptr_ptr).base_ptr };
    vm.close_upvalues_from(base_ptr);

    let Some(frame) = vm.pop_frame() else {
        return Err(vm.error("RETURN with no frame on stack".to_string()));
    };

    let base_ptr = frame.base_ptr;
    let result_reg = frame.get_result_reg();
    let num_results = frame.get_num_results();

    // Calculate return count
    let return_count = if b == 0 {
        frame.top.saturating_sub(a)
    } else {
        b - 1
    };

    // === 零拷贝返回值优化 ===
    // 关键：返回值需要写回 caller 的 R[result_reg]
    // 而不是写到 caller 的栈顶
    if !vm.frames_is_empty() {
        let caller_frame = vm.current_frame();
        let caller_base = caller_frame.base_ptr;

        // 返回值目标位置：caller_base + result_reg
        let dest_base = caller_base + result_reg;

        // 确保目标位置有足够空间
        let dest_end = dest_base + return_count.max(num_results.min(return_count));
        if vm.register_stack.len() < dest_end {
            vm.ensure_stack_capacity(dest_end);
            vm.register_stack.resize(dest_end, LuaValue::nil());
        }

        unsafe {
            let reg_ptr = vm.register_stack.as_mut_ptr();

            if num_results == usize::MAX {
                // 返回所有值
                if return_count > 0 {
                    // 源：base_ptr + a
                    // 目标：caller_base + result_reg
                    // 检查是否重叠
                    let src_start = base_ptr + a;
                    let src_end = src_start + return_count;
                    let dst_start = dest_base;
                    let dst_end = dst_start + return_count;

                    // 如果区域重叠，使用 copy；否则使用 copy_nonoverlapping
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

                // 更新 caller 的 top
                vm.current_frame_mut().top = result_reg + return_count;
            } else {
                // 固定数量的返回值
                let nil_val = LuaValue::nil();
                for i in 0..num_results {
                    let val = if i < return_count {
                        *reg_ptr.add(base_ptr + a + i)
                    } else {
                        nil_val
                    };
                    *reg_ptr.add(dest_base + i) = val;
                }

                // 更新 caller 的 top
                vm.current_frame_mut().top = result_reg + num_results;
            }
        }

        // 截断寄存器栈回到 caller 的范围
        // 零拷贝设计：callee 的栈空间可能与 caller 重叠
        // 需要保留 caller 需要的部分
        let caller_frame = vm.current_frame();
        if let Some(func_id) = caller_frame.function_value.as_function_id() {
            if let Some(func_ref) = vm.object_pool.get_function(func_id) {
                let caller_max_stack = func_ref.chunk.max_stack_size;
                let caller_end = caller_frame.base_ptr + caller_max_stack;

                // 保留返回值所需的空间
                let needed_end = dest_end.max(caller_end);
                if vm.register_stack.len() > needed_end {
                    vm.register_stack.truncate(needed_end);
                }
            }
        }
    }

    // Handle upvalue closing (k bit)
    if k {
        let close_from = base_ptr + a;
        vm.close_upvalues_from(close_from);
        // Also call __close metamethods for to-be-closed variables
        vm.close_to_be_closed(close_from)?;
    }

    // CRITICAL: If frames are now empty, we're done - return control to caller
    // This prevents run() loop from trying to access empty frame
    if vm.frames_is_empty() {
        // Save return values before exiting
        vm.return_values.clear();
        for i in 0..return_count {
            if base_ptr + a + i < vm.register_stack.len() {
                vm.return_values.push(vm.register_stack[base_ptr + a + i]);
            }
        }
        // Signal end of execution
        return Err(LuaError::Exit);
    }

    // Update frame_ptr to point to new current frame
    *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };

    Ok(())
}

// ============ Jump Instructions ============

/// JMP sJ
/// pc += sJ
#[inline(always)]
pub fn exec_jmp(instr: u32, frame_ptr: *mut LuaCallFrame) {
    let sj = Instruction::get_sj(instr);

    unsafe {
        // PC already incremented by dispatcher, so we add offset directly
        (*frame_ptr).pc = ((*frame_ptr).pc as i32 + sj) as usize;
    }
}

// ============ Test Instructions ============

/// TEST A k
/// if (not R[A] == k) then pc++
/// ULTRA-OPTIMIZED: Direct type tag check, single branch
#[inline(always)]
pub fn exec_test(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) {
    let a = Instruction::get_a(instr) as usize;
    let k = Instruction::get_k(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;

        // OPTIMIZATION: Direct unsafe access and type tag comparison
        let value = *vm.register_stack.as_ptr().add(base_ptr + a);

        // OPTIMIZATION: Fast truthiness check using type tags
        // nil = TAG_NIL, false = VALUE_FALSE
        // Only nil and false are falsy
        use crate::lua_value::{TAG_NIL, VALUE_FALSE};
        let is_truthy = value.primary != TAG_NIL && value.primary != VALUE_FALSE;

        // If (not value) == k, skip next instruction
        if !is_truthy == k {
            (*frame_ptr).pc += 1;
        }
    }
}

/// TESTSET A B k
/// if (not R[B] == k) then R[A] := R[B] else pc++
/// ULTRA-OPTIMIZED: Direct type tag check, single branch
#[inline(always)]
pub fn exec_testset(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;

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
            (*frame_ptr).pc += 1;
        }
    }
}

// ============ Comparison Instructions ============

/// EQ A B k
/// if ((R[A] == R[B]) ~= k) then pc++
/// ULTRA-OPTIMIZED: Fast path for common types (integers, floats, strings)
#[inline(always)]
pub fn exec_eq(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = unsafe { (*frame_ptr).base_ptr };

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
        let mm_key = vm.create_string("__eq");

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
        unsafe { (*frame_ptr).pc += 1; }
    }

    Ok(())
}

/// LT A B k
/// if ((R[A] < R[B]) ~= k) then pc++
/// ULTRA-OPTIMIZED: Inline integer/float comparison, minimal type checks
#[inline(always)]
pub fn exec_lt(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = unsafe { (*frame_ptr).base_ptr };

    // OPTIMIZATION: Use unsafe for unchecked register access (hot path)
    let (left, right) = unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        (*reg_base.add(a), *reg_base.add(b))
    };

    // OPTIMIZATION: Direct type tag comparison (inline integer/float checks)
    use crate::lua_value::{TAG_FLOAT, TAG_INTEGER, TAG_STRING, TYPE_MASK};
    let left_tag = left.primary & TYPE_MASK;
    let right_tag = right.primary & TYPE_MASK;

    // Combined type check for fast paths (single branch!)
    let combined_tags = (left_tag << 16) | right_tag;
    const INT_INT: u64 = (TAG_INTEGER << 16) | TAG_INTEGER;
    const FLOAT_FLOAT: u64 = (TAG_FLOAT << 16) | TAG_FLOAT;
    const INT_FLOAT: u64 = (TAG_INTEGER << 16) | TAG_FLOAT;
    const FLOAT_INT: u64 = (TAG_FLOAT << 16) | TAG_INTEGER;
    const STRING_STRING: u64 = (TAG_STRING << 16) | TAG_STRING;

    let is_less = if combined_tags == INT_INT {
        // Fast integer path - single branch!
        (left.secondary as i64) < (right.secondary as i64)
    } else if combined_tags == FLOAT_FLOAT {
        // Fast float path
        f64::from_bits(left.secondary) < f64::from_bits(right.secondary)
    } else if combined_tags == INT_FLOAT {
        // Mixed: integer < float
        ((left.secondary as i64) as f64) < f64::from_bits(right.secondary)
    } else if combined_tags == FLOAT_INT {
        // Mixed: float < integer
        f64::from_bits(left.secondary) < ((right.secondary as i64) as f64)
    } else if combined_tags == STRING_STRING {
        // String comparison
        left < right
    } else {
        // Try __lt metamethod
        let mm_key = vm.create_string("__lt");
        let mut found_metamethod = false;

        if let Some(mt) = vm.table_get_metatable(&left) {
            if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key) {
                if !metamethod.is_nil() {
                    if let Some(result) = vm.call_metamethod(&metamethod, &[left, right])? {
                        let is_less_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                        if is_less_result != k {
                            unsafe { (*frame_ptr).pc += 1; }
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
                            let is_less_result =
                                !result.is_nil() && result.as_bool().unwrap_or(true);
                            if is_less_result != k {
                                unsafe { (*frame_ptr).pc += 1; }
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
        return Ok(());
    };

    if is_less != k {
        unsafe { (*frame_ptr).pc += 1; }
    }

    Ok(())
}

/// LE A B k
/// if ((R[A] <= R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_le(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = unsafe { (*frame_ptr).base_ptr };

    // OPTIMIZATION: Use unsafe for unchecked register access
    let (left, right) = unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        (*reg_base.add(a), *reg_base.add(b))
    };

    // OPTIMIZATION: Direct type tag comparison
    use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
    let is_less_or_equal = if (left.primary & TYPE_MASK) == TAG_INTEGER
        && (right.primary & TYPE_MASK) == TAG_INTEGER
    {
        (left.secondary as i64) <= (right.secondary as i64)
    } else if let (Some(l), Some(r)) = (left.as_number(), right.as_number()) {
        l <= r
    } else if left.is_string() && right.is_string() {
        left <= right
    } else {
        // Try __le metamethod first
        let mm_key_le = vm.create_string("__le");
        let mut found_metamethod = false;

        if let Some(mt) = vm.table_get_metatable(&left) {
            if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key_le) {
                if !metamethod.is_nil() {
                    if let Some(result) = vm.call_metamethod(&metamethod, &[left, right])? {
                        let is_le_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                        if is_le_result != k {
                            unsafe { (*frame_ptr).pc += 1; }
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
                                unsafe { (*frame_ptr).pc += 1; }
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
            let mm_key_lt = vm.create_string("__lt");

            if let Some(mt) = vm.table_get_metatable(&right) {
                if let Some(metamethod) = vm.table_get_with_meta(&mt, &mm_key_lt) {
                    if !metamethod.is_nil() {
                        if let Some(result) = vm.call_metamethod(&metamethod, &[right, left])? {
                            let is_gt_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                            let is_le_result = !is_gt_result; // a <= b is !(b < a)
                            if is_le_result != k {
                                unsafe { (*frame_ptr).pc += 1; }
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
                                    unsafe { (*frame_ptr).pc += 1; }
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
        unsafe { (*frame_ptr).pc += 1; }
    }

    Ok(())
}

/// EQK A B k
/// if ((R[A] == K[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_eqk(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let (base_ptr, func_value) = unsafe { ((*frame_ptr).base_ptr, (*frame_ptr).function_value) };

    // Get function using new ID-based API
    let Some(func_id) = func_value.as_function_id() else {
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
        unsafe { (*frame_ptr).pc += 1; }
    }

    Ok(())
}

/// EQI A sB k
/// if ((R[A] == sB) ~= k) then pc++
#[inline(always)]
pub fn exec_eqi(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
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
            (*frame_ptr).pc += 1;
        }
    }
}

/// LTI A sB k
/// if ((R[A] < sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lti(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
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
            (*frame_ptr).pc += 1;
        }
    }

    Ok(())
}

/// LEI A sB k
/// if ((R[A] <= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lei(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
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
            (*frame_ptr).pc += 1;
        }
    }

    Ok(())
}

/// GTI A sB k
/// if ((R[A] > sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gti(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
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
            (*frame_ptr).pc += 1;
        }
    }

    Ok(())
}

/// GEI A sB k
/// if ((R[A] >= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gei(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    unsafe {
        let base_ptr = (*frame_ptr).base_ptr;
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
            (*frame_ptr).pc += 1;
        }
    }

    Ok(())
}

// ============ Call Instructions ============

/// CALL A B C
/// R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
/// ULTRA-OPTIMIZED: Minimize overhead for the common case (Lua function, no metamethod)
#[inline(always)]
pub fn exec_call(vm: &mut LuaVM, instr: u32, frame_ptr_ptr: &mut *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    // OPTIMIZATION: Use passed frame_ptr directly - avoid Vec lookup!
    let (base, func) = unsafe {
        let base = (**frame_ptr_ptr).base_ptr;
        let func = *vm.register_stack.get_unchecked(base + a);
        (base, func)
    };

    // OPTIMIZATION: Fast path for Lua functions (most common case)
    // Check type tag directly without full pattern match
    use crate::lua_value::TAG_FUNCTION;
    if (func.primary & crate::lua_value::TYPE_MASK) == TAG_FUNCTION {
        return exec_call_lua_function(vm, func, a, b, c, base, false, LuaValue::nil(), frame_ptr_ptr);
    }

    // Check for CFunction
    if func.is_cfunction() {
        return exec_call_cfunction(vm, func, a, b, c, base, false, LuaValue::nil(), frame_ptr_ptr);
    }

    // Slow path: Check for __call metamethod
    if func.kind() == LuaValueKind::Table {
        let metatable_opt = func.as_table_id().and_then(|table_id| {
            vm.object_pool
                .get_table(table_id)
                .and_then(|t| t.get_metatable())
        });

        if let Some(metatable) = metatable_opt {
            let call_key = vm.create_string("__call");
            if let Some(call_func) = vm.table_get_with_meta(&metatable, &call_key) {
                if call_func.is_callable() {
                    if call_func.is_cfunction() {
                        return exec_call_cfunction(vm, call_func, a, b, c, base, true, func, frame_ptr_ptr);
                    } else {
                        return exec_call_lua_function(
                            vm, call_func, a, b, c, base, true, func, frame_ptr_ptr,
                        );
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
    // Get function ID - we already verified this is a function in exec_call
    let func_id = unsafe { func.as_function_id().unwrap_unchecked() };

    // Extract chunk info from ObjectPool - use unchecked for speed
    let func_ref = unsafe { vm.object_pool.get_function_unchecked(func_id) };

    // Cache chunk pointer to avoid Rc deref overhead
    let chunk = &*func_ref.chunk;
    let max_stack_size = chunk.max_stack_size;
    let is_vararg = chunk.is_vararg;
    let code_ptr = chunk.code.as_ptr();

    // FAST PATH: No metamethod, no vararg (most common case)
    // Check this first to allow better branch prediction
    if !use_call_metamethod && !is_vararg {
        // Calculate argument count
        // For fixed args (b != 0), this is just b - 1
        // Use branchless where possible
        let arg_count = if b != 0 {
            unsafe { (**frame_ptr_ptr).top = a + b; }
            b - 1
        } else {
            unsafe { (**frame_ptr_ptr).top.saturating_sub(a + 1) }
        };

        // Zero-copy: new frame base = R[A+1]
        let new_base = caller_base + a + 1;
        let required_capacity = new_base + max_stack_size;

        // Only check capacity if max_stack_size > 0 (skip for empty functions)
        if max_stack_size > 0 {
            if vm.register_stack.len() < required_capacity {
                vm.register_stack.reserve(required_capacity - vm.register_stack.len());
                unsafe { vm.register_stack.set_len(required_capacity); }
            }
            // Initialize locals beyond arguments
            if arg_count < max_stack_size {
                unsafe {
                    let reg_ptr = vm.register_stack.as_mut_ptr().add(new_base);
                    let nil_val = LuaValue::nil();
                    for i in arg_count..max_stack_size {
                        *reg_ptr.add(i) = nil_val;
                    }
                }
            }
        }

        // Create and push new frame
        // Use direct comparison instead of conditional for nresults
        let nresults = if c == 0 { -1i16 } else { (c - 1) as i16 };
        let new_frame = LuaCallFrame::new_lua_function(
            func,
            code_ptr,
            new_base,
            arg_count,
            a,
            nresults,
        );
        vm.push_frame(new_frame);
        *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };
        return Ok(());
    }

    // Calculate argument count for slow path
    let arg_count = if b == 0 {
        unsafe { (**frame_ptr_ptr).top.saturating_sub(a + 1) }
    } else {
        unsafe { (**frame_ptr_ptr).top = a + b; }
        b - 1
    };

    let return_count = if c == 0 { usize::MAX } else { c - 1 };
    let new_base = caller_base + a + 1;

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
    let nresults = if return_count == usize::MAX { -1i16 } else { return_count as i16 };
    let new_frame = LuaCallFrame::new_lua_function(
        func,
        code_ptr,
        new_base,
        actual_arg_count,  // top = number of arguments
        a,                 // result_reg
        nresults,
    );

    vm.push_frame(new_frame);
    // Update frame_ptr to point to new frame
    *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };
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
        if frame.top > a + 1 {
            frame.top - (a + 1)
        } else {
            0
        }
    } else {
        vm.current_frame_mut().top = a + b;
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
    let temp_frame = LuaCallFrame::new_c_function(
        call_base,
        actual_arg_count + 1,
    );

    vm.push_frame(temp_frame);

    // Call C function
    let result = match cfunc(vm) {
        Ok(r) => r,
        Err(LuaError::Yield) => {
            vm.pop_frame_discard();
            // Update frame_ptr after pop (safely check for empty frames)
            if vm.frame_count > 0 {
                *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };
            }
            return Err(LuaError::Yield);
        }
        Err(e) => {
            vm.pop_frame_discard();
            // Update frame_ptr after pop (safely check for empty frames)
            if vm.frame_count > 0 {
                *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };
            }
            return Err(e);
        }
    };
    vm.pop_frame_discard();
    // CRITICAL: Update frame_ptr after C function call returns
    // The C function may have called Lua code which could have reallocated the frames Vec
    if vm.frame_count > 0 {
        *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };
    }

    // Copy return values
    let values = result.all_values();
    let num_returns = if return_count == usize::MAX {
        values.len()
    } else {
        return_count.min(values.len())
    };

    if num_returns > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(
                values.as_ptr(),
                vm.register_stack.as_mut_ptr().add(call_base),
                num_returns,
            );
        }
    }

    // Fill remaining with nil
    if return_count != usize::MAX {
        for i in num_returns..return_count {
            vm.register_stack[call_base + i] = crate::LuaValue::nil();
        }
    }

    vm.current_frame_mut().top = a + num_returns;
    Ok(())
}

/// TAILCALL A B C k
/// return R[A](R[A+1], ... ,R[A+B-1])
pub fn exec_tailcall(vm: &mut LuaVM, instr: u32, frame_ptr_ptr: &mut *mut LuaCallFrame) -> LuaResult<()> {
    // TAILCALL A B C: return R[A](R[A+1], ..., R[A+B-1])
    // Reuse current frame (tail call optimization)
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    // Extract all frame information we'll need BEFORE taking mutable references
    let (base, return_count, result_reg, _function_value, _pc) = {
        let frame = vm.frames.last().unwrap();
        (
            frame.base_ptr,
            frame.get_num_results(),
            frame.get_result_reg(),
            frame.function_value,
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
        if frame.top > args_start_rel {
            frame.top - args_start_rel
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
            let nresults = if return_count == usize::MAX { -1i16 } else { return_count as i16 };
            let new_frame = LuaCallFrame::new_lua_function(
                func,
                code_ptr,
                old_base,
                arg_count,  // top = number of arguments passed
                result_reg, // result_reg from the CALLER (not 0!)
                nresults,
            );
            vm.push_frame(new_frame);

            // Update frame_ptr to point to new frame
            *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };

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

            // Set up arguments in a temporary stack space
            let call_base = vm.register_stack.len();
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
                *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };

                let parent_base = vm.current_frame().base_ptr;
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
                vm.current_frame_mut().top = result_reg + count;
            }

            Ok(())
        }
        _ => Err(vm.error(format!("attempt to call a {} value", func.type_name()))),
    }
}

/// RETURN0
/// return (no values)
/// OPTIMIZED: Use frame_ptr directly
#[inline(always)]
pub fn exec_return0(vm: &mut LuaVM, _instr: u32, frame_ptr_ptr: &mut *mut LuaCallFrame) -> LuaResult<()> {
    // FAST PATH: Use passed frame_ptr directly - get all info BEFORE popping
    let (base_ptr, result_reg, num_results) = unsafe {
        (
            (**frame_ptr_ptr).base_ptr,
            (**frame_ptr_ptr).get_result_reg(),
            (**frame_ptr_ptr).get_num_results(),
        )
    };

    // Only close upvalues if there are any (rare for simple functions)
    if !vm.open_upvalues.is_empty() {
        vm.close_upvalues_from(base_ptr);
    }

    vm.pop_frame_discard();

    // FAST PATH: Check if we have a caller frame
    if vm.frame_count > 0 {
        // Update frame_ptr to point to caller frame
        *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };

        // Fill expected return values with nil (only if caller expects results)
        // Most common case: num_results == 0, skip entirely
        if num_results > 0 && num_results != usize::MAX {
            let caller_base = unsafe { (**frame_ptr_ptr).base_ptr };
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
        unsafe { (**frame_ptr_ptr).top = result_reg; }
        Ok(())
    } else {
        // No caller - exit VM
        vm.return_values.clear();
        Err(LuaError::Exit)
    }
}

/// RETURN1 A
/// return R[A]
/// OPTIMIZED: Fast path for single-value return (most common case)
#[inline(always)]
pub fn exec_return1(vm: &mut LuaVM, instr: u32, frame_ptr_ptr: &mut *mut LuaCallFrame) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;

    // FAST PATH: Use passed frame_ptr directly - get all info we need
    let (base_ptr, result_reg) = unsafe { ((**frame_ptr_ptr).base_ptr, (**frame_ptr_ptr).get_result_reg()) };

    // Only close upvalues if there are any open (rare for simple functions)
    if !vm.open_upvalues.is_empty() {
        vm.close_upvalues_from(base_ptr);
    }

    // Get return value before popping frame - use unchecked for speed
    let return_value = unsafe { *vm.register_stack.get_unchecked(base_ptr + a) };

    // Pop frame - we already have all info we need from frame_ptr
    vm.pop_frame_discard();

    // Check if there's a caller frame
    if vm.frame_count > 0 {
        // Update frame_ptr to point to caller frame
        *frame_ptr_ptr = unsafe { vm.frames.as_mut_ptr().add(vm.frame_count - 1) };

        // Get caller's base_ptr and write return value directly
        let caller_base = unsafe { (**frame_ptr_ptr).base_ptr };
        unsafe {
            *vm.register_stack.get_unchecked_mut(caller_base + result_reg) = return_value;
            (**frame_ptr_ptr).top = result_reg + 1;
        }

        Ok(())
    } else {
        // No caller - exit VM (only happens at script end)
        // Set return_values for external callers
        vm.return_values.clear();
        vm.return_values.push(return_value);
        Err(LuaError::Exit)
    }
}
