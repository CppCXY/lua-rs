/// Control flow instructions
/// 
/// These instructions handle function calls, returns, jumps, and coroutine operations.

use crate::LuaValue;
use crate::lua_vm::{LuaVM, LuaResult, LuaError, Instruction};
use super::DispatchAction;

/// RETURN A B C k
/// return R[A], ... ,R[A+B-2]
pub fn exec_return(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let b = Instruction::get_b(instr) as usize;
    let _c = Instruction::get_c(instr) as usize;
    let _k = Instruction::get_k(instr);

    let frame = vm.frames.pop().ok_or_else(|| {
        LuaError::RuntimeError("RETURN with no frame on stack".to_string())
    })?;

    let base_ptr = frame.base_ptr;

    // Collect return values
    vm.return_values.clear();

    if b == 0 {
        // Return all values from R[A] to top of stack
        let top = frame.top;
        for i in a..top {
            if base_ptr + i < vm.register_stack.len() {
                vm.return_values
                    .push(vm.register_stack[base_ptr + i]);
            }
        }
    } else {
        // Return b-1 values
        let count = b - 1;
        for i in 0..count {
            if base_ptr + a + i < vm.register_stack.len() {
                vm.return_values
                    .push(vm.register_stack[base_ptr + a + i]);
            }
        }
    }

    // If there are more frames, place return values in the caller's registers
    if !vm.frames.is_empty() {
        let caller_frame = vm.current_frame();
        let result_reg = caller_frame.get_result_reg();
        let num_results = caller_frame.get_num_results();
        let caller_base = caller_frame.base_ptr;

        // Copy return values to result registers
        if num_results == usize::MAX {
            // Multiple return values expected
            for (i, value) in vm.return_values.iter().enumerate() {
                vm.register_stack[caller_base + result_reg + i] = *value;
            }
        } else {
            // Fixed number of return values
            for i in 0..num_results {
                let value = vm.return_values.get(i).copied().unwrap_or(LuaValue::nil());
                vm.register_stack[caller_base + result_reg + i] = value;
            }
        }
    }

    // TODO: Handle upvalue closing (k bit)
    // if k { close_upvalues() }

    Ok(DispatchAction::Return)
}
