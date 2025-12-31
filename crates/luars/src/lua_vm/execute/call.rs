/// Function call implementation
///
/// Implements CALL and TAILCALL opcodes
///
/// IMPORTANT: These do NOT recursively call execute_frame!
/// Following Lua's design:
/// - CALL: push new frame, return FrameAction::Call (main loop loads new chunk)
/// - TAILCALL: replace current frame, return FrameAction::TailCall (main loop loads new chunk)
use crate::{
    LuaValue,
    lua_vm::{CFunction, LuaError, LuaResult, LuaState},
};

pub enum FrameAction {
    Return,   // Frame finished, return to caller
    Call,     // Pushed new frame, execute callee
    TailCall, // Replaced current frame, execute tail callee
    Continue, // C function executed, continue current frame
}

/// Handle CALL opcode - Lua style (push frame, don't recurse)
/// R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
#[inline]
pub fn handle_call(
    lua_state: &mut LuaState,
    _frame_idx: usize,
    base: usize,
    a: usize,
    b: usize,
    c: usize,
) -> LuaResult<FrameAction> {
    let nargs = if b == 0 {
        // Variable args: use stack top
        // TODO: 需要从stack top计算参数数量
        0
    } else {
        b - 1
    };

    let nresults = if c == 0 {
        -1 // Multiple return
    } else {
        (c - 1) as i32
    };

    // Get function to call
    let func_idx = base + a;
    let func = lua_state
        .stack_get(func_idx)
        .ok_or_else(|| lua_state.error("CALL: function not found".to_string()))?;

    // Check if it's a light C function (most common case for stdlib functions)
    if func.is_cfunction() {
        // Light C function call: execute directly
        call_c_function(lua_state, func_idx, nargs, nresults)?;

        // C function executed synchronously, continue with current frame
        return Ok(FrameAction::Continue);
    }

    // Check if it's a GC function (Lua or C)
    if let Some(new_func_id) = func.as_function_id() {
        let new_func = lua_state
            .vm_mut()
            .object_pool
            .get_function(new_func_id)
            .ok_or(LuaError::RuntimeError)?;

        if new_func.is_lua_function() {
            // Lua function call: push new frame
            let new_base = func_idx + 1; // Arguments start after function

            // Push new call frame
            lua_state.push_frame(func, new_base, nargs)?;

            // Return FrameAction::Call - main loop will load new chunk and continue
            Ok(FrameAction::Call)
        } else if new_func.is_c_function() {
            // GC C function call: execute directly
            call_c_function(lua_state, func_idx, nargs, nresults)?;

            // C function executed synchronously, continue with current frame
            Ok(FrameAction::Continue)
        } else {
            Err(lua_state.error("CALL: unknown function type".to_string()))
        }
    } else {
        // Not a function - should check for __call metamethod
        Err(lua_state.error("CALL: attempt to call a non-function".to_string()))
    }
}

/// Call a C function and handle results  
/// Similar to Lua's precallC - much simpler than our initial attempt
/// Lua 的做法：C 函数直接在当前栈上执行，返回结果数量
fn call_c_function(
    lua_state: &mut LuaState,
    func_idx: usize,
    nargs: usize,
    nresults: i32,
) -> LuaResult<()> {
    // Get the function
    let func = lua_state
        .stack_get(func_idx)
        .ok_or_else(|| lua_state.error("C function not found".to_string()))?;

    // Get the C function pointer - handle both light C functions and GC C functions
    let c_func: CFunction = if func.is_cfunction() {
        // Light C function - extract directly from value
        unsafe {
            let func_ptr = func.value_.f as usize;
            std::mem::transmute(func_ptr)
        }
    } else if let Some(func_id) = func.as_function_id() {
        // GC function - need to get from object pool
        let gc_func = lua_state
            .vm_mut()
            .object_pool
            .get_function(func_id)
            .ok_or(LuaError::RuntimeError)?;

        gc_func
            .c_function()
            .ok_or_else(|| lua_state.error("Not a C function".to_string()))?
    } else {
        return Err(lua_state.error("Not a callable value".to_string()));
    };

    // Lua 的做法很简单：
    // 1. Push 一个临时 CallInfo (用于 C 函数访问参数)
    // 2. 调用 C 函数
    // 3. 调用 luaD_poscall 处理返回值
    // 我们简化版本：
    let call_base = func_idx + 1;

    // Push temporary frame for C function
    lua_state.push_frame(func, call_base, nargs)?;

    // Call the C function (it returns number of results)
    let n = c_func(lua_state)?;

    // Pop the frame
    lua_state.pop_frame();

    // Move results from call_base to func_idx (Lua's moveresults)
    // TODO: 这里需要实现完整的 moveresults 逻辑，处理 nresults 的各种情况
    // 现在简化版本：直接移动结果
    let actual_nresults = if nresults >= 0 {
        nresults as usize
    } else {
        n // MULTRET: use actual return count
    };

    for i in 0..actual_nresults {
        let val = if i < n {
            lua_state
                .stack_get(call_base + i)
                .unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil() // Pad with nil
        };
        lua_state.stack_set(func_idx + i, val);
    }

    Ok(())
}

/// Handle TAILCALL opcode - Lua style (replace frame, don't recurse)
/// Tail call optimization: return R[A](R[A+1], ... ,R[A+B-1])
#[inline]
pub fn handle_tailcall(
    lua_state: &mut LuaState,
    _frame_idx: usize,
    base: usize,
    a: usize,
    b: usize,
) -> LuaResult<FrameAction> {
    let nargs = if b == 0 { 0 } else { b - 1 };

    // Get function to call
    let func_idx = base + a;
    let func = lua_state
        .stack_get(func_idx)
        .ok_or_else(|| lua_state.error("TAILCALL: function not found".to_string()))?;

    // Check if it's a function
    if let Some(new_func_id) = func.as_function_id() {
        let new_func = lua_state
            .vm_mut()
            .object_pool
            .get_function(new_func_id)
            .ok_or(LuaError::RuntimeError)?;

        if new_func.is_lua_function() {
            // Move arguments to current frame base
            let mut args = Vec::with_capacity(nargs);
            for i in 0..nargs {
                let src_idx = func_idx + 1 + i;
                if let Some(arg) = lua_state.stack_get(src_idx) {
                    args.push(arg);
                }
            }

            for (i, arg) in args.into_iter().enumerate() {
                let dst_idx = base + i;
                lua_state.stack_set(dst_idx, arg);
            }

            // Replace function at base
            lua_state.stack_set(base - 1, func);

            // Update frame function and reset PC (reusing current frame)
            // Don't pop/push - just modify the existing frame
            // TODO: Need set_frame_func that updates the current frame's function
            // For now, this is simplified

            // Return FrameAction::TailCall - main loop will load new chunk and continue
            Ok(FrameAction::TailCall)
        } else if new_func.is_c_function() {
            // C function tail call: execute directly and continue
            // Lua \u4e5f\u662f\u8fd9\u6837\u505a\u7684\uff1aluaD_pretailcall \u8c03\u7528 precallC
            call_c_function(lua_state, func_idx, nargs, -1)?;

            // Move results to current frame base (for return)
            let mut i = 0;
            loop {
                match lua_state.stack_get(func_idx + i) {
                    Some(result) if i == 0 || !result.is_nil() => {
                        lua_state.stack_set(base + i, result);
                        i += 1;
                    }
                    _ => break,
                }
            }

            // C function done, just continue (it's like a return for tail call)
            Ok(FrameAction::Continue)
        } else if func.is_cfunction() {
            // Light C function tail call
            call_c_function(lua_state, func_idx, nargs, -1)?;

            // Move results to current frame base
            let mut i = 0;
            loop {
                match lua_state.stack_get(func_idx + i) {
                    Some(result) if i == 0 || !result.is_nil() => {
                        lua_state.stack_set(base + i, result);
                        i += 1;
                    }
                    _ => break,
                }
            }

            Ok(FrameAction::Continue)
        } else {
            Err(lua_state.error("TAILCALL: unknown function type".to_string()))
        }
    } else {
        Err(lua_state.error("TAILCALL: attempt to call a non-function".to_string()))
    }
}
