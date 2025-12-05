// Debug library implementation
// Implements: debug, gethook, getinfo, getlocal, getmetatable, getregistry,
// getupvalue, getuservalue, sethook, setlocal, setupvalue,
// setuservalue, traceback, upvalueid, upvaluejoin
// Note: debug.setmetatable is not implemented (as requested)

use crate::lib_registry::{LibraryModule, get_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

pub fn create_debug_lib() -> LibraryModule {
    crate::lib_module!("debug", {
        // "debug" => debug_debug,
        // "gethook" => debug_gethook,
        // "getinfo" => debug_getinfo,
        // "getlocal" => debug_getlocal,
        // "getmetatable" => debug_getmetatable,
        // "getregistry" => debug_getregistry,
        // "getupvalue" => debug_getupvalue,
        // "getuservalue" => debug_getuservalue,
        // "sethook" => debug_sethook,
        // "setlocal" => debug_setlocal,
        // "setupvalue" => debug_setupvalue,
        // "setuservalue" => debug_setuservalue,
        "traceback" => debug_traceback,
        // "upvalueid" => debug_upvalueid,
        // "upvaluejoin" => debug_upvaluejoin,
    })
}

fn debug_traceback(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // debug.traceback([thread,] [message [, level]])
    // For now, we don't support the thread parameter

    // Get message argument (can be nil)
    let message = get_arg(vm, 1).unwrap_or(LuaValue::nil());
    let message_str = if message.is_nil() {
        None
    } else {
        // Try to convert to string
        Some(vm.value_to_string_raw(&message))
    };

    // Get level argument (default is 1)
    let level = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(1) as usize;

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
                        if let Some(chunk) = func.chunk() {
                            let source = chunk.source_name.as_deref().unwrap_or("?");

                            // Get line number from pc
                            let pc = frame.pc.saturating_sub(1) as usize;
                            let line = if !chunk.line_info.is_empty() && pc < chunk.line_info.len()
                            {
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
                            // C closure
                            trace.push_str("\n\t[C closure]: in function");
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

// fn debug_getinfo(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
//     // Stub: return nil
//     Ok(MultiValue::single(LuaValue::nil()))
// }
