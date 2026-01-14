// Basic library (_G global functions)
// Implements: print, type, assert, error, tonumber, tostring,
// select, ipairs, pairs, next, pcall, xpcall, getmetatable, setmetatable,
// rawget, rawset, rawlen, rawequal, collectgarbage, dofile, loadfile, load
mod require;

use std::rc::Rc;

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, LuaValueKind};
use crate::lua_vm::{LuaError, LuaResult, LuaState, get_metamethod_event, get_metatable};
use require::lua_require;

pub fn create_basic_lib() -> LibraryModule {
    crate::lib_module!("_G", {
        "print" => lua_print,
        "type" => lua_type,
        "assert" => lua_assert,
        "error" => lua_error,
        "tonumber" => lua_tonumber,
        "tostring" => lua_tostring,
        "select" => lua_select,
        "ipairs" => lua_ipairs,
        "pairs" => lua_pairs,
        "next" => lua_next,
        "pcall" => lua_pcall,
        "xpcall" => lua_xpcall,
        "getmetatable" => lua_getmetatable,
        "setmetatable" => lua_setmetatable,
        "rawget" => lua_rawget,
        "rawset" => lua_rawset,
        "rawlen" => lua_rawlen,
        "rawequal" => lua_rawequal,
        "collectgarbage" => lua_collectgarbage,
        "require" => lua_require,
        "load" => lua_load,
        "loadfile" => lua_loadfile,
        "dofile" => lua_dofile,
        "warn" => lua_warn,
    })
    .with_value("_VERSION", |vm| vm.create_string("Lua 5.5"))
}

/// print(...) - Print values to stdout
fn lua_print(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();
    let vm = l.vm_mut();

    let output: Vec<String> = args.iter().map(|v| vm.value_to_string_raw(v)).collect();

    if !output.is_empty() {
        println!("{}", output.join("\t"));
    } else {
        println!();
    }

    Ok(0)
}

/// type(v) - Return the type of a value as a string
fn lua_type(l: &mut LuaState) -> LuaResult<usize> {
    let value = match l.get_arg(1) {
        Some(v) => v,
        None => {
            return Err(l.error("bad argument #1 to 'type' (value expected)".to_string()));
        }
    };

    let type_name = match value.kind() {
        LuaValueKind::Nil => "nil",
        LuaValueKind::Boolean => "boolean",
        LuaValueKind::Integer | LuaValueKind::Float => "number",
        LuaValueKind::String => "string",
        LuaValueKind::Binary => "string", // Binary is also a string type
        LuaValueKind::Table => "table",
        LuaValueKind::Function | LuaValueKind::CFunction => "function",
        LuaValueKind::Userdata => "userdata",
        LuaValueKind::Thread => "thread",
    };

    let result = l.vm_mut().create_string(type_name);
    l.push_value(result)?;
    Ok(1)
}

/// assert(v [, message]) - Raise error if v is false or nil
fn lua_assert(l: &mut LuaState) -> LuaResult<usize> {
    let arg_count = l.arg_count();

    // Get first argument without consuming it
    let condition = l.get_arg(1).unwrap_or(LuaValue::nil());

    if !condition.is_truthy() {
        let message = l
            .get_arg(2)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "assertion failed!".to_string());
        return Err(l.error(message));
    }

    // Return all arguments - they are already on stack
    // Just return the count
    Ok(arg_count)
}

/// error(message) - Raise an error
fn lua_error(l: &mut LuaState) -> LuaResult<usize> {
    let arg = l.get_arg(1);
    let vm = l.vm_mut();
    let message = arg
        .map(|v| vm.value_to_string_raw(&v))
        .unwrap_or_else(|| "error".to_string());

    Err(l.error(message))
}

fn parse_lua_number(s: &str) -> LuaValue {
    let s = s.trim();
    if s.is_empty() {
        return LuaValue::nil();
    }

    // Handle sign
    let (sign, rest) = if s.starts_with('-') {
        (-1i64, &s[1..])
    } else if s.starts_with('+') {
        (1i64, &s[1..])
    } else {
        (1i64, s)
    };

    let rest = rest.trim_start();

    // Check for hex prefix (0x or 0X)
    if rest.starts_with("0x") || rest.starts_with("0X") {
        let hex_part = &rest[2..];

        // Hex float contains '.' or 'p'/'P' - always treat as float
        if hex_part.contains('.') || hex_part.to_lowercase().contains('p') {
            // For hex float, just try to parse as float (Rust doesn't support hex float literals)
            // Return nil for now - proper implementation would need custom parser
            return LuaValue::nil();
        }

        // Plain hex integer
        if let Ok(i) = u64::from_str_radix(hex_part, 16) {
            let i = i as i64;
            return LuaValue::integer(sign * i);
        }
        return LuaValue::nil();
    }

    // Decimal number - determine if integer or float
    // Integer: no '.' and no 'e'/'E'
    // Float: contains '.' or 'e'/'E'
    let has_dot = rest.contains('.');
    let has_exponent = rest.to_lowercase().contains('e');

    if !has_dot && !has_exponent {
        // Try as integer
        if let Ok(i) = s.parse::<i64>() {
            return LuaValue::integer(i);
        }
    }

    // Try as float (either has '.'/e' or integer parse failed due to overflow)
    if let Ok(f) = s.parse::<f64>() {
        return LuaValue::float(f);
    }

    LuaValue::nil()
}

/// tonumber(e [, base]) - Convert to number
fn lua_tonumber(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("tonumber() requires argument 1".to_string()))?;
    let base = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(10);

    if base < 2 || base > 36 {
        return Err(l.error("bad argument #2 to 'tonumber' (base out of range)".to_string()));
    }

    let result = match value.kind() {
        LuaValueKind::Integer => value.clone(),
        LuaValueKind::Float => value.clone(),
        LuaValueKind::String => {
            if let Some(s) = value.as_str() {
                let s_str = s.trim();
                if base == 10 {
                    parse_lua_number(s_str)
                } else {
                    if let Ok(i) = i64::from_str_radix(s_str, base as u32) {
                        LuaValue::integer(i)
                    } else {
                        LuaValue::nil()
                    }
                }
            } else {
                LuaValue::nil()
            }
        }
        _ => LuaValue::nil(),
    };

    l.push_value(result)?;
    Ok(1)
}

/// tostring(v) - Convert to string
/// OPTIMIZED: Fast path for common types
fn lua_tostring(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("tostring() requires argument 1".to_string()))?;

    // Fast path: if already a string, return it directly
    if value.is_string() {
        l.push_value(value)?;
        return Ok(1);
    }

    // Fast path: simple types without metamethods
    if value.is_nil() || value.as_bool().is_some() || value.is_integer() || value.is_number() {
        let vm = l.vm_mut();
        let s = if value.is_nil() {
            vm.create_string("nil")
        } else if let Some(b) = value.as_bool() {
            vm.create_string(if b { "true" } else { "false" })
        } else if let Some(i) = value.as_integer() {
            vm.create_string(&i.to_string())
        } else if let Some(f) = value.as_number() {
            vm.create_string(&f.to_string())
        } else {
            unreachable!()
        };
        l.push_value(s)?;
        return Ok(1);
    }

    // Check for __tostring metamethod
    if let Some(mm) = get_tostring_metamethod(l, &value) {
        // Call __tostring metamethod
        let (succ, results) = l.pcall(mm, vec![value])?;
        if !succ {
            return Err(l.error("error in __tostring metamethod".to_string()));
        }
        if let Some(result) = results.get(0) {
            l.push_value(result.clone())?;
            return Ok(1);
        } else {
            return Err(l.error("error in __tostring metamethod: no result".to_string()));
        }
    }

    // No metamethod: use default representation
    let vm = l.vm_mut();
    let value_str = vm.value_to_string_raw(&value);
    let result = vm.create_string(&value_str);
    l.push_value(result)?;
    Ok(1)
}

/// Get __tostring metamethod for a value
fn get_tostring_metamethod(lua_state: &mut LuaState, value: &LuaValue) -> Option<LuaValue> {
    get_metamethod_event(lua_state, value, "__tostring")
}

/// select(index, ...) - Return subset of arguments
/// ULTRA-OPTIMIZED: Fast path for "#" and direct stack manipulation
fn lua_select(l: &mut LuaState) -> LuaResult<usize> {
    let index_arg = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'select' (value expected)".to_string()))?;

    // Total args after index
    let total_args = l.arg_count();
    let vararg_count = if total_args > 0 { total_args - 1 } else { 0 };

    // FAST PATH: Check for "#"
    if let Some(s) = index_arg.as_str() {
        if s == "#" {
            let result = LuaValue::integer(vararg_count as i64);
            l.push_value(result)?;
            return Ok(1);
        }
        return Err(l.error("bad argument #1 to 'select' (number expected)".to_string()));
    }

    let index = index_arg
        .as_integer()
        .ok_or_else(|| l.error("bad argument #1 to 'select' (number expected)".to_string()))?;

    if index == 0 {
        return Err(l.error("bad argument #1 to 'select' (index out of range)".to_string()));
    }

    // Calculate start position (1-based to 0-based)
    let start_idx = if index > 0 {
        (index - 1) as usize
    } else {
        let abs_idx = (-index) as usize;
        if abs_idx > vararg_count {
            return Err(l.error("bad argument #1 to 'select' (index out of range)".to_string()));
        }
        vararg_count - abs_idx
    };

    if start_idx >= vararg_count {
        return Ok(0);
    }

    // Return values from start_idx onward (arg 2, 3, ...)
    // Need to PUSH the values to the stack (C function results must be on stack)
    let result_count = vararg_count - start_idx;

    // Argument indices: arg 1 is at base (index parameter)
    // arg 2, 3, ... are at base+1, base+2, ...
    // We want to return from arg (2+start_idx) onwards
    let first_arg_idx = 2 + start_idx; // This is 1-based argument index

    // Push each result value onto the stack
    for i in 0..result_count {
        if let Some(val) = l.get_arg(first_arg_idx + i) {
            l.push_value(val)?;
        } else {
            l.push_value(LuaValue::nil())?;
        }
    }

    Ok(result_count)
}

/// ipairs(t) - Return iterator for array part of table
fn lua_ipairs(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'ipairs' (value expected)".to_string()))?;

    // Validate that it's a table
    if !table_val.is_table() {
        return Err(l.error("bad argument #1 to 'ipairs' (table expected)".to_string()));
    }

    // Return iterator function, table, and 0 (3 values)
    let iter_func = LuaValue::cfunction(ipairs_next);
    l.push_value(iter_func)?;
    l.push_value(table_val)?;
    l.push_value(LuaValue::integer(0))?;
    Ok(3)
}

/// Iterator function for ipairs - Optimized for performance
#[inline]
fn ipairs_next(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("ipairs iterator: missing table".to_string()))?;
    let index_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("ipairs iterator: missing index".to_string()))?;

    // Fast path: both table and index are valid
    if let Some(table) = table_val.as_table() {
        if let Some(index) = index_val.as_integer() {
            let next_index = index + 1;

            if let Some(value) = table.get_int(next_index) {
                // Return (next_index, value)
                l.push_value(LuaValue::integer(next_index))?;
                l.push_value(value)?;
                return Ok(2);
            }
            // Reached end of array - return nil
            l.push_value(LuaValue::nil())?;
            return Ok(1);
        }
    }

    // Slow path with error
    Err(l.error("ipairs iterator: invalid table or index".to_string()))
}

/// pairs(t) - Return iterator for all key-value pairs
fn lua_pairs(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'pairs' (value expected)".to_string()))?;

    // Validate that it's a table
    if table_val.as_table_id().is_none() {
        return Err(l.error("bad argument #1 to 'pairs' (table expected)".to_string()));
    }

    // Return next function, table, and nil (3 values)
    let next_func = LuaValue::cfunction(lua_next);
    l.push_value(next_func)?;
    l.push_value(table_val)?;
    l.push_value(LuaValue::nil())?;
    Ok(3)
}

/// next(table [, index]) - Return next key-value pair
/// Port of Lua 5.5's luaB_next using luaH_next
fn lua_next(l: &mut LuaState) -> LuaResult<usize> {
    // next(table [, index])
    // Returns the next index-value pair in the table
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'next' (table expected)".to_string()))?;

    let index_val = l.get_arg(2).unwrap_or(LuaValue::nil());

    // Use efficient lua_next implementation - direct table access
    let table = table_val
        .as_table()
        .ok_or_else(|| l.error("bad argument #1 to 'next' (table expected)".to_string()))?;

    // First, if index is not nil, verify it exists in the table
    // This distinguishes "key not found" from "no more keys"
    if !index_val.is_nil() {
        let key_exists = table.raw_get(&index_val).is_some();
        if !key_exists {
            return Err(l.error("invalid key to 'next'".to_string()));
        }
    }

    let result = table.next(&index_val);

    // Return next key-value pair, or nil if at end
    if let Some((k, v)) = result {
        l.push_value(k)?;
        l.push_value(v)?;
        Ok(2)
    } else {
        l.push_value(LuaValue::nil())?;
        Ok(1)
    }
}

/// pcall(f [, arg1, ...]) - Protected call
fn lua_pcall(l: &mut LuaState) -> LuaResult<usize> {
    // Arguments are already on stack from the call:
    // stack: [pcall_func, target_func, arg1, arg2, ...]
    // We need: [target_func, arg1, arg2, ...] and call it

    let arg_count = l.arg_count();
    if arg_count < 1 {
        return Err(l.error("bad argument #1 to 'pcall' (value expected)".to_string()));
    }

    // Get current frame info
    let base = l
        .current_frame()
        .map(|f| f.base)
        .ok_or_else(|| LuaError::RuntimeError)?;

    // func is at base+0, args are at base+1..base+arg_count-1
    // We want to call func with arg_count-1 arguments
    let func_idx = base;
    let call_arg_count = arg_count - 1;

    // Call using stack-based API (no Vec allocation!)
    let (success, result_count) = l.pcall_stack_based(func_idx, call_arg_count)?;

    // pcall_stack_based leaves results at func_idx
    // We need to push them so call_c_function can handle them correctly
    // Strategy: collect results, then push them

    // Collect all results (success flag + actual results)
    let mut all_results = Vec::with_capacity(result_count + 1);
    all_results.push(LuaValue::boolean(success));
    for i in 0..result_count {
        if let Some(val) = l.stack_get(func_idx + i) {
            all_results.push(val);
        } else {
            all_results.push(LuaValue::nil());
        }
    }

    // Push all results so call_c_function can collect them
    for val in all_results {
        l.push_value(val)?;
    }

    Ok(result_count + 1)
}

/// xpcall(f, msgh [, arg1, ...]) - Protected call with error handler
fn lua_xpcall(l: &mut LuaState) -> LuaResult<usize> {
    // Get function (first argument)
    let func = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'xpcall' (value expected)".to_string()))?;

    // Get error handler (second argument)
    let err_handler = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'xpcall' (value expected)".to_string()))?;

    // Collect remaining arguments
    let mut args = Vec::new();
    let arg_count = l.arg_count();
    for i in 3..=arg_count {
        if let Some(arg) = l.get_arg(i) {
            args.push(arg);
        }
    }

    // Call xpcall
    let (success, results) = l.xpcall(func, args, err_handler)?;

    // Push success status
    l.push_value(LuaValue::boolean(success))?;

    // Push results and count them
    let result_count = results.len();
    for result in results {
        l.push_value(result)?;
    }

    Ok(1 + result_count)
}

/// getmetatable(object) - Get metatable
fn lua_getmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'getmetatable' (value expected)".to_string()))?;

    let v = get_metatable(l, &value).unwrap_or(LuaValue::nil());
    l.push_value(v)?;
    Ok(1)
}

/// setmetatable(table, metatable) - Set metatable
fn lua_setmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let table = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'setmetatable' (value expected)".to_string()))?;
    let metatable = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'setmetatable' (value expected)".to_string()))?;

    // Now modify the table
    if let Some(table_ref) = table.as_table_mut() {
        match metatable.kind() {
            LuaValueKind::Nil => {
                table_ref.set_metatable(None);
            }
            LuaValueKind::Table => {
                table_ref.set_metatable(Some(metatable.clone()));
            }
            _ => {
                return Err(
                    l.error("setmetatable() second argument must be a table or nil".to_string())
                );
            }
        }
    }
    // Return the original table
    l.push_value(table)?;
    Ok(1)
}

/// rawget(table, index) - Get without metamethods
fn lua_rawget(l: &mut LuaState) -> LuaResult<usize> {
    let table = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rawget' (value expected)".to_string()))?;
    let key = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'rawget' (value expected)".to_string()))?;

    if let Some(table_ref) = table.as_table() {
        let value = table_ref.raw_get(&key).unwrap_or(LuaValue::nil());
        l.push_value(value)?;
        return Ok(1);
    }
    Err(l.error("bad argument #1 to 'rawget' (table expected)".to_string()))
}

/// rawset(table, index, value) - Set without metamethods
fn lua_rawset(l: &mut LuaState) -> LuaResult<usize> {
    let table = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rawset' (value expected)".to_string()))?;
    let key = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'rawset' (value expected)".to_string()))?;
    let value = l
        .get_arg(3)
        .ok_or_else(|| l.error("bad argument #3 to 'rawset' (value expected)".to_string()))?;

    if let Some(table_ref) = table.as_table_mut() {
        table_ref.raw_set(&key, value);
        l.push_value(table)?;
        return Ok(1);
    }
    Err(l.error("bad argument #1 to 'rawset' (table expected)".to_string()))
}

/// rawlen(v) - Length without metamethods
fn lua_rawlen(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rawlen' (value expected)".to_string()))?;

    let len = match value.kind() {
        LuaValueKind::Table => {
            if let Some(table) = value.as_table() {
                table.len() as i64
            } else {
                return Err(
                    l.error("bad argument #1 to 'rawlen' (table or string expected)".to_string())
                );
            }
        }
        LuaValueKind::String => {
            if let Some(s) = value.as_str() {
                s.len() as i64
            } else {
                return Err(
                    l.error("bad argument #1 to 'rawlen' (table or string expected)".to_string())
                );
            }
        }
        _ => {
            return Err(
                l.error("bad argument #1 to 'rawlen' (table or string expected)".to_string())
            );
        }
    };

    l.push_value(LuaValue::integer(len))?;
    Ok(1)
}

/// rawequal(v1, v2) - Equality without metamethods
fn lua_rawequal(l: &mut LuaState) -> LuaResult<usize> {
    let v1 = l.get_arg(1).unwrap_or(LuaValue::nil());
    let v2 = l.get_arg(2).unwrap_or(LuaValue::nil());

    let result = v1 == v2;
    l.push_value(LuaValue::boolean(result))?;
    Ok(1)
}

/// collectgarbage([opt [, arg, arg2]]) - Garbage collector control
/// Lua 5.5 version with full parameter support including the new 'param' option
fn lua_collectgarbage(l: &mut LuaState) -> LuaResult<usize> {
    let arg1 = l.get_arg(1);

    let opt = arg1
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "collect".to_string());

    match opt.as_str() {
        "collect" => {
            l.collect_garbage()?;
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "count" => {
            let gc = &l.vm_mut().gc;
            let total = gc.total_bytes;
            let kb = total.max(0) as f64 / 1024.0;
            l.push_value(LuaValue::number(kb))?;
            Ok(1)
        }
        "stop" => {
            // LUA_GCSTOP: Stop collector (like Lua's gcstp = GCSTPUSR)
            l.vm_mut().gc.gc_stopped = true;
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "restart" => {
            // LUA_GCRESTART: Restart collector
            l.vm_mut().gc.gc_stopped = false;
            l.vm_mut().gc.set_debt(0); // Reset debt to allow GC to run
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "step" => {
            // LUA_GCSTEP: Single step with optional size argument (in KB)
            // Like Lua 5.5's lapi.c:1203-1212
            // NOTE: This should work even when GC is stopped (force=true)
            let arg2 = l.get_arg(2);
            let step_size_kb = arg2.and_then(|v| v.as_integer()).unwrap_or(0);

            // Save old state to check if we complete a cycle
            let old_state = l.vm_mut().gc.gc_state;

            // Calculate n (bytes to subtract from debt)
            // If n <= 0, use current GCdebt to force one basic step
            let n = if step_size_kb <= 0 {
                // If 0, use current debt (force standard step)
                // If negative debt, we'll effectively use min_step
                l.vm_mut().gc.gc_debt
            } else {
                (step_size_kb * 1024) as isize
            };

            // Set debt to n to force proportional work
            // set_debt preserves real_bytes, so this is safe
            l.vm_mut().gc.set_debt(n);

            // Perform the GC step with force=true (ignore gc_stopped)
            let vm = l.vm_mut();
            let roots = vm.collect_roots();
            vm.gc.step_internal(&roots, &mut vm.object_pool, true);

            // Check if we completed a full cycle
            // A full cycle means: either we reached Pause from non-Pause,
            // or we started at Pause and went through at least one non-Pause state
            let completed = {
                let vm = l.vm_mut();
                let current_state = vm.gc.gc_state;

                // If we started at Pause and are still at Pause, we didn't do a full cycle
                // If we started at non-Pause and reached Pause, we completed a cycle
                matches!(current_state, crate::gc::GcState::Pause)
                    && !matches!(old_state, crate::gc::GcState::Pause)
            };

            l.push_value(LuaValue::boolean(completed))?;
            Ok(1)
        }
        "isrunning" => {
            // LUA_GCISRUNNING: Check if collector is running
            // GC is running if not stopped by user
            let is_running = !l.vm_mut().gc.gc_stopped;
            l.push_value(LuaValue::boolean(is_running))?;
            Ok(1)
        }
        "generational" => {
            // LUA_GCGEN: Switch to generational mode
            let vm = l.vm_mut();
            let old_mode = match vm.gc.gc_kind {
                crate::gc::GcKind::Inc => "incremental",
                crate::gc::GcKind::GenMinor => "generational",
                crate::gc::GcKind::GenMajor => "generational",
            };

            // Switch to generational mode
            vm.gc.gc_kind = crate::gc::GcKind::GenMinor;

            // Push previous mode name (must track if new)
            let mode_value = vm.create_string(old_mode);
            l.push_value(mode_value)?;
            Ok(1)
        }
        "incremental" => {
            // LUA_GCINC: Switch to incremental mode
            let vm = l.vm_mut();
            let old_mode = match vm.gc.gc_kind {
                crate::gc::GcKind::Inc => "incremental",
                crate::gc::GcKind::GenMinor => "generational",
                crate::gc::GcKind::GenMajor => "generational",
            };

            // Switch to incremental mode (like luaC_changemode in Lua 5.5)
            vm.gc.change_to_incremental_mode(&mut vm.object_pool);

            let mode_value = vm.create_string(old_mode);
            l.push_value(mode_value)?;
            Ok(1)
        }
        "param" => {
            // LUA_GCPARAM: Get/set GC parameters (NEW in Lua 5.5!)
            let arg2 = l.get_arg(2);
            let arg3 = l.get_arg(3);

            // Get parameter name string
            let param_name = if let Some(v) = arg2 {
                v.as_str().map(|s| s.to_string())
            } else {
                None
            };

            if param_name.is_none() {
                return Err(l.error("collectgarbage 'param': parameter name expected".to_string()));
            }

            let param_name = param_name.unwrap();

            // Map parameter name to index
            let param_idx = match param_name.as_str() {
                "minormul" => Some(crate::gc::MINORMUL), // 3: LUA_GCPMINORMUL
                "majorminor" => Some(crate::gc::MAJORMINOR), // 5: LUA_GCPMAJORMINOR
                "minormajor" => Some(crate::gc::MINORMAJOR), // 4: LUA_GCPMINORMAJOR
                "pause" => Some(crate::gc::PAUSE),       // 0: LUA_GCPPAUSE
                "stepmul" => Some(crate::gc::STEPMUL),   // 1: LUA_GCPSTEPMUL
                "stepsize" => Some(crate::gc::STEPSIZE), // 2: LUA_GCPSTEPSIZE
                _ => None,
            };

            if param_idx.is_none() {
                return Err(l.error(format!(
                    "collectgarbage 'param': invalid parameter name '{}'",
                    param_name
                )));
            }

            let param_idx = param_idx.unwrap();

            // Get old value and potentially set new value
            let old_value = {
                let vm = l.vm_mut();
                let old = vm.gc.gc_params[param_idx];

                // Set new value if provided
                if let Some(new_val) = arg3 {
                    if let Some(new_int) = new_val.as_integer() {
                        vm.gc.gc_params[param_idx] = new_int as i32;
                    }
                }

                old
            };

            // Return old value
            l.push_value(LuaValue::integer(old_value as i64))?;
            Ok(1)
        }
        _ => Err(l.error(format!("collectgarbage: invalid option '{}'", opt))),
    }
}

/// load(chunk [, chunkname [, mode [, env]]]) - Load a chunk
fn lua_load(l: &mut LuaState) -> LuaResult<usize> {
    use crate::lua_value::chunk_serializer;

    let chunk_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'load' (value expected)".to_string()))?;
    let arg2 = l.get_arg(2);
    let arg3 = l.get_arg(3);
    let arg4 = l.get_arg(4);

    // Get the chunk string or binary data
    let (code_bytes, is_binary) = if let Some(b) = chunk_val.as_binary() {
        (b.to_vec(), true)
    } else if let Some(s) = chunk_val.as_str() {
        // Check if this is binary bytecode by looking at first byte
        let is_binary = s.as_bytes().first() == Some(&0x1B);
        (s.as_bytes().to_vec(), is_binary)
    } else {
        return Err(l.error("bad argument #1 to 'load' (string expected)".to_string()));
    };

    // Optional chunk name for error messages
    let chunkname = arg2
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "=(load)".to_string());

    // Optional mode ("b", "t", or "bt") - we only support "t" (text)
    let _mode = arg3
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "bt".to_string());

    // Optional environment table
    let env = arg4;

    let chunk_result = if is_binary {
        // Deserialize binary bytecode
        let vm = l.vm_mut();
        match chunk_serializer::deserialize_chunk_with_strings(&code_bytes) {
            Ok((mut chunk, string_constants)) => {
                // Register string constants in the object pool and update constants
                for (const_idx, string_val) in string_constants {
                    let string_id = vm.create_string_owned(string_val);
                    if const_idx < chunk.constants.len() {
                        chunk.constants[const_idx] = string_id;
                    }
                }
                Ok(chunk)
            }
            Err(e) => Err(format!("binary load error: {}", e)),
        }
    } else {
        // Compile text code using VM's string pool with chunk name
        let code_str = match String::from_utf8(code_bytes) {
            Ok(s) => s,
            Err(_) => return Err(l.error("invalid UTF-8 in text code".to_string())),
        };
        let vm = l.vm_mut();
        vm.compile_with_name(&code_str, &chunkname)
            .map_err(|e| format!("{}", e))
    };

    match chunk_result {
        Ok(chunk) => {
            let vm = l.vm_mut();
            // Create upvalue for _ENV (global table)
            let env_upvalue_id = if let Some(env) = env {
                vm.create_upvalue_closed(env)
            } else {
                vm.create_upvalue_closed(vm.global)
            };
            let upvalues = vec![env_upvalue_id];

            let func = vm.create_function(Rc::new(chunk), upvalues);
            l.push_value(func)?;
            Ok(1)
        }
        Err(e) => {
            // Return nil and error message
            let vm = l.vm_mut();
            let err_msg = vm.create_string(&format!("load error: {}", e));
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            Ok(2)
        }
    }
}

/// loadfile([filename [, mode [, env]]]) - Load a file as a chunk
fn lua_loadfile(l: &mut LuaState) -> LuaResult<usize> {
    let filename = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'loadfile' (value expected)".to_string()))?;

    let filename_str = if let Some(s) = filename.as_str() {
        s.to_string()
    } else {
        return Err(l.error("bad argument #1 to 'loadfile' (string expected)".to_string()));
    };

    let vm = l.vm_mut();

    // Load from specified file
    let code = match std::fs::read_to_string(&filename_str) {
        Ok(c) => c,
        Err(e) => {
            let err_msg = vm.create_string(&format!("cannot open {}: {}", filename_str, e));
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            return Ok(2);
        }
    };

    // Compile the code using VM's string pool with chunk name
    let chunkname = format!("@{}", filename_str);
    match vm.compile_with_name(&code, &chunkname) {
        Ok(chunk) => {
            // Create upvalue for _ENV (global table)
            let env_upvalue_id = vm.create_upvalue_closed(vm.global);
            let upvalues = vec![env_upvalue_id];
            let func = vm.create_function(std::rc::Rc::new(chunk), upvalues);
            l.push_value(func)?;
            Ok(1)
        }
        Err(e) => {
            let err_msg = vm.create_string(&format!("load error: {}", e));
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            Ok(2)
        }
    }
}

/// dofile([filename]) - Execute a file
/// TODO: Implement protected_call for full functionality
/// dofile([filename]) - Execute a Lua file
/// If no filename is given, executes from stdin
fn lua_dofile(l: &mut LuaState) -> LuaResult<usize> {
    let arg1 = l.get_arg(1);

    // Get filename (nil/none means stdin, which we don't support yet)
    let filename_str = if let Some(v) = arg1 {
        if v.is_nil() {
            return Err(l.error("dofile: reading from stdin not yet implemented".to_string()));
        }
        if let Some(s) = v.as_str() {
            s.to_string()
        } else {
            return Err(l.error("bad argument #1 to 'dofile' (string expected)".to_string()));
        }
    } else {
        return Err(l.error("dofile: reading from stdin not yet implemented".to_string()));
    };

    // Load from file
    let vm = l.vm_mut();
    let code = match std::fs::read_to_string(&filename_str) {
        Ok(c) => c,
        Err(e) => {
            return Err(l.error(format!("cannot open {}: {}", filename_str, e)));
        }
    };

    // Compile the code
    let chunkname = format!("@{}", filename_str);
    let chunk = match vm.compile_with_name(&code, &chunkname) {
        Ok(chunk) => chunk,
        Err(e) => {
            return Err(l.error(format!("error loading {}: {}", filename_str, e)));
        }
    };

    // Create function with _ENV upvalue (global table)
    let env_upvalue_id = vm.create_upvalue_closed(vm.global);
    let upvalues = vec![env_upvalue_id];
    let func = vm.create_function(std::rc::Rc::new(chunk), upvalues);

    // Call the function with 0 arguments
    let (success, results) = l.pcall(func, vec![])?;

    if !success {
        // Error occurred - results[0] contains error message
        if !results.is_empty() {
            return Err(LuaError::RuntimeError);
        }
        return Err(l.error("error in dofile".to_string()));
    }

    // Push all results
    let num_results = results.len();
    for result in results {
        l.push_value(result)?;
    }

    Ok(num_results)
}

/// warn(msg1, ...) - Emit a warning
fn lua_warn(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();
    let vm = l.vm_mut();

    let messages: Vec<String> = args.iter().map(|v| vm.value_to_string_raw(v)).collect();
    let message = messages.join("");

    // Emit warning to stderr
    eprintln!("Lua warning: {}", message);

    Ok(0)
}
