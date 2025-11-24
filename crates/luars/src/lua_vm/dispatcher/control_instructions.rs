/// Control flow instructions
///
/// These instructions handle function calls, returns, jumps, and coroutine operations.
use crate::LuaValue;
use crate::lua_value::LuaValueKind;
use crate::lua_vm::{Instruction, LuaCallFrame, LuaError, LuaResult, LuaVM};

/// RETURN A B C k
/// return R[A], ... ,R[A+B-2]
#[inline(always)]
pub fn exec_return(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let _c = Instruction::get_c(instr) as usize;
    let _k = Instruction::get_k(instr);

    // Close upvalues before popping the frame
    let base_ptr = vm.current_frame().base_ptr;
    vm.close_upvalues_from(base_ptr);

    let frame = vm
        .frames
        .pop()
        .ok_or_else(|| LuaError::RuntimeError("RETURN with no frame on stack".to_string()))?;

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
    if !vm.frames.is_empty() {
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
        if let Some(func_ptr) = caller_frame.function_value.as_function_ptr() {
            let caller_max_stack = unsafe { (*func_ptr).borrow().chunk.max_stack_size };
            let caller_end = caller_frame.base_ptr + caller_max_stack;

            // 保留返回值所需的空间
            let needed_end = dest_end.max(caller_end);
            if vm.register_stack.len() > needed_end {
                vm.register_stack.truncate(needed_end);
            }
        }
    }

    // Handle upvalue closing (k bit)
    if _k {
        let close_from = base_ptr + a;
        vm.close_upvalues_from(close_from);
    }

    Ok(())
}

// ============ Jump Instructions ============

/// JMP sJ
/// pc += sJ
#[inline(always)]
pub fn exec_jmp(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let sj = Instruction::get_sj(instr);

    let frame = vm.current_frame_mut();
    // PC already incremented by dispatcher, so we add offset directly
    frame.pc = (frame.pc as i32 + sj) as usize;

    Ok(())
}

// ============ Test Instructions ============

/// TEST A k
/// if (not R[A] == k) then pc++
#[inline(always)]
pub fn exec_test(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access (hot path)
    let value = unsafe { *vm.register_stack.as_ptr().add(base_ptr + a) };

    // Lua truthiness: nil and false are falsy, everything else is truthy
    let is_truthy = !value.is_nil() && value.as_bool().unwrap_or(true);

    // If (not value) == k, skip next instruction
    if !is_truthy == k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// TESTSET A B k
/// if (not R[B] == k) then R[A] := R[B] else pc++
#[inline(always)]
pub fn exec_testset(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access
    let value = unsafe { *vm.register_stack.as_ptr().add(base_ptr + b) };

    // Lua truthiness: not l_isfalse(v) means v is truthy
    let is_truthy = !value.is_nil() && value.as_bool().unwrap_or(true);

    // TESTSET: if ((not l_isfalse(R[B])) == k) then R[A] := R[B] else pc++
    // If (is_truthy == k), assign R[A] = R[B], otherwise skip next instruction
    if is_truthy == k {
        unsafe {
            *vm.register_stack.as_mut_ptr().add(base_ptr + a) = value;
        }
    } else {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

// ============ Comparison Instructions ============

/// EQ A B k
/// if ((R[A] == R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_eq(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access
    let (left, right) = unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        (*reg_base.add(a), *reg_base.add(b))
    };

    let mut is_equal = left == right;

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
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// LT A B k
/// if ((R[A] < R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_lt(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access (hot path)
    let (left, right) = unsafe {
        let reg_base = vm.register_stack.as_ptr().add(base_ptr);
        (*reg_base.add(a), *reg_base.add(b))
    };

    // OPTIMIZATION: Direct type tag comparison (avoid method calls)
    use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
    let left_tag = left.primary;
    let right_tag = right.primary;

    let is_less = if (left_tag & TYPE_MASK) == TAG_INTEGER && (right_tag & TYPE_MASK) == TAG_INTEGER
    {
        // Fast integer path - compare secondary values directly
        (left.secondary as i64) < (right.secondary as i64)
    } else if let (Some(l), Some(r)) = (left.as_number(), right.as_number()) {
        l < r
    } else if left.is_string() && right.is_string() {
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
                            vm.current_frame_mut().pc += 1;
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
                                vm.current_frame_mut().pc += 1;
                            }
                            return Ok(());
                        }
                        found_metamethod = true;
                    }
                }
            }
        }

        if !found_metamethod {
            return Err(LuaError::RuntimeError(format!(
                "attempt to compare {} with {}",
                left.type_name(),
                right.type_name()
            )));
        }
        return Ok(());
    };

    if is_less != k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// LE A B k
/// if ((R[A] <= R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_le(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

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
                            vm.current_frame_mut().pc += 1;
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
                                vm.current_frame_mut().pc += 1;
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
                                vm.current_frame_mut().pc += 1;
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
                                    vm.current_frame_mut().pc += 1;
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
            return Err(LuaError::RuntimeError(format!(
                "attempt to compare {} with {}",
                left.type_name(),
                right.type_name()
            )));
        }
        return Ok(());
    };

    if is_less_or_equal != k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// EQK A B k
/// if ((R[A] == K[B]) ~= k) then pc++
pub fn exec_eqk(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    let func_ptr = frame
        .get_function_ptr()
        .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
    let func = unsafe { &*func_ptr };
    let constant = func
        .borrow()
        .chunk
        .constants
        .get(b)
        .copied()
        .ok_or_else(|| LuaError::RuntimeError(format!("Invalid constant index: {}", b)))?;

    let left = vm.register_stack[base_ptr + a];

    let is_equal = left == constant;

    if is_equal != k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// EQI A sB k
/// if ((R[A] == sB) ~= k) then pc++
#[inline(always)]
pub fn exec_eqi(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    let left = unsafe { *vm.register_stack.as_ptr().add(base_ptr + a) };

    use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
    let is_equal = if (left.primary & TYPE_MASK) == TAG_INTEGER {
        (left.secondary as i64) == (sb as i64)
    } else if let Some(l) = left.as_number() {
        l == sb as f64
    } else {
        false
    };

    if is_equal != k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// LTI A sB k
/// if ((R[A] < sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lti(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access
    let left = unsafe { *vm.register_stack.as_ptr().add(base_ptr + a) };

    // OPTIMIZATION: Direct type tag comparison
    use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
    let is_less = if (left.primary & TYPE_MASK) == TAG_INTEGER {
        // Fast integer path
        (left.secondary as i64) < (sb as i64)
    } else if let Some(l) = left.as_number() {
        l < sb as f64
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with number",
            left.type_name()
        )));
    };

    if is_less != k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// LEI A sB k
/// if ((R[A] <= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lei(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    let left = unsafe { *vm.register_stack.as_ptr().add(base_ptr + a) };

    use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
    let is_less_equal = if (left.primary & TYPE_MASK) == TAG_INTEGER {
        (left.secondary as i64) <= (sb as i64)
    } else if let Some(l) = left.as_number() {
        l <= sb as f64
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with number",
            left.type_name()
        )));
    };

    if is_less_equal != k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// GTI A sB k
/// if ((R[A] > sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gti(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    let left = unsafe { *vm.register_stack.as_ptr().add(base_ptr + a) };

    use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
    let is_greater = if (left.primary & TYPE_MASK) == TAG_INTEGER {
        (left.secondary as i64) > (sb as i64)
    } else if let Some(l) = left.as_number() {
        l > sb as f64
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with number",
            left.type_name()
        )));
    };

    if is_greater != k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

/// GEI A sB k
/// if ((R[A] >= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gei(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    let left = unsafe { *vm.register_stack.as_ptr().add(base_ptr + a) };

    use crate::lua_value::{TAG_INTEGER, TYPE_MASK};
    let is_greater_equal = if (left.primary & TYPE_MASK) == TAG_INTEGER {
        (left.secondary as i64) >= (sb as i64)
    } else if let Some(l) = left.as_number() {
        l >= sb as f64
    } else {
        return Err(LuaError::RuntimeError(format!(
            "attempt to compare {} with number",
            left.type_name()
        )));
    };

    if is_greater_equal != k {
        vm.current_frame_mut().pc += 1;
    }

    Ok(())
}

// ============ Call Instructions ============

/// CALL A B C
/// R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
#[inline(always)]
pub fn exec_call(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    use crate::lua_value::LuaValueKind;
    use crate::lua_vm::LuaCallFrame;

    // CALL A B C: R[A], ..., R[A+C-2] := R[A](R[A+1], ..., R[A+B-1])
    // A: function register, B: arg count + 1 (0 = use top), C: return count + 1 (0 = use top)
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let c = Instruction::get_c(instr) as usize;

    let base = {
        let frame = vm.frames.last().unwrap();
        frame.base_ptr
    };

    // Get function from R[A]
    let mut func = vm.register_stack[base + a].clone();
    let mut use_call_metamethod = false;
    let mut call_metamethod_self = LuaValue::nil();

    // Check for __call metamethod if func is not callable
    if !func.is_callable() {
        // Try to get __call metamethod for tables
        if func.kind() == LuaValueKind::Table {
            // First, get the metatable (need to release table_ref before creating string)
            let metatable_opt = func
                .as_lua_table()
                .and_then(|table_ref| table_ref.borrow().get_metatable());

            if let Some(metatable) = metatable_opt {
                let call_key = vm.create_string("__call");
                if let Some(call_func) = vm.table_get_with_meta(&metatable, &call_key) {
                    if call_func.is_callable() {
                        // Use metamethod instead
                        call_metamethod_self = func;
                        func = call_func;
                        use_call_metamethod = true;
                    }
                }
            }
        }

        // If still not callable, error
        if !func.is_callable() {
            return Err(LuaError::RuntimeError(format!(
                "attempt to call a {} value",
                func.type_name()
            )));
        }
    }

    // Determine argument count
    // CRITICAL: In Lua, when B != 0, CALL sets its own top (doesn't use frame.top)
    // When B == 0, it uses the top set by the previous instruction (e.g., another CALL)
    let arg_count = if b == 0 {
        // Use all values from R[A+1] to current top
        // This top was set by the previous instruction (usually a CALL with C=0)
        let frame = vm.current_frame();
        if frame.top > a + 1 {
            frame.top - (a + 1)
        } else {
            0
        }
    } else {
        // Fixed argument count: B-1
        // We IGNORE frame.top here - B specifies the exact number of arguments
        // AND we update frame.top to reflect the call boundary
        // This matches Lua's: L->top.p = ra + b
        vm.current_frame_mut().top = a + b;
        b - 1
    };

    // Determine expected return count
    let return_count = if c == 0 {
        usize::MAX // Want all return values
    } else {
        c - 1
    };

    match func.kind() {
        LuaValueKind::CFunction => {
            // Call C function immediately - Lua-style: NO COPY!
            let cfunc = func.as_cfunction().unwrap();

            // === CRITICAL: 完全模仿Lua的precallC - 在原位调用 ===
            // Lua: L->ci = ci = prepCallInfo(L, func, nresults, CIST_C, L->top.p + LUA_MINSTACK);
            // 我们：直接把frame的base_ptr设置为 R[A] 的位置，不复制任何东西！

            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;

            // C函数的调用栈：R[A] = func, R[A+1..A+B-1] = args
            // 直接在这个位置创建frame，不需要复制到新地址！
            let call_base = base + a;

            // Handle __call metamethod: need to insert self as first argument
            if use_call_metamethod {
                // Shift arguments right by 1 to make room for self
                // R[A+1] = original table (self)
                // R[A+2..A+B] = original R[A+1..A+B-1]
                if arg_count > 0 {
                    // Move arguments: from back to front to avoid overwriting
                    for i in (0..arg_count).rev() {
                        vm.register_stack[call_base + 2 + i] =
                            vm.register_stack[call_base + 1 + i].clone();
                    }
                }
                vm.register_stack[call_base + 1] = call_metamethod_self;
                vm.register_stack[call_base] = func; // Replace original table with __call function
            }
            // else: 参数已经在正确位置了！无需任何操作！

            let actual_arg_count = if use_call_metamethod {
                arg_count + 1
            } else {
                arg_count
            };

            // 确保栈足够（Lua: checkstackGCp(L, LUA_MINSTACK, func)）
            let required_top = call_base + actual_arg_count + 1 + 20; // +20 for C function working space
            vm.ensure_stack_capacity(required_top);

            // Push C function frame - 注意：base_ptr = call_base (R[A]的位置)
            let temp_frame = LuaCallFrame::new_c_function(
                frame_id,
                vm.current_frame().function_value,
                vm.current_frame().pc,
                call_base,
                actual_arg_count + 1, // +1 for function itself
            );

            vm.frames.push(temp_frame);

            // Lua: n = (*f)(L);  // 直接调用
            let result = match cfunc(vm) {
                Ok(r) => Ok(r),
                Err(LuaError::Yield(values)) => {
                    vm.frames.pop();
                    return Err(LuaError::Yield(values));
                }
                Err(e) => {
                    vm.frames.pop();
                    return Err(e);
                }
            };
            vm.frames.pop();
            let result = result?;

            // === 关键改进：返回值已经在正确位置了！===
            // C函数通过vm.push()写入返回值，已经在register_stack的末尾
            // 但CALL指令期望返回值在 R[A], R[A+1], ...
            // 所以我们需要把返回值从栈顶移动到R[A]
            let values = result.all_values();
            let num_returns = if return_count == usize::MAX {
                values.len()
            } else {
                return_count.min(values.len())
            };

            // OPTIMIZATION: 只在返回值不在正确位置时才复制
            if num_returns > 0 {
                unsafe {
                    let src_ptr = values.as_ptr();
                    let dst_ptr = vm.register_stack.as_mut_ptr().add(call_base);
                    std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, num_returns);
                }
            }

            // Fill remaining with nil if needed
            if return_count != usize::MAX {
                for i in num_returns..return_count {
                    vm.register_stack[call_base + i] = crate::LuaValue::nil();
                }
            }

            // Update caller's top
            vm.current_frame_mut().top = a + num_returns;

            Ok(())
        }
        LuaValueKind::Function => {
            // OPTIMIZATION: Direct pointer access - NO hash lookup!
            let func_ptr = func
                .as_function_ptr()
                .ok_or_else(|| LuaError::RuntimeError("Invalid function pointer".to_string()))?;
            let (max_stack_size, is_vararg) = unsafe {
                let func_borrow = (*func_ptr).borrow();
                let size = if func_borrow.chunk.max_stack_size == 0 {
                    1
                } else {
                    func_borrow.chunk.max_stack_size
                };
                let vararg = func_borrow.chunk.is_vararg;
                (size, vararg)
            }; // Borrow released immediately

            // Create new frame
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;

            // === 零拷贝关键设计 ===
            // Lua 5.4: 新frame的base直接指向 caller_base + a + 1
            // 参数已经在 R[A+1], R[A+2], ... 的位置了！
            // 只需要确保栈足够大，无需复制参数！

            let caller_base = base;

            // 零拷贝：新frame的base = R[A+1] 的位置
            // R[A] = func (不属于新frame)
            // R[A+1] = 第一个参数 = 新frame的 R[0]
            let new_base = caller_base + a + 1;

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

            // 确保栈容量足够
            let required_capacity = new_base + total_stack_size;
            if vm.register_stack.len() < required_capacity {
                vm.ensure_stack_capacity(required_capacity);
                vm.register_stack.resize(required_capacity, LuaValue::nil());
            }

            // 零拷贝！只需要处理特殊情况
            unsafe {
                let reg_ptr = vm.register_stack.as_mut_ptr();
                let nil_val = LuaValue::nil();

                // 参数已经在 new_base 位置了！
                // 只需要初始化 new_base + arg_count 之后的局部变量槽位
                if actual_arg_count < actual_stack_size {
                    for i in actual_arg_count..actual_stack_size {
                        *reg_ptr.add(new_base + i) = nil_val;
                    }
                }

                // Vararg: 需要额外空间
                if is_vararg && actual_arg_count > 0 {
                    for i in actual_stack_size..total_stack_size {
                        *reg_ptr.add(new_base + i) = nil_val;
                    }
                }

                // __call metamethod: 需要插入 self 作为第一个参数
                if use_call_metamethod {
                    // 参数右移一位：R[0] <- self, R[1] <- 原R[0], R[2] <- 原R[1], ...
                    if arg_count > 0 {
                        // 从后往前复制，避免覆盖
                        for i in (0..arg_count).rev() {
                            *reg_ptr.add(new_base + 1 + i) = *reg_ptr.add(new_base + i);
                        }
                    }
                    *reg_ptr.add(new_base) = call_metamethod_self;
                }
            }

            // Get code pointer from function
            let code_ptr = unsafe { (*func_ptr).borrow().chunk.code.as_ptr() };

            // Create and push new frame
            let new_frame = LuaCallFrame::new_lua_function(
                frame_id,
                func,
                code_ptr,
                new_base,         // 零拷贝：直接指向参数位置！
                actual_arg_count, // top = number of arguments passed
                a,                // result_reg: where to store return values (相对于caller的base)
                return_count,
            );

            vm.frames.push(new_frame);

            Ok(())
        }
        _ => Err(LuaError::RuntimeError(format!(
            "attempt to call a {} value",
            func.type_name()
        ))),
    }
}

/// TAILCALL A B C k
/// return R[A](R[A+1], ... ,R[A+B-1])
pub fn exec_tailcall(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    // TAILCALL A B C: return R[A](R[A+1], ..., R[A+B-1])
    // Reuse current frame (tail call optimization)
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;

    // Extract all frame information we'll need BEFORE taking mutable references
    let (base, return_count, result_reg, function_value, pc) = {
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

            let func_ptr = func
                .as_function_ptr()
                .ok_or_else(|| LuaError::RuntimeError("Invalid function pointer".to_string()))?;
            let max_stack_size = unsafe { (*func_ptr).borrow().chunk.max_stack_size };

            // CRITICAL: Before popping the frame, close all upvalues that point to it
            // This ensures that any closures created in this frame can still access
            // the captured values after the frame is destroyed
            vm.close_upvalues_from(base);

            // Pop current frame (tail call optimization)
            let old_base = base; // Already extracted
            // return_count already extracted
            vm.frames.pop();

            // Create new frame at same location
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;

            vm.ensure_stack_capacity(old_base + max_stack_size);

            // Copy arguments to frame base
            for (i, arg) in args.iter().enumerate() {
                vm.register_stack[old_base + i] = *arg;
            }

            // Get code pointer from function
            let func_ptr = func
                .as_function_ptr()
                .ok_or_else(|| LuaError::RuntimeError("Not a Lua function".to_string()))?;
            let func_obj = unsafe { &*func_ptr };
            let code_ptr = func_obj.borrow().chunk.code.as_ptr();

            let new_frame = LuaCallFrame::new_lua_function(
                frame_id,
                func,
                code_ptr,
                old_base,
                arg_count,  // top = number of arguments passed
                result_reg, // result_reg from the CALLER (not 0!)
                return_count,
            );
            vm.frames.push(new_frame);

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

            let c_func = func
                .as_cfunction()
                .ok_or_else(|| LuaError::RuntimeError("Invalid C function".to_string()))?;

            // Create temporary C function frame
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;

            // Set up arguments in a temporary stack space
            let call_base = vm.register_stack.len();
            vm.ensure_stack_capacity(call_base + args_len + 1);

            vm.register_stack[call_base] = func;
            for (i, arg) in args.iter().enumerate() {
                vm.register_stack[call_base + 1 + i] = *arg;
            }

            let temp_frame =
                LuaCallFrame::new_c_function(frame_id, function_value, pc, call_base, args_len + 1);

            // Push temp frame and call C function
            vm.frames.push(temp_frame);
            let result = c_func(vm)?;
            vm.frames.pop(); // Pop temp frame

            // NOW pop the tail-calling function's frame
            vm.frames.pop();

            // Write return values to PARENT frame
            // CRITICAL: result_reg is relative to PARENT's base_ptr!
            if !vm.frames.is_empty() {
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
        _ => Err(LuaError::RuntimeError(format!(
            "attempt to call a {} value",
            func.type_name()
        ))),
    }
}

/// RETURN0
/// return (no values)
pub fn exec_return0(vm: &mut LuaVM, _instr: u32) -> LuaResult<()> {
    // Close upvalues before popping the frame
    let base_ptr = vm.current_frame().base_ptr;
    vm.close_upvalues_from(base_ptr);

    let frame = vm
        .frames
        .pop()
        .ok_or_else(|| LuaError::RuntimeError("RETURN0 with no frame on stack".to_string()))?;

    vm.return_values.clear();

    // IMPORTANT: For RETURN0, we need to fill the result register(s) with nil
    // if the caller expects return values (num_results > 0)
    if !vm.frames.is_empty() {
        let result_reg = frame.get_result_reg();
        let num_results = frame.get_num_results();
        let caller_base = vm.current_frame().base_ptr;

        // Fill expected return values with nil
        if num_results != usize::MAX && num_results > 0 {
            for i in 0..num_results {
                vm.register_stack[caller_base + result_reg + i] = LuaValue::nil();
            }
        }

        // Update caller's top to indicate 0 return values (for variable returns)
        vm.current_frame_mut().top = result_reg; // No return values, so top = result_reg + 0
    }

    Ok(())
}

/// RETURN1 A
/// return R[A]
pub fn exec_return1(vm: &mut LuaVM, instr: u32) -> LuaResult<()> {
    let a = Instruction::get_a(instr) as usize;

    // Close upvalues before popping the frame
    let base_ptr = vm.current_frame().base_ptr;
    vm.close_upvalues_from(base_ptr);

    let frame = vm
        .frames
        .pop()
        .ok_or_else(|| LuaError::RuntimeError("RETURN1 with no frame on stack".to_string()))?;

    let base_ptr = frame.base_ptr;
    let result_reg = frame.get_result_reg();

    vm.return_values.clear();
    if base_ptr + a < vm.register_stack.len() {
        let return_value = vm.register_stack[base_ptr + a];
        vm.return_values.push(return_value);
    }

    // Copy return value to caller's registers if needed
    if !vm.frames.is_empty() {
        let caller_base = vm.current_frame().base_ptr;

        // Ensure destination is valid
        let dest_pos = caller_base + result_reg;
        if vm.register_stack.len() <= dest_pos {
            vm.ensure_stack_capacity(dest_pos + 1);
            vm.register_stack.resize(dest_pos + 1, LuaValue::nil());
        }

        if !vm.return_values.is_empty() {
            vm.register_stack[caller_base + result_reg] = vm.return_values[0];
        }

        // Only update top for normal CALL instructions
        // For internal calls (result_reg >= max_stack), don't modify caller's top
        let caller_frame = vm.current_frame();
        let should_update_top =
            if let Some(func_ptr) = caller_frame.function_value.as_function_ptr() {
                let max_stack = unsafe { (*func_ptr).borrow().chunk.max_stack_size };
                result_reg < max_stack
            } else {
                result_reg < 256
            };

        if should_update_top {
            vm.current_frame_mut().top = result_reg + 1;
        }
    }

    Ok(())
}
