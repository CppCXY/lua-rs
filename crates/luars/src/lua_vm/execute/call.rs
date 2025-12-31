/// Function call implementation
/// 
/// Implements CALL and TAILCALL opcodes
/// 
/// IMPORTANT: These do NOT recursively call execute_frame!
/// Following Lua's design:
/// - CALL: push new frame, return FrameAction::Call (main loop loads new chunk)
/// - TAILCALL: replace current frame, return FrameAction::TailCall (main loop loads new chunk)

use crate::lua_vm::{LuaState, LuaError, LuaResult};

pub enum FrameAction {
    Return,    // Frame finished, return to caller
    Call,      // Pushed new frame, execute callee
    TailCall,  // Replaced current frame, execute tail callee
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
    
    let _nresults = if c == 0 {
        -1  // Multiple return
    } else {
        (c - 1) as i32
    };
    
    // Get function to call
    let func_idx = base + a;
    let func = lua_state.stack_get(func_idx)
        .ok_or_else(|| lua_state.error("CALL: function not found".to_string()))?;
    
    // Check if it's a Lua function
    if let Some(new_func_id) = func.as_function_id() {
        // Verify it's a Lua function
        let new_func = lua_state.vm_mut().object_pool.get_function(new_func_id)
            .ok_or(LuaError::RuntimeError)?;
        
        if new_func.is_lua_function() {
            // Lua function call: push new frame
            let new_base = func_idx + 1;  // Arguments start after function
            
            // Push new call frame
            lua_state.push_frame(func, new_base, nargs)?;
            
            // Return FrameAction::Call - main loop will load new chunk and continue
            Ok(FrameAction::Call)
        } else {
            // C function call
            Err(lua_state.error("CALL: C function not yet supported".to_string()))
        }
    } else {
        // Not a function - should check for __call metamethod
        Err(lua_state.error("CALL: attempt to call a non-function".to_string()))
    }
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
    let nargs = if b == 0 {
        0
    } else {
        b - 1
    };
    
    // Get function to call
    let func_idx = base + a;
    let func = lua_state.stack_get(func_idx)
        .ok_or_else(|| lua_state.error("TAILCALL: function not found".to_string()))?;
    
    // Check if it's a Lua function
    if let Some(new_func_id) = func.as_function_id() {
        let new_func = lua_state.vm_mut().object_pool.get_function(new_func_id)
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
        } else {
            Err(lua_state.error("TAILCALL: C function not yet supported".to_string()))
        }
    } else {
        Err(lua_state.error("TAILCALL: attempt to call a non-function".to_string()))
    }
}
