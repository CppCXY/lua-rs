// Debug library (stub implementation)
// Implements: debug, gethook, getinfo, getlocal, getmetatable, getregistry,
// getupvalue, getuservalue, sethook, setlocal, setmetatable, setupvalue,
// setuservalue, traceback, upvalueid, upvaluejoin

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

pub fn create_debug_lib() -> LibraryModule {
    crate::lib_module!("debug", {
        "traceback" => debug_traceback,
        "getinfo" => debug_getinfo,
    })
}

fn debug_traceback(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // debug.traceback([thread,] [message [, level]])
    // For now, we don't support the thread parameter
    
    // Get message argument (can be nil)
    let message = crate::lib_registry::get_arg(vm, 0).unwrap_or(LuaValue::nil());
    let message_str = if message.is_nil() {
        None
    } else {
        // Try to convert to string
        Some(vm.value_to_string_raw(&message))
    };
    
    // Get level argument (default is 1)
    let level = crate::lib_registry::get_arg(vm, 1)
        .and_then(|v| v.as_integer())
        .unwrap_or(1) as usize;
    
    // Generate traceback, skipping the first 'level' frames
    let mut trace = String::new();
    
    if let Some(msg) = message_str {
        trace.push_str(&msg);
        trace.push('\n');
    }
    
    trace.push_str("stack traceback:");
    
    // Iterate through call frames, starting from 'level'
    let total_frames = vm.frame_count;
    if level < total_frames {
        for i in (level..total_frames).rev() {
            let frame = &vm.frames[i];
            
            if frame.is_lua() {
                if let Some(func_id) = frame.get_function_id() {
                    if let Some(func) = vm.object_pool.get_function(func_id) {
                        let chunk = &func.chunk;
                        let source = chunk.source_name.as_deref().unwrap_or("?");
                        
                        // Get line number from pc
                        let pc = frame.pc.saturating_sub(1);
                        let line = if !chunk.line_info.is_empty() && pc < chunk.line_info.len() {
                            chunk.line_info[pc]
                        } else {
                            0
                        };
                        
                        if line > 0 {
                            trace.push_str(&format!("\n\t{}:{}: in function", source, line));
                        } else {
                            trace.push_str(&format!("\n\t{}: in function", source));
                        }
                    } else {
                        trace.push_str("\n\t?: in function");
                    }
                } else {
                    trace.push_str("\n\t?: in function");
                }
            } else {
                // C function
                trace.push_str("\n\t[C]: in function");
            }
        }
    }
    
    let result = vm.create_string(&trace);
    Ok(MultiValue::single(result))
}

fn debug_getinfo(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Stub: return nil
    Ok(MultiValue::single(LuaValue::nil()))
}
