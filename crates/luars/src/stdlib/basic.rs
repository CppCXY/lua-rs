// Basic library (_G global functions)
// Implements: print, type, assert, error, tonumber, tostring,
// select, ipairs, pairs, next, pcall, xpcall, getmetatable, setmetatable,
// rawget, rawset, rawlen, rawequal, collectgarbage, dofile, loadfile, load

use std::rc::Rc;

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, LuaValueKind};
use crate::lua_vm::{LuaError, LuaResult, LuaState};

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
    eprintln!("[DEBUG] print: got {} args", args.len());
    for (i, arg) in args.iter().enumerate() {
        eprintln!(
            "[DEBUG] print: arg[{}] is_string={}, is_nil={}",
            i,
            arg.is_string(),
            arg.is_nil()
        );
    }
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
        LuaValueKind::Table => "table",
        LuaValueKind::Function | LuaValueKind::CFunction => "function",
        LuaValueKind::Userdata => "userdata",
        LuaValueKind::Thread => "thread",
    };

    let result = l.vm_mut().create_string(type_name);
    eprintln!(
        "[DEBUG] type(): Created string result, is_string={}, is_nil={}",
        result.is_string(),
        result.is_nil()
    );
    if result.is_string() {
        eprintln!("[DEBUG] type(): String ID = {:?}", result.as_string_id());
    }
    let top_before = l.stack_len();
    l.push_value(result)?;
    let top_after = l.stack_len();
    eprintln!(
        "[DEBUG] type(): top before={}, after={}",
        top_before, top_after
    );
    if let Some(pushed) = l.stack_get(top_after - 1) {
        eprintln!(
            "[DEBUG] type(): Pushed value is_string={}, is_nil={}",
            pushed.is_string(),
            pushed.is_nil()
        );
    }
    Ok(1)
}

/// assert(v [, message]) - Raise error if v is false or nil
fn lua_assert(l: &mut LuaState) -> LuaResult<usize> {
    let condition = l.get_arg(1).unwrap_or(LuaValue::nil());

    if !condition.is_truthy() {
        let message = l
            .get_arg(2)
            .and_then(|v| {
                v.as_string_id().and_then(|id| {
                    l.vm_mut()
                        .object_pool
                        .get_string(id)
                        .map(|s| s.as_str().to_string())
                })
            })
            .unwrap_or_else(|| "assertion failed!".to_string());
        return Err(l.error(message));
    }

    // Return all arguments (already on stack)
    Ok(l.arg_count())
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

/// tonumber(e [, base]) - Convert to number
fn lua_tonumber(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("tonumber() requires argument 1".to_string()))?;
    let base = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(10);

    if base < 2 || base > 36 {
        return Err(l.error("bad argument #2 to 'tonumber' (base out of range)".to_string()));
    }

    let vm = l.vm_mut();
    let result = match value.kind() {
        LuaValueKind::Integer => value.clone(),
        LuaValueKind::Float => value.clone(),
        LuaValueKind::String => {
            if let Some(string_id) = value.as_string_id() {
                if let Some(s) = vm.object_pool.get_string(string_id) {
                    let s_str = s.as_str().trim();
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
            } else {
                LuaValue::nil()
            }
        }
        _ => LuaValue::nil(),
    };

    l.push_value(result)?;
    Ok(1)
}

/// Parse a Lua number string, supporting hex integers and hex floats
fn parse_lua_number(s: &str) -> LuaValue {
    let s = s.trim();
    if s.is_empty() {
        return LuaValue::nil();
    }

    // Check for hex prefix (0x or 0X), with optional sign
    let (sign, rest) = if s.starts_with('-') {
        (-1i64, &s[1..])
    } else if s.starts_with('+') {
        (1i64, &s[1..])
    } else {
        (1i64, s)
    };

    let rest = rest.trim_start();

    if rest.starts_with("0x") || rest.starts_with("0X") {
        let hex_part = &rest[2..];

        // Check if this is a hex float (contains '.' or 'p'/'P')
        if hex_part.contains('.') || hex_part.to_lowercase().contains('p') {
            // Parse hex float: 0xAA.BB or 0xAA.BBpEE or 0xAApEE
            if let Some(f) = parse_hex_float(hex_part) {
                return LuaValue::float(sign as f64 * f);
            }
            return LuaValue::nil();
        }

        // Plain hex integer
        if let Ok(i) = u64::from_str_radix(hex_part, 16) {
            // Reinterpret as i64 for large values
            let i = i as i64;
            return LuaValue::integer(sign * i);
        }
        return LuaValue::nil();
    }

    // Regular decimal number
    if let Ok(i) = s.parse::<i64>() {
        return LuaValue::integer(i);
    }
    if let Ok(f) = s.parse::<f64>() {
        return LuaValue::float(f);
    }

    LuaValue::nil()
}

/// Parse hex float like "AA.BB" or "AA.BBpEE" (without 0x prefix)
fn parse_hex_float(s: &str) -> Option<f64> {
    let s_lower = s.to_lowercase();

    // Split by 'p' for exponent
    let (mantissa_str, exp_str) = if let Some(p_pos) = s_lower.find('p') {
        (&s[..p_pos], Some(&s[p_pos + 1..]))
    } else {
        (s, None)
    };

    // Parse mantissa (integer.fraction in hex)
    let mantissa = if let Some(dot_pos) = mantissa_str.find('.') {
        let int_part = &mantissa_str[..dot_pos];
        let frac_part = &mantissa_str[dot_pos + 1..];

        let int_val = if int_part.is_empty() {
            0u64
        } else {
            u64::from_str_radix(int_part, 16).ok()?
        };

        let frac_val = if frac_part.is_empty() {
            0.0
        } else {
            let frac_int = u64::from_str_radix(frac_part, 16).ok()?;
            frac_int as f64 / 16f64.powi(frac_part.len() as i32)
        };

        int_val as f64 + frac_val
    } else {
        let int_val = u64::from_str_radix(mantissa_str, 16).ok()?;
        int_val as f64
    };

    // Parse exponent (base 2)
    let exp = if let Some(exp_str) = exp_str {
        exp_str.parse::<i32>().ok()?
    } else {
        0
    };

    Some(mantissa * 2f64.powi(exp))
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

    let vm = l.vm_mut();

    // Fast path: simple types without metamethods
    let result = if value.is_nil() {
        vm.create_string("nil")
    } else if let Some(b) = value.as_bool() {
        vm.create_string(if b { "true" } else { "false" })
    } else if let Some(i) = value.as_integer() {
        vm.create_string(&i.to_string())
    } else if let Some(f) = value.as_number() {
        vm.create_string(&f.to_string())
    } else {
        // Slow path: check for __tostring metamethod or default representation
        let value_str = vm.value_to_string_raw(&value);
        vm.create_string(&value_str)
    };

    l.push_value(result)?;
    Ok(1)
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
    if let Some(string_id) = index_arg.as_string_id() {
        let vm = l.vm_mut();
        if let Some(s) = vm.object_pool.get_string(string_id) {
            if s.as_str() == "#" {
                let result = LuaValue::integer(vararg_count as i64);
                l.push_value(result)?;
                return Ok(1);
            }
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
    let result_count = vararg_count - start_idx;
    // Values are already on stack at positions 2+start_idx onwards
    // Just return the count
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
    if let Some(table_id) = table_val.as_table_id() {
        if let Some(index) = index_val.as_integer() {
            let next_index = index + 1;

            // Access table via ObjectPool
            let vm = l.vm_mut();
            if let Some(table) = vm.object_pool.get_table(table_id) {
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

    // TODO: Check for __pairs metamethod

    // Return next function, table, and nil (3 values)
    let next_func = LuaValue::cfunction(lua_next);
    l.push_value(next_func)?;
    l.push_value(table_val)?;
    l.push_value(LuaValue::nil())?;
    Ok(3)
}

/// next(table [, index]) - Return next key-value pair
/// TODO: Implement table.next() method on LuaTable
fn lua_next(l: &mut LuaState) -> LuaResult<usize> {
    // Temporary simplified implementation
    Err(l.error("next() not yet fully implemented".to_string()))
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

    // Results are already on stack starting at func_idx
    // We need to insert success boolean before them
    let success_val = LuaValue::boolean(success);

    // Insert success at func_idx position
    l.stack_insert(func_idx, success_val)?;

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

    let vm = l.vm_mut();
    match value.kind() {
        LuaValueKind::Table => {
            let Some(table_id) = value.as_table_id() else {
                return Err(l.error("Invalid table".to_string()));
            };

            // Get metatable
            let mt = {
                let Some(table_ref) = vm.object_pool.get_table(table_id) else {
                    return Err(l.error("Invalid table".to_string()));
                };
                table_ref.get_metatable()
            };

            if let Some(mt) = mt {
                // Check for __metatable field
                let metatable_key = vm.create_string("__metatable");
                if let Some(mt_id) = mt.as_table_id() {
                    if let Some(mt_table) = vm.object_pool.get_table(mt_id) {
                        if let Some(protected) = mt_table.raw_get(&metatable_key) {
                            if !protected.is_nil() {
                                l.push_value(protected)?;
                                return Ok(1);
                            }
                        }
                    }
                }
                l.push_value(mt)?;
                Ok(1)
            } else {
                l.push_value(LuaValue::nil())?;
                Ok(1)
            }
        }
        LuaValueKind::String => {
            // TODO: Implement shared string metatable
            l.push_value(LuaValue::nil())?;
            Ok(1)
        }
        LuaValueKind::Userdata => {
            // Return userdata metatable
            if let Some(ud_id) = value.as_userdata_id() {
                if let Some(ud) = vm.object_pool.get_userdata(ud_id) {
                    let mt = ud.get_metatable();
                    if !mt.is_nil() {
                        // Check for __metatable field
                        let metatable_key = vm.create_string("__metatable");
                        if let Some(mt_id) = mt.as_table_id() {
                            if let Some(mt_table) = vm.object_pool.get_table(mt_id) {
                                if let Some(protected) = mt_table.raw_get(&metatable_key) {
                                    if !protected.is_nil() {
                                        l.push_value(protected)?;
                                        return Ok(1);
                                    }
                                }
                            }
                        }
                        l.push_value(mt)?;
                        return Ok(1);
                    }
                }
            }
            l.push_value(LuaValue::nil())?;
            Ok(1)
        }
        // TODO: Support metatables for other types (numbers, etc.)
        _ => {
            l.push_value(LuaValue::nil())?;
            Ok(1)
        }
    }
}

/// setmetatable(table, metatable) - Set metatable
fn lua_setmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let table = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'setmetatable' (value expected)".to_string()))?;
    let metatable = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'setmetatable' (value expected)".to_string()))?;

    let vm = l.vm_mut();

    // Set the new metatable using ObjectPool
    let Some(table_id) = table.as_table_id() else {
        return Err(l.error("bad argument #1 to 'setmetatable' (table expected)".to_string()));
    };

    // Create the key first to avoid borrow issues
    let metatable_field = vm.create_string("__metatable");

    // Check if current metatable has __metatable field (protection)
    let is_protected = {
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(l.error("Invalid table".to_string()));
        };
        if let Some(current_mt) = table_ref.get_metatable() {
            if let Some(mt_id) = current_mt.as_table_id() {
                if let Some(mt_table) = vm.object_pool.get_table(mt_id) {
                    mt_table.raw_get(&metatable_field).is_some()
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        }
    };

    if is_protected {
        return Err(l.error("cannot change a protected metatable".to_string()));
    }

    // Now modify the table
    let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
        return Err(l.error("Invalid table".to_string()));
    };

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

    // Return the original table
    l.push_value(table.clone())?;
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

    if let Some(table_id) = table.as_table_id() {
        let vm = l.vm_mut();
        if let Some(table_ref) = vm.object_pool.get_table(table_id) {
            let value = table_ref.raw_get(&key).unwrap_or(LuaValue::nil());
            l.push_value(value)?;
            return Ok(1);
        }
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

    if let Some(table_id) = table.as_table_id() {
        let vm = l.vm_mut();
        if let Some(table_ref) = vm.object_pool.get_table_mut(table_id) {
            table_ref.raw_set(key, value);
            l.push_value(table)?;
            return Ok(1);
        }
    }
    Err(l.error("bad argument #1 to 'rawset' (table expected)".to_string()))
}

/// rawlen(v) - Length without metamethods
fn lua_rawlen(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rawlen' (value expected)".to_string()))?;

    let vm = l.vm_mut();
    let len = match value.kind() {
        LuaValueKind::Table => {
            if let Some(table_id) = value.as_table_id() {
                if let Some(table) = vm.object_pool.get_table(table_id) {
                    table.len() as i64
                } else {
                    return Err(l.error(
                        "bad argument #1 to 'rawlen' (table or string expected)".to_string(),
                    ));
                }
            } else {
                return Err(
                    l.error("bad argument #1 to 'rawlen' (table or string expected)".to_string())
                );
            }
        }
        LuaValueKind::String => {
            if let Some(string_id) = value.as_string_id() {
                if let Some(s) = vm.object_pool.get_string(string_id) {
                    s.as_str().len() as i64
                } else {
                    return Err(l.error(
                        "bad argument #1 to 'rawlen' (table or string expected)".to_string(),
                    ));
                }
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

    let object_pool = &l.vm_mut().object_pool;
    let result = v1.raw_equal(&v2, object_pool);
    l.push_value(LuaValue::boolean(result))?;
    Ok(1)
}

/// collectgarbage([opt [, arg]]) - Garbage collector control
fn lua_collectgarbage(l: &mut LuaState) -> LuaResult<usize> {
    let arg1 = l.get_arg(1);
    let vm = l.vm_mut();
    let opt = arg1
        .and_then(|v| {
            v.as_string_id().and_then(|id| {
                vm.object_pool
                    .get_string(id)
                    .map(|s| s.as_str().to_string())
            })
        })
        .unwrap_or_else(|| "collect".to_string());

    match opt.as_str() {
        "collect" => {
            vm.collect_garbage();
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "count" => {
            // Return a dummy value for now
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "stop" => {
            // Set GC debt to very negative value to prevent collection
            vm.gc.gc_debt = isize::MIN / 2;
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "restart" => {
            // Reset GC debt to trigger collection
            vm.gc.gc_debt = 0;
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "step" | "setpause" | "setstepmul" | "isrunning" => {
            // Simplified: just return 0
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        _ => Err(l.error(format!("collectgarbage: invalid option '{}'", opt))),
    }
}

/// require(modname) - Load a module  
/// Simplified implementation - loads from package.preload or package.path
fn lua_require(l: &mut LuaState) -> LuaResult<usize> {
    let modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'require' (string expected)".to_string()))?;

    let Some(modname_id) = modname_val.as_string_id() else {
        return Err(l.error("bad argument #1 to 'require' (string expected)".to_string()));
    };

    let modname_str = {
        let vm = l.vm_mut();
        let Some(s) = vm.object_pool.get_string(modname_id) else {
            return Err(l.error("bad argument #1 to 'require' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    // Get package.loaded
    let package_table = l
        .get_global("package")
        .ok_or_else(|| l.error("package table not found".to_string()))?;

    let Some(package_id) = package_table.as_table_id() else {
        return Err(l.error("package must be a table".to_string()));
    };

    let loaded_key = l.create_string("loaded");
    let loaded_val = {
        let vm = l.vm_mut();
        let Some(pkg_table) = vm.object_pool.get_table(package_id) else {
            return Err(l.error("package must be a table".to_string()));
        };
        pkg_table
            .raw_get(&loaded_key)
            .ok_or_else(|| l.error("package.loaded not found".to_string()))?
    };

    let Some(loaded_id) = loaded_val.as_table_id() else {
        return Err(l.error("package.loaded must be a table".to_string()));
    };

    // Check if module is already loaded
    let already_loaded = {
        let vm = l.vm_mut();
        let Some(loaded_table) = vm.object_pool.get_table(loaded_id) else {
            return Err(l.error("package.loaded must be a table".to_string()));
        };
        loaded_table
            .raw_get(&modname_val)
            .unwrap_or(LuaValue::nil())
    };

    // If module is already loaded and not nil/false, return it
    if !already_loaded.is_nil() {
        if let Some(b) = already_loaded.as_boolean() {
            if !b {
                // It's false, continue to load
            } else {
                l.push_value(already_loaded)?;
                return Ok(1);
            }
        } else {
            l.push_value(already_loaded)?;
            return Ok(1);
        }
    }

    // For now, return error suggesting package system not fully implemented
    // Full implementation would iterate through package.searchers
    Err(l.error(format!(
        "module '{}' not found (require not fully implemented yet)",
        modname_str
    )))
}

/// load(chunk [, chunkname [, mode [, env]]]) - Load a chunk
fn lua_load(l: &mut LuaState) -> LuaResult<usize> {
    let chunk_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'load' (value expected)".to_string()))?;
    let arg2 = l.get_arg(2);
    let arg3 = l.get_arg(3);
    let arg4 = l.get_arg(4);

    let vm = l.vm_mut();

    // Get the chunk string
    let Some(string_id) = chunk_val.as_string_id() else {
        return Err(l.error("bad argument #1 to 'load' (string expected)".to_string()));
    };
    let code_str = {
        let Some(code) = vm.object_pool.get_string(string_id) else {
            return Err(l.error("bad argument #1 to 'load' (string expected)".to_string()));
        };
        code.as_str().to_string()
    };

    // Optional chunk name for error messages
    let chunkname = arg2
        .and_then(|v| {
            v.as_string_id().and_then(|id| {
                vm.object_pool
                    .get_string(id)
                    .map(|s| s.as_str().to_string())
            })
        })
        .unwrap_or_else(|| "=(load)".to_string());

    // Optional mode ("b", "t", or "bt") - we only support "t" (text)
    let _mode = arg3
        .and_then(|v| {
            v.as_string_id().and_then(|id| {
                vm.object_pool
                    .get_string(id)
                    .map(|s| s.as_str().to_string())
            })
        })
        .unwrap_or_else(|| "bt".to_string());

    // Optional environment table
    let env = arg4;

    // Compile the code using VM's string pool with chunk name
    match vm.compile_with_name(&code_str, &chunkname) {
        Ok(chunk) => {
            // Create upvalue for _ENV (global table)
            let env_upvalue_id = if let Some(env) = env {
                vm.create_upvalue_closed(env)
            } else {
                vm.create_upvalue_closed(LuaValue::table(vm.global))
            };
            let upvalues = vec![env_upvalue_id];

            let func = vm.create_function(Rc::new(chunk), upvalues);
            l.push_value(func)?;
            Ok(1)
        }
        Err(e) => {
            // Return nil and error message
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

    let vm = l.vm_mut();
    let Some(string_id) = filename.as_string_id() else {
        return Err(l.error("bad argument #1 to 'loadfile' (string expected)".to_string()));
    };
    let filename_str = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(l.error("bad argument #1 to 'loadfile' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

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
            let env_upvalue_id = vm.create_upvalue_closed(LuaValue::table(vm.global));
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
fn lua_dofile(l: &mut LuaState) -> LuaResult<usize> {
    // Temporary simplified implementation
    Err(l.error("dofile() not yet fully implemented".to_string()))
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
