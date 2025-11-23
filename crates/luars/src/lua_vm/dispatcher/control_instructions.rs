use crate::lua_value::LuaValueKind;
/// Control flow instructions
/// 
/// These instructions handle function calls, returns, jumps, and coroutine operations.

use crate::LuaValue;
use crate::lua_vm::{Instruction, LuaCallFrame, LuaError, LuaResult, LuaVM};
use super::DispatchAction;

/// RETURN A B C k
/// return R[A], ... ,R[A+B-2]
#[inline(always)]
pub fn exec_return(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let _c = Instruction::get_c(instr) as usize;
    let _k = Instruction::get_k(instr);

    let frame = vm.frames.pop().ok_or_else(|| {
        LuaError::RuntimeError("RETURN with no frame on stack".to_string())
    })?;

    let base_ptr = frame.base_ptr;
    let result_reg = frame.get_result_reg();
    let num_results = frame.get_num_results();
    
    // OPTIMIZATION: Calculate return count once
    let return_count = if b == 0 {
        frame.top.saturating_sub(a)
    } else {
        b - 1
    };

    // OPTIMIZATION: Direct copy to caller's registers using unsafe (skip intermediate vec)
    if !vm.frames.is_empty() {
        let caller_frame = vm.current_frame();
        let caller_base = caller_frame.base_ptr;
        
        // Ensure destination has enough capacity before unsafe write
        let dest_end = caller_base + result_reg + return_count;
        if vm.register_stack.len() < dest_end {
            vm.ensure_stack_capacity(dest_end);
            vm.register_stack.resize(dest_end, LuaValue::nil());
        }
        
        unsafe {
            let reg_ptr = vm.register_stack.as_mut_ptr();
            
            if num_results == usize::MAX {
                // Copy all return values
                let count = return_count.min(vm.register_stack.len().saturating_sub(base_ptr + a));
                if count > 0 {
                    std::ptr::copy_nonoverlapping(
                        reg_ptr.add(base_ptr + a),
                        reg_ptr.add(caller_base + result_reg),
                        count
                    );
                }
                // Only update top if result_reg is within caller's normal range
                let caller_frame = vm.current_frame();
                let should_update_top = if let Some(func_ptr) = caller_frame.function_value.as_function_ptr() {
                    let max_stack = unsafe { (*func_ptr).borrow().chunk.max_stack_size };
                    result_reg < max_stack
                } else {
                    result_reg < 256
                };
                if should_update_top {
                    vm.current_frame_mut().top = result_reg + count;
                }
            } else {
                // Fixed number of return values
                let nil_val = LuaValue::nil();
                for i in 0..num_results {
                    let val = if i < return_count {
                        *reg_ptr.add(base_ptr + a + i)
                    } else {
                        nil_val
                    };
                    *reg_ptr.add(caller_base + result_reg + i) = val;
                }
                // Only update top for normal CALL instructions
                let caller_frame = vm.current_frame();
                let should_update_top = if let Some(func_ptr) = caller_frame.function_value.as_function_ptr() {
                    let max_stack = unsafe { (*func_ptr).borrow().chunk.max_stack_size };
                    result_reg < max_stack
                } else {
                    result_reg < 256
                };
                if should_update_top {
                    vm.current_frame_mut().top = result_reg + num_results;
                }
            }
        }
        
        // Truncate register stack back to caller's frame
        let caller_frame = vm.current_frame();
        // OPTIMIZATION: Use direct pointer access instead of hash lookup
        if let Some(func_ptr) = caller_frame.function_value.as_function_ptr() {
            let caller_max_stack = unsafe { (*func_ptr).borrow().chunk.max_stack_size };
            let caller_base = caller_frame.base_ptr;
            let caller_stack_end = caller_base + caller_max_stack;
            
            if vm.register_stack.len() < caller_stack_end {
                vm.ensure_stack_capacity(caller_stack_end);
            }
            vm.register_stack.truncate(caller_stack_end);
        }
    }

    // Handle upvalue closing (k bit)
    // k=1 means close upvalues >= R[A]
    if _k {
        let close_from = base_ptr + a;
        vm.close_upvalues_from(close_from);
    }

    Ok(DispatchAction::Return)
}

// ============ Jump Instructions ============

/// JMP sJ
/// pc += sJ
#[inline(always)]
pub fn exec_jmp(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let sj = Instruction::get_sj(instr);
    
    let frame = vm.current_frame_mut();
    // PC already incremented by dispatcher, so we add offset directly
    frame.pc = (frame.pc as i32 + sj) as usize;
    
    Ok(DispatchAction::Continue)
}

// ============ Test Instructions ============

/// TEST A k
/// if (not R[A] == k) then pc++
#[inline(always)]
pub fn exec_test(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access (hot path)
    let value = unsafe {
        *vm.register_stack.as_ptr().add(base_ptr + a)
    };
    
    // Lua truthiness: nil and false are falsy, everything else is truthy
    let is_truthy = !value.is_nil() && value.as_bool().unwrap_or(true);
    
    // If (not value) == k, skip next instruction
    if !is_truthy == k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// TESTSET A B k
/// if (not R[B] == k) then R[A] := R[B] else pc++
#[inline(always)]
pub fn exec_testset(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access
    let value = unsafe {
        *vm.register_stack.as_ptr().add(base_ptr + b)
    };
    
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
    
    Ok(DispatchAction::Continue)
}

// ============ Comparison Instructions ============

/// EQ A B k
/// if ((R[A] == R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_eq(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
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
    
    Ok(DispatchAction::Continue)
}

/// LT A B k
/// if ((R[A] < R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_lt(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
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
    
    let is_less = if (left_tag & TYPE_MASK) == TAG_INTEGER && (right_tag & TYPE_MASK) == TAG_INTEGER {
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
                        return Ok(DispatchAction::Continue);
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
                                vm.current_frame_mut().pc += 1;
                            }
                            return Ok(DispatchAction::Continue);
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
        return Ok(DispatchAction::Continue);
    };
    
    if is_less != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// LE A B k
/// if ((R[A] <= R[B]) ~= k) then pc++
#[inline(always)]
pub fn exec_le(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
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
    let is_less_or_equal = if (left.primary & TYPE_MASK) == TAG_INTEGER && (right.primary & TYPE_MASK) == TAG_INTEGER {
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
                        return Ok(DispatchAction::Continue);
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
                            return Ok(DispatchAction::Continue);
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
                            return Ok(DispatchAction::Continue);
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
                                let is_gt_result = !result.is_nil() && result.as_bool().unwrap_or(true);
                                let is_le_result = !is_gt_result;
                                if is_le_result != k {
                                    vm.current_frame_mut().pc += 1;
                                }
                                return Ok(DispatchAction::Continue);
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
        return Ok(DispatchAction::Continue);
    };
    
    if is_less_or_equal != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// EQK A B k
/// if ((R[A] == K[B]) ~= k) then pc++
pub fn exec_eqk(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let k = Instruction::get_k(instr);

    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;
    
    let func_ptr = frame.get_function_ptr().ok_or_else(|| {
        LuaError::RuntimeError("Not a Lua function".to_string())
    })?;
    let func = unsafe { &*func_ptr };
    let constant = func.borrow().chunk.constants.get(b).copied().ok_or_else(|| {
        LuaError::RuntimeError(format!("Invalid constant index: {}", b))
    })?;

    let left = vm.register_stack[base_ptr + a];

    let is_equal = left == constant;
    
    if is_equal != k {
        vm.current_frame_mut().pc += 1;
    }
    
    Ok(DispatchAction::Continue)
}

/// EQI A sB k
/// if ((R[A] == sB) ~= k) then pc++
#[inline(always)]
pub fn exec_eqi(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    let left = unsafe {
        *vm.register_stack.as_ptr().add(base_ptr + a)
    };

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
    
    Ok(DispatchAction::Continue)
}

/// LTI A sB k
/// if ((R[A] < sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lti(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    // OPTIMIZATION: Use unsafe for unchecked register access
    let left = unsafe {
        *vm.register_stack.as_ptr().add(base_ptr + a)
    };

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
    
    Ok(DispatchAction::Continue)
}

/// LEI A sB k
/// if ((R[A] <= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_lei(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    let left = unsafe {
        *vm.register_stack.as_ptr().add(base_ptr + a)
    };

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
    
    Ok(DispatchAction::Continue)
}

/// GTI A sB k
/// if ((R[A] > sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gti(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    let left = unsafe {
        *vm.register_stack.as_ptr().add(base_ptr + a)
    };

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
    
    Ok(DispatchAction::Continue)
}

/// GEI A sB k
/// if ((R[A] >= sB) ~= k) then pc++
#[inline(always)]
pub fn exec_gei(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sb = Instruction::get_sb(instr);
    let k = Instruction::get_k(instr);

    let base_ptr = vm.current_frame().base_ptr;

    let left = unsafe {
        *vm.register_stack.as_ptr().add(base_ptr + a)
    };

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
    
    Ok(DispatchAction::Continue)
}

// ============ Call Instructions ============

/// CALL A B C
/// R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
#[inline(always)]
pub fn exec_call(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
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
            let metatable_opt = func.as_lua_table()
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
            return Err(LuaError::RuntimeError(
                format!("attempt to call a {} value", func.type_name())
            ));
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
            
            // Call C function immediately
            let cfunc = func.as_cfunction().unwrap();
            
            // Create temporary frame for CFunction
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;
            
            // Set up call arguments in a new stack segment
            let call_base = vm.register_stack.len();
            let actual_arg_count = if use_call_metamethod {
                arg_count + 1 // Add 1 for self argument
            } else {
                arg_count
            };
            vm.ensure_stack_capacity(call_base + actual_arg_count + 1);
            
            // OPTIMIZATION: Bulk copy function and arguments (critical for table.insert perf!)
            vm.register_stack[call_base] = func;
            if use_call_metamethod {
                // First argument is the original table (self)
                vm.register_stack[call_base + 1] = call_metamethod_self;
                // Then bulk copy the original arguments
                if arg_count > 0 {
                    unsafe {
                        let src_ptr = vm.register_stack.as_ptr().add(base + a + 1);
                        let dst_ptr = vm.register_stack.as_mut_ptr().add(call_base + 2);
                        std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, arg_count);
                    }
                }
            } else {
                // Normal call: bulk copy arguments directly
                if arg_count > 0 {
                    unsafe {
                        let src_ptr = vm.register_stack.as_ptr().add(base + a + 1);
                        let dst_ptr = vm.register_stack.as_mut_ptr().add(call_base + 1);
                        std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, arg_count);
                    }
                }
            }
            
            let temp_frame = LuaCallFrame::new_c_function(
                frame_id,
                vm.current_frame().function_value,
                vm.current_frame().pc,
                call_base,
                actual_arg_count + 1,
            );
            
            vm.frames.push(temp_frame);
            let result = match cfunc(vm) {
                Ok(r) => Ok(r),
                Err(LuaError::Yield(values)) => {
                    // CFunction yielded - pop the temporary frame before yielding
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
            
            // OPTIMIZATION: Bulk copy return values
            let values = result.all_values();
            let num_returns = if return_count == usize::MAX {
                values.len()
            } else {
                return_count.min(values.len())
            };
            
            if num_returns > 0 {
                unsafe {
                    let src_ptr = values.as_ptr();
                    let dst_ptr = vm.register_stack.as_mut_ptr().add(base + a);
                    std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, num_returns);
                }
            }
            // Fill remaining with nil if needed (only when return_count is fixed)
            if return_count != usize::MAX {
                for i in num_returns..return_count {
                    vm.register_stack[base + a + i] = crate::LuaValue::nil();
                }
            }
            
            // CRITICAL: Update caller's top to indicate how many values were returned
            // This is essential for variable returns (C=0) so the next instruction knows
            // how many values are available (e.g., CALL with B=0)
            vm.current_frame_mut().top = a + num_returns;
            
            Ok(DispatchAction::Continue)
        },
        LuaValueKind::Function => {
            // OPTIMIZATION: Direct pointer access - NO hash lookup!
            let func_ptr = func.as_function_ptr()
                .ok_or_else(|| LuaError::RuntimeError("Invalid function pointer".to_string()))?;
            let (max_stack_size, is_vararg) = unsafe {
                let func_borrow = (*func_ptr).borrow();
                let size = if func_borrow.chunk.max_stack_size == 0 { 1 } else { func_borrow.chunk.max_stack_size };
                let vararg = func_borrow.chunk.is_vararg;
                (size, vararg)
            }; // Borrow released immediately

            // Create new frame
            let frame_id = vm.next_frame_id;
            vm.next_frame_id += 1;

            // OPTIMIZATION: Calculate all sizes upfront and do ONE capacity check
            let frame = vm.current_frame();
            let caller_base = frame.base_ptr;
            // OPTIMIZATION: Direct pointer access for caller function
            let caller_max_stack = if let Some(func_ptr) = frame.function_value.as_function_ptr() {
                unsafe { (*func_ptr).borrow().chunk.max_stack_size }
            } else {
                vm.register_stack.len().saturating_sub(caller_base)
            };
            
            let caller_stack_end = caller_base + caller_max_stack;
            let new_base = caller_stack_end;
            
            let actual_arg_count = if use_call_metamethod { arg_count + 1 } else { arg_count };
            let actual_stack_size = max_stack_size.max(actual_arg_count);
            let total_stack_size = if is_vararg && actual_arg_count > 0 {
                actual_stack_size + actual_arg_count
            } else {
                actual_stack_size
            };
            
            // OPTIMIZATION: Single capacity check for everything
            let required_capacity = (base + a + 1 + arg_count).max(caller_stack_end).max(new_base + total_stack_size);
            vm.ensure_stack_capacity(required_capacity);

            // OPTIMIZATION: Initialize with nil only once, then overwrite with arguments
            // Use unsafe for faster bulk operations
            unsafe {
                let reg_ptr = vm.register_stack.as_mut_ptr();
                let nil_val = crate::LuaValue::nil();
                
                // Initialize entire frame with nil
                for i in 0..total_stack_size {
                    *reg_ptr.add(new_base + i) = nil_val;
                }
                
                // OPTIMIZATION: Use ptr::copy for argument copying (faster than loop)
                // For all functions (including vararg), arguments are placed starting at new_base
                // VARARGPREP will later move the varargs to their proper location
                if use_call_metamethod {
                    *reg_ptr.add(new_base) = call_metamethod_self;
                    if arg_count > 0 {
                        std::ptr::copy_nonoverlapping(
                            reg_ptr.add(base + a + 1),
                            reg_ptr.add(new_base + 1),
                            arg_count
                        );
                    }
                } else if actual_arg_count > 0 {
                    std::ptr::copy_nonoverlapping(
                        reg_ptr.add(base + a + 1),
                        reg_ptr.add(new_base),
                        actual_arg_count
                    );
                }
            }

            // Create and push new frame
            // IMPORTANT: For vararg functions, top should reflect actual arg count, not max_stack_size
            // VARARGPREP will use this to determine the number of varargs
            let new_frame = LuaCallFrame::new_lua_function(
                frame_id,
                func,
                new_base,
                actual_arg_count, // top = number of arguments passed
                a, // result_reg: where to store return values
                return_count,
            );
            
            vm.frames.push(new_frame);

            Ok(DispatchAction::Call)
        },
        _ => {
            Err(LuaError::RuntimeError(format!(
                "attempt to call a {} value",
                func.type_name()
            )))
        }
    }
}

/// TAILCALL A B C k
/// return R[A](R[A+1], ... ,R[A+B-1])
pub fn exec_tailcall(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    
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
            
            let func_ptr = func.as_function_ptr()
                .ok_or_else(|| LuaError::RuntimeError("Invalid function pointer".to_string()))?;
            let max_stack_size = unsafe { (*func_ptr).borrow().chunk.max_stack_size };

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

            let new_frame = LuaCallFrame::new_lua_function(
                frame_id,
                func,
                old_base,
                arg_count, // top = number of arguments passed
                result_reg, // result_reg from the CALLER (not 0!)
                return_count,
            );
            vm.frames.push(new_frame);

            Ok(DispatchAction::Call)
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
            
            let c_func = func.as_cfunction()
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
            
            let temp_frame = LuaCallFrame::new_c_function(
                frame_id,
                function_value,
                pc,
                call_base,
                args_len + 1,
            );
            
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

            Ok(DispatchAction::Continue)
        }
        _ => {
            Err(LuaError::RuntimeError(format!(
                "attempt to call a {} value",
                func.type_name()
            )))
        }
    }
}

/// RETURN0
/// return (no values)
pub fn exec_return0(vm: &mut LuaVM, _instr: u32) -> LuaResult<DispatchAction> {
    let frame = vm.frames.pop().ok_or_else(|| {
        LuaError::RuntimeError("RETURN0 with no frame on stack".to_string())
    })?;

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
    
    Ok(DispatchAction::Return)
}

/// RETURN1 A
/// return R[A]
pub fn exec_return1(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;

    let frame = vm.frames.pop().ok_or_else(|| {
        LuaError::RuntimeError("RETURN1 with no frame on stack".to_string())
    })?;

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
        let should_update_top = if let Some(func_ptr) = caller_frame.function_value.as_function_ptr() {
            let max_stack = unsafe { (*func_ptr).borrow().chunk.max_stack_size };
            result_reg < max_stack
        } else {
            result_reg < 256
        };
        
        if should_update_top {
            vm.current_frame_mut().top = result_reg + 1;
        }
    }

    Ok(DispatchAction::Return)
}
