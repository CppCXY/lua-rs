// Debug library implementation
// Implements: traceback, getinfo, getlocal, getmetatable, getupvalue, etc.

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::call_info::call_status;
use crate::lua_vm::{LuaResult, LuaState, get_metatable};

pub fn create_debug_lib() -> LibraryModule {
    crate::lib_module!("debug", {
        "traceback" => debug_traceback,
        "getinfo" => debug_getinfo,
        "getmetatable" => debug_getmetatable,
        "setmetatable" => debug_setmetatable,
        "getregistry" => debug_getregistry,
        "getlocal" => debug_getlocal,
        "setlocal" => debug_setlocal,
        "getupvalue" => debug_getupvalue,
        "setupvalue" => debug_setupvalue,
        "upvalueid" => debug_upvalueid,
        "upvaluejoin" => debug_upvaluejoin,
        "gethook" => debug_gethook,
        "sethook" => debug_sethook,
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

    // Adjust level to skip the traceback function itself if called from Lua
    let start_level = level;

    // Iterate through call frames, starting from 'level'
    if start_level < call_depth {
        let mut shown = 0;
        for i in (start_level..call_depth).rev() {
            // Limit traceback to avoid overly long output
            if shown >= 20 {
                trace.push_str("\n\t...");
                break;
            }

            if let Some(func) = l.get_frame_func(i) {
                let pc = l.get_frame_pc(i);

                // Try to get function info
                if let Some(func_obj) = func.as_lua_function() {
                    if let Some(chunk) = func_obj.chunk() {
                        // Lua function
                        let source = chunk.source_name.as_deref().unwrap_or("?");

                        // Format source name (strip @ prefix if present)
                        let source_display = if source.starts_with('@') {
                            &source[1..]
                        } else {
                            source
                        };

                        // Get line number from pc
                        let pc_idx = pc.saturating_sub(1) as usize;
                        let line = if !chunk.line_info.is_empty() && pc_idx < chunk.line_info.len()
                        {
                            chunk.line_info[pc_idx]
                        } else {
                            0
                        };

                        // Determine function name and type
                        // For now, use simplified logic - full implementation would need
                        // to search locals/upvalues of calling frame
                        let (name_what, func_name) = if chunk.linedefined == 0 {
                            // Main chunk (linedefined == 0 means top-level code)
                            ("main chunk", String::new())
                        } else if i == call_depth - 1 {
                            // Also main chunk if at bottom of stack
                            ("main chunk", String::new())
                        } else {
                            // TODO: Search for function name in calling frame's locals/upvalues
                            // This requires inspecting the previous frame's locals and upvalues
                            ("function", String::new())
                        };

                        if line > 0 {
                            if func_name.is_empty() {
                                trace.push_str(&format!(
                                    "\n\t{}:{}: in {}",
                                    source_display, line, name_what
                                ));
                            } else {
                                trace.push_str(&format!(
                                    "\n\t{}:{}: in {} '{}'",
                                    source_display, line, name_what, func_name
                                ));
                            }
                        } else {
                            if func_name.is_empty() {
                                trace
                                    .push_str(&format!("\n\t{}: in {}", source_display, name_what));
                            } else {
                                trace.push_str(&format!(
                                    "\n\t{}: in {} '{}'",
                                    source_display, name_what, func_name
                                ));
                            }
                        }
                    } else {
                        // C closure
                        trace.push_str("\n\t[C]: in function");
                    }
                } else if func.is_cfunction() {
                    // C function - try to get name
                    // In full implementation, would track C function names
                    trace.push_str("\n\t[C]: in function");
                } else {
                    trace.push_str("\n\t?: in function");
                }
                shown += 1;
            }
        }
    }

    let result = l.create_string(&trace)?;
    l.push_value(result)?;
    Ok(1)
}

/// debug.getinfo([thread,] f [, what]) - Get function info
fn debug_getinfo(l: &mut LuaState) -> LuaResult<usize> {
    // Parse arguments
    let arg1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("getinfo requires at least 1 argument".to_string()))?;
    let arg2 = l.get_arg(2);

    // Determine if arg1 is a function or a stack level
    let (func, what_str) = if arg1.is_function() {
        // arg1 is a function, arg2 is what
        let what = if let Some(w) = arg2 {
            if let Some(s) = w.as_str() {
                s.to_string()
            } else {
                "flnSrtu".to_string() // Default what
            }
        } else {
            "flnSrtu".to_string()
        };
        (arg1, what)
    } else if let Some(level) = arg1.as_integer() {
        // arg1 is a stack level, arg2 is what
        let what = if let Some(w) = arg2 {
            if let Some(s) = w.as_str() {
                s.to_string()
            } else {
                "flnSrtu".to_string()
            }
        } else {
            "flnSrtu".to_string()
        };

        // Get function at stack level
        let call_depth = l.call_depth();
        if level < 0 || level as usize >= call_depth {
            // Level out of range
            return Ok(0);
        }

        let func = l
            .get_frame_func(level as usize)
            .ok_or_else(|| l.error("invalid stack level".to_string()))?;
        (func, what)
    } else {
        return Err(
            l.error("bad argument #1 to 'getinfo' (function or number expected)".to_string())
        );
    };

    // Create result table
    let info_table = l.create_table(0, 8)?;

    // Process function info based on 'what' parameter
    if let Some(lua_func) = func.as_lua_function() {
        if let Some(chunk) = lua_func.chunk() {
            // Lua function
            if what_str.contains('S') {
                // Source info
                let source = chunk.source_name.as_deref().unwrap_or("?");
                let source_key = l.create_string("source")?;
                let source_val = l.create_string(source)?;
                l.raw_set(&info_table, source_key, source_val);

                let short_src_key = l.create_string("short_src")?;
                let short_src_val = l.create_string(source)?;
                l.raw_set(&info_table, short_src_key, short_src_val);

                let linedefined_key = l.create_string("linedefined")?;
                let linedefined_val = LuaValue::integer(chunk.linedefined as i64);
                l.raw_set(&info_table, linedefined_key, linedefined_val);

                let lastlinedefined_key = l.create_string("lastlinedefined")?;
                let lastlinedefined_val = LuaValue::integer(chunk.lastlinedefined as i64);
                l.raw_set(&info_table, lastlinedefined_key, lastlinedefined_val);

                let what_key = l.create_string("what")?;
                let what_val = l.create_string("Lua")?;
                l.raw_set(&info_table, what_key, what_val);
            }

            if what_str.contains('l') {
                // Current line (only meaningful for stack level, not direct function)
                // For now, return -1 (unknown)
                let currentline_key = l.create_string("currentline")?;
                let currentline_val = LuaValue::integer(-1);
                l.raw_set(&info_table, currentline_key, currentline_val);
            }

            if what_str.contains('u') {
                // Upvalue info
                let nups_key = l.create_string("nups")?;
                let nups_val = LuaValue::integer(chunk.upvalue_count as i64);
                l.raw_set(&info_table, nups_key, nups_val);

                let nparams_key = l.create_string("nparams")?;
                let nparams_val = LuaValue::integer(chunk.param_count as i64);
                l.raw_set(&info_table, nparams_key, nparams_val);

                let isvararg_key = l.create_string("isvararg")?;
                let isvararg_val = LuaValue::boolean(chunk.is_vararg);
                l.raw_set(&info_table, isvararg_key, isvararg_val);
            }

            if what_str.contains('n') {
                // Name info (not implemented, use defaults)
                let name_key = l.create_string("name")?;
                let name_val = LuaValue::nil();
                l.raw_set(&info_table, name_key, name_val);

                let namewhat_key = l.create_string("namewhat")?;
                let namewhat_val = l.create_string("")?;
                l.raw_set(&info_table, namewhat_key, namewhat_val);
            }

            if what_str.contains('t') {
                // Tail call info
                let istailcall_key = l.create_string("istailcall")?;
                let istailcall_val = LuaValue::boolean(false);
                l.raw_set(&info_table, istailcall_key, istailcall_val);

                // extraargs: number of __call metamethods in the call chain
                // Extract from call_status bits 8-11 (CIST_CCMT)
                let extraargs_opt = if let Some(level) = arg1.as_integer() {
                    use crate::lua_vm::call_info::call_status;
                    l.get_frame(level as usize)
                        .map(|f| call_status::get_ccmt_count(f.call_status))
                } else {
                    None
                };

                if let Some(extraargs) = extraargs_opt {
                    let extraargs_key = l.create_string("extraargs")?;
                    let extraargs_val = LuaValue::integer(extraargs as i64);
                    l.raw_set(&info_table, extraargs_key, extraargs_val);
                }
            }

            if what_str.contains('f') {
                // Function itself
                let func_key = l.create_string("func")?;
                l.raw_set(&info_table, func_key, func);
            }
        }
    } else if func.is_cfunction() {
        // C function
        if what_str.contains('S') {
            let source_key = l.create_string("source")?;
            let source_val = l.create_string("=[C]")?;
            l.raw_set(&info_table, source_key, source_val);

            let short_src_key = l.create_string("short_src")?;
            let short_src_val = l.create_string("[C]")?;
            l.raw_set(&info_table, short_src_key, short_src_val);

            let linedefined_key = l.create_string("linedefined")?;
            let linedefined_val = LuaValue::integer(-1);
            l.raw_set(&info_table, linedefined_key, linedefined_val);

            let lastlinedefined_key = l.create_string("lastlinedefined")?;
            let lastlinedefined_val = LuaValue::integer(-1);
            l.raw_set(&info_table, lastlinedefined_key, lastlinedefined_val);

            let what_key = l.create_string("what")?;
            let what_val = l.create_string("C")?;
            l.raw_set(&info_table, what_key, what_val);
        }

        if what_str.contains('n') {
            let name_key = l.create_string("name")?;
            let name_val = LuaValue::nil();
            l.raw_set(&info_table, name_key, name_val);

            let namewhat_key = l.create_string("namewhat")?;
            let namewhat_val = l.create_string("")?;
            l.raw_set(&info_table, namewhat_key, namewhat_val);
        }

        if what_str.contains('f') {
            let func_key = l.create_string("func")?;
            l.raw_set(&info_table, func_key, func);
        }

        if what_str.contains('t') {
            // Tail call info for C functions
            let istailcall_key = l.create_string("istailcall")?;
            let istailcall_val = LuaValue::boolean(false);
            l.raw_set(&info_table, istailcall_key, istailcall_val);

            // extraargs for C functions
            let extraargs_opt = if let Some(level) = arg1.as_integer() {
                l.get_frame(level as usize)
                    .map(|f| call_status::get_ccmt_count(f.call_status))
            } else {
                None
            };

            if let Some(extraargs) = extraargs_opt {
                let extraargs_key = l.create_string("extraargs")?;
                let extraargs_val = LuaValue::integer(extraargs as i64);
                l.raw_set(&info_table, extraargs_key, extraargs_val);
            }
        }
    }

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

    // Register for finalization if __gc is present
    l.vm_mut().gc.check_finalizer(&value);

    l.push_value(value)?;
    Ok(1)
}

/// debug.gethook([thread]) - Get current hook settings
/// Stub implementation: always returns nil (no hooks set)
fn debug_gethook(_l: &mut LuaState) -> LuaResult<usize> {
    // TODO: Implement proper hook support
    // For now, return nil to indicate no hook is set
    Ok(0) // Return nothing (nil)
}

/// debug.sethook([thread,] hook, mask [, count]) - Set a debug hook
/// Stub implementation: accepts arguments but does nothing
fn debug_sethook(_l: &mut LuaState) -> LuaResult<usize> {
    // TODO: Implement proper hook support
    // For now, just accept the arguments and do nothing
    Ok(0) // Return nothing
}

/// debug.getregistry() - Return the registry table
fn debug_getregistry(l: &mut LuaState) -> LuaResult<usize> {
    let registry = l.vm_mut().registry.clone();
    l.push_value(registry)?;
    Ok(1)
}

/// debug.getlocal([thread,] f, local) - Get the name and value of a local variable
fn debug_getlocal(l: &mut LuaState) -> LuaResult<usize> {
    // Parse arguments: [thread,] level/func, local_index
    let arg1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("getlocal requires at least 2 arguments".to_string()))?;
    let arg2 = l
        .get_arg(2)
        .ok_or_else(|| l.error("getlocal requires at least 2 arguments".to_string()))?;

    // For now, we only support level (not thread or function)
    // arg1: level (stack level)
    // arg2: local_index
    let level = arg1
        .as_integer()
        .ok_or_else(|| l.error("bad argument #1 to 'getlocal' (number expected)".to_string()))?
        as usize;
    let local_index = arg2
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'getlocal' (number expected)".to_string()))?
        as usize;

    // Get the call frame at the specified level
    let call_depth = l.call_depth();
    if level >= call_depth {
        // Level out of range, return nil
        return Ok(0);
    }

    // Get function at this level
    let frame_func = l
        .get_frame_func(level)
        .ok_or_else(|| l.error("invalid stack level".to_string()))?;

    if let Some(lua_func) = frame_func.as_lua_function() {
        if let Some(chunk) = lua_func.chunk() {
            // Get local variable name from chunk
            if local_index > 0 && local_index <= chunk.locals.len() {
                let name = &chunk.locals[local_index - 1];

                // Get the value from the stack
                // The local variables are at base + (local_index - 1)
                let base = l.get_frame_base(level);
                let value_idx = base + local_index - 1;

                let top = l.get_top();
                if value_idx < top {
                    let value = l.stack_get(value_idx).unwrap_or(LuaValue::nil());
                    let name_str = l.create_string(name)?;
                    l.push_value(name_str)?;
                    l.push_value(value)?;
                    return Ok(2);
                }
            }
        }
    }

    // No local variable found, return nil
    Ok(0)
}

/// debug.setlocal([thread,] level, local, value) - Set the value of a local variable
fn debug_setlocal(l: &mut LuaState) -> LuaResult<usize> {
    // Parse arguments: [thread,] level, local_index, value
    let level_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("setlocal requires at least 3 arguments".to_string()))?;
    let local_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("setlocal requires at least 3 arguments".to_string()))?;
    let value = l
        .get_arg(3)
        .ok_or_else(|| l.error("setlocal requires at least 3 arguments".to_string()))?;

    let level = level_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #1 to 'setlocal' (number expected)".to_string()))?
        as usize;
    let local_index = local_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'setlocal' (number expected)".to_string()))?
        as usize;

    // Get the call frame at the specified level
    let call_depth = l.call_depth();
    if level >= call_depth {
        // Level out of range, return nil
        return Ok(0);
    }

    // Get function at this level
    let frame_func = l
        .get_frame_func(level)
        .ok_or_else(|| l.error("invalid stack level".to_string()))?;

    if let Some(lua_func) = frame_func.as_lua_function() {
        if let Some(chunk) = lua_func.chunk() {
            // Get local variable name from chunk
            if local_index > 0 && local_index <= chunk.locals.len() {
                let name = &chunk.locals[local_index - 1];

                // Set the value on the stack
                let base = l.get_frame_base(level);
                let value_idx = base + local_index - 1;

                let top = l.get_top();
                if value_idx < top {
                    l.stack_set(value_idx, value)?;
                    let name_str = l.create_string(name)?;
                    l.push_value(name_str)?;
                    return Ok(1);
                }
            }
        }
    }

    // No local variable found, return nil
    Ok(0)
}

/// debug.getupvalue(f, up) - Get the name and value of an upvalue
fn debug_getupvalue(l: &mut LuaState) -> LuaResult<usize> {
    let func = l
        .get_arg(1)
        .ok_or_else(|| l.error("getupvalue requires 2 arguments".to_string()))?;
    let up_index_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("getupvalue requires 2 arguments".to_string()))?;

    // Check that first argument is a function
    if !func.is_function() {
        return Err(l.error("bad argument #1 to 'getupvalue' (function expected)".to_string()));
    }

    let up_index = up_index_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'getupvalue' (number expected)".to_string()))?
        as usize;

    if let Some(lua_func) = func.as_lua_function() {
        // Get upvalue from Lua function
        let upvalues = lua_func.upvalues();
        if up_index > 0 && up_index <= upvalues.len() {
            let upvalue = &upvalues[up_index - 1];

            // Get the name from chunk
            if let Some(chunk) = lua_func.chunk() {
                if up_index <= chunk.upvalue_descs.len() {
                    // Use actual upvalue name from chunk
                    let name = &chunk.upvalue_descs[up_index - 1].name;
                    let name_str = l.create_string(name)?;

                    // Get the value
                    let value = upvalue.as_ref().data.get_value();
                    l.push_value(name_str)?;
                    l.push_value(value)?;
                    return Ok(2);
                }
            }
        }
    }

    // No upvalue found, return nil
    Ok(0)
}

/// debug.setupvalue(f, up, value) - Set the value of an upvalue
fn debug_setupvalue(l: &mut LuaState) -> LuaResult<usize> {
    let func = l
        .get_arg(1)
        .ok_or_else(|| l.error("setupvalue requires 3 arguments".to_string()))?;
    let up_index_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("setupvalue requires 3 arguments".to_string()))?;
    let value = l
        .get_arg(3)
        .ok_or_else(|| l.error("setupvalue requires 3 arguments".to_string()))?;

    // Check that first argument is a function
    if !func.is_function() {
        return Err(l.error("bad argument #1 to 'setupvalue' (function expected)".to_string()));
    }

    let up_index = up_index_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'setupvalue' (number expected)".to_string()))?
        as usize;

    if let Some(lua_func) = func.as_lua_function() {
        // Set upvalue in Lua function
        let upvalues = lua_func.upvalues();
        if up_index > 0 && up_index <= upvalues.len() {
            let upvalue_ptr = upvalues[up_index - 1];

            // Get the upvalue name from the chunk
            let upvalue_name = if let Some(chunk) = lua_func.chunk() {
                if up_index - 1 < chunk.upvalue_descs.len() {
                    chunk.upvalue_descs[up_index - 1].name.clone()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            // Set the upvalue value (similar to SETUPVAL instruction)
            let upval_ref = upvalue_ptr.as_mut_ref();
            match &mut upval_ref.data {
                crate::lua_value::LuaUpvalue::Open { stack_ptr, .. } => {
                    // Open upvalue: write to stack location
                    unsafe {
                        *stack_ptr.ptr = value;
                    }
                }
                crate::lua_value::LuaUpvalue::Closed(_) => {
                    // Closed upvalue: update heap storage
                    upval_ref.data.close(value);
                }
            }

            // GC barrier if needed
            if value.is_collectable() {
                if let Some(value_gc_ptr) = value.as_gc_ptr() {
                    l.gc_barrier(upvalue_ptr, value_gc_ptr);
                }
            }

            // Return the upvalue name
            if !upvalue_name.is_empty() {
                let name_val = l.create_string(&upvalue_name)?;
                l.push_value(name_val)?;
                return Ok(1);
            } else {
                l.push_value(LuaValue::nil())?;
                return Ok(1);
            }
        }
    }

    // No upvalue found, return nil
    Ok(0)
}

/// debug.upvalueid(f, n) - Get a unique identifier for an upvalue
fn debug_upvalueid(l: &mut LuaState) -> LuaResult<usize> {
    let func = l
        .get_arg(1)
        .ok_or_else(|| l.error("upvalueid requires 2 arguments".to_string()))?;
    let up_index_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("upvalueid requires 2 arguments".to_string()))?;

    // Check that first argument is a function
    if !func.is_function() {
        return Err(l.error("bad argument #1 to 'upvalueid' (function expected)".to_string()));
    }

    let up_index = up_index_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'upvalueid' (number expected)".to_string()))?
        as usize;

    if let Some(lua_func) = func.as_lua_function() {
        let upvalues = lua_func.upvalues();
        if up_index > 0 && up_index <= upvalues.len() {
            let upvalue = &upvalues[up_index - 1];
            // Use the pointer address as unique ID
            let id = upvalue.as_ptr() as usize as i64;
            l.push_value(LuaValue::integer(id))?;
            return Ok(1);
        }
    }

    // Invalid upvalue index, return nil
    Ok(0)
}

/// debug.upvaluejoin(f1, n1, f2, n2) - Make upvalue n1 of f1 refer to upvalue n2 of f2
fn debug_upvaluejoin(l: &mut LuaState) -> LuaResult<usize> {
    let func1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("upvaluejoin requires 4 arguments".to_string()))?;
    let n1_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("upvaluejoin requires 4 arguments".to_string()))?;
    let func2 = l
        .get_arg(3)
        .ok_or_else(|| l.error("upvaluejoin requires 4 arguments".to_string()))?;
    let n2_val = l
        .get_arg(4)
        .ok_or_else(|| l.error("upvaluejoin requires 4 arguments".to_string()))?;

    // Check that arguments are functions
    if !func1.is_function() || !func2.is_function() {
        return Err(l.error("bad argument to 'upvaluejoin' (function expected)".to_string()));
    }

    // Check that they are Lua functions (not C functions)
    if func1.is_cfunction() || func2.is_cfunction() {
        return Err(l.error("bad argument to 'upvaluejoin' (Lua function expected)".to_string()));
    }

    let n1 = n1_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'upvaluejoin' (number expected)".to_string()))?
        as usize;
    let n2 = n2_val
        .as_integer()
        .ok_or_else(|| l.error("bad argument #4 to 'upvaluejoin' (number expected)".to_string()))?
        as usize;

    // Get both Lua functions
    let lua_func1 = func1
        .as_lua_function()
        .ok_or_else(|| l.error("upvaluejoin: function 1 is not a Lua function".to_string()))?;
    let lua_func2 = func2
        .as_lua_function()
        .ok_or_else(|| l.error("upvaluejoin: function 2 is not a Lua function".to_string()))?;

    // Check upvalue indices
    let upvalues1 = lua_func1.upvalues();
    let upvalues2 = lua_func2.upvalues();
    if n1 == 0 || n1 > upvalues1.len() {
        return Err(l.error(format!("invalid upvalue index {} for function 1", n1)));
    }
    if n2 == 0 || n2 > upvalues2.len() {
        return Err(l.error(format!("invalid upvalue index {} for function 2", n2)));
    }

    // Clone the upvalue from func2
    let upvalue_to_share = upvalues2[n2 - 1].clone();

    // Replace upvalue in func1 - we need mutable access
    let lua_func1_mut = func1.as_lua_function_mut().ok_or_else(|| {
        l.error("upvaluejoin: cannot get mutable reference to function 1".to_string())
    })?;

    let upvalues1_mut = lua_func1_mut.upvalues_mut();
    upvalues1_mut[n1 - 1] = upvalue_to_share;

    Ok(0)
}
