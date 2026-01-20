// Debug library implementation
// Implements: traceback, getinfo, getlocal, getmetatable, getupvalue, etc.

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState, get_metatable};

pub fn create_debug_lib() -> LibraryModule {
    crate::lib_module!("debug", {
        "traceback" => debug_traceback,
        "getinfo" => debug_getinfo,
        "getmetatable" => debug_getmetatable,
        "setmetatable" => debug_setmetatable,
    })
}

/// debug.traceback([message [, level]]) - Get stack traceback
fn debug_traceback(l: &mut LuaState) -> LuaResult<usize> {
    // Get message argument (can be nil)
    let message_val = l.get_arg(1).unwrap_or(LuaValue::nil());
    let message_str = if message_val.is_nil() {
        None
    } else if let Some(s) = message_val.as_str() {
        Some(s.to_string())
    } else {
        return Err(l.error("bad argument #1 to 'traceback' (string or nil expected)".to_string()));
    };

    // Get level argument (default is 1)
    let level = l
        .get_arg(2)
        .and_then(|v| v.as_integer())
        .unwrap_or(1)
        .max(0) as usize;

    // Generate traceback
    let mut trace = String::new();

    if let Some(msg) = message_str {
        trace.push_str(&msg);
        trace.push('\n');
    }

    trace.push_str("stack traceback:");

    // Get call stack info
    let call_depth = l.call_depth();

    // Iterate through call frames, starting from 'level'
    if level < call_depth {
        for i in (level..call_depth).rev() {
            if let Some(func) = l.get_frame_func(i) {
                let pc = l.get_frame_pc(i);

                // Try to get function info
                if let Some(func_obj) = func.as_lua_function() {
                    if let Some(chunk) = func_obj.chunk() {
                        // Lua function
                        let source = chunk.source_name.as_deref().unwrap_or("?");

                        // Get line number from pc
                        let pc_idx = pc.saturating_sub(1) as usize;
                        let line = if !chunk.line_info.is_empty() && pc_idx < chunk.line_info.len()
                        {
                            chunk.line_info[pc_idx]
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
                        trace.push_str("\n\t[C]: in function");
                    }
                } else if func.is_cfunction() {
                    // C function
                    trace.push_str("\n\t[C]: in function");
                } else {
                    trace.push_str("\n\t?: in function");
                }
            }
        }
    }

    let result = l.create_string(&trace);
    l.push_value(result)?;
    Ok(1)
}

/// debug.getinfo([thread,] f [, what]) - Get function info
fn debug_getinfo(l: &mut LuaState) -> LuaResult<usize> {
    // Simplified implementation: just return a table with basic info
    let vm = l.vm_mut();
    let info_table = vm.create_table(0, 4);

    // Set some basic fields
    let source_key = vm.create_string("source");
    let source_val = vm.create_string("=[C]");
    vm.table_set(&info_table, source_key, source_val);

    let what_key = vm.create_string("what");
    let what_val = vm.create_string("C");
    vm.table_set(&info_table, what_key, what_val);
    
    l.push_value(info_table)?;
    Ok(1)
}

/// debug.getmetatable(value) - Get metatable of a value (no protection)
fn debug_getmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("getmetatable() requires argument 1".to_string()))?;

    // For tables, get metatable directly
    let v = get_metatable(l, &value).unwrap_or(LuaValue::nil());
    // For other types, return nil (simplified)
    l.push_value(v)?;
    Ok(1)
}

/// debug.setmetatable(value, table) - Set metatable of a value
fn debug_setmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("setmetatable() requires argument 1".to_string()))?;

    let metatable = l.get_arg(2);

    // Only support tables for now
    if let Some(table) = value.as_table_mut() {
        if let Some(mt) = metatable {
            if mt.is_nil() {
                table.set_metatable(None);
            } else if mt.is_table() {
                table.set_metatable(Some(mt));
            } else {
                return Err(l.error("setmetatable() requires a table or nil".to_string()));
            }
        } else {
            table.set_metatable(None);
        }
    }

    l.push_value(value)?;
    Ok(1)
}
