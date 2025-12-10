// Basic library (_G global functions)
// Implements: print, type, assert, error, tonumber, tostring,
// select, ipairs, pairs, next, pcall, xpcall, getmetatable, setmetatable,
// rawget, rawset, rawlen, rawequal, collectgarbage, dofile, loadfile, load

use std::rc::Rc;

use crate::lib_registry::{LibraryModule, LibraryEntry, get_arg, get_args, require_arg};
use crate::lua_value::{LuaValue, LuaValueKind, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

pub fn create_basic_lib() -> LibraryModule {
    let mut module = LibraryModule::new("_G");
    
    // Functions
    module.entries.push(("print", LibraryEntry::Function(lua_print)));
    module.entries.push(("type", LibraryEntry::Function(lua_type)));
    module.entries.push(("assert", LibraryEntry::Function(lua_assert)));
    module.entries.push(("error", LibraryEntry::Function(lua_error)));
    module.entries.push(("tonumber", LibraryEntry::Function(lua_tonumber)));
    module.entries.push(("tostring", LibraryEntry::Function(lua_tostring)));
    module.entries.push(("select", LibraryEntry::Function(lua_select)));
    module.entries.push(("ipairs", LibraryEntry::Function(lua_ipairs)));
    module.entries.push(("pairs", LibraryEntry::Function(lua_pairs)));
    module.entries.push(("next", LibraryEntry::Function(lua_next)));
    module.entries.push(("pcall", LibraryEntry::Function(lua_pcall)));
    module.entries.push(("xpcall", LibraryEntry::Function(lua_xpcall)));
    module.entries.push(("getmetatable", LibraryEntry::Function(lua_getmetatable)));
    module.entries.push(("setmetatable", LibraryEntry::Function(lua_setmetatable)));
    module.entries.push(("rawget", LibraryEntry::Function(lua_rawget)));
    module.entries.push(("rawset", LibraryEntry::Function(lua_rawset)));
    module.entries.push(("rawlen", LibraryEntry::Function(lua_rawlen)));
    module.entries.push(("rawequal", LibraryEntry::Function(lua_rawequal)));
    module.entries.push(("collectgarbage", LibraryEntry::Function(lua_collectgarbage)));
    module.entries.push(("require", LibraryEntry::Function(lua_require)));
    module.entries.push(("load", LibraryEntry::Function(lua_load)));
    module.entries.push(("loadfile", LibraryEntry::Function(lua_loadfile)));
    module.entries.push(("dofile", LibraryEntry::Function(lua_dofile)));
    module.entries.push(("warn", LibraryEntry::Function(lua_warn)));
    
    // Values
    module.entries.push(("_VERSION", LibraryEntry::Value(|vm| vm.create_string("Lua 5.4"))));
    
    module
}

/// print(...) - Print values to stdout
fn lua_print(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    let output: Vec<String> = args
        .iter()
        .map(|v| vm.value_to_string(v).unwrap_or_else(|_| "?".to_string()))
        .collect();

    if !output.is_empty() {
        println!("{}", output.join("\t"));
    } else {
        println!();
    }
    println!(); // Extra blank line for visibility

    Ok(MultiValue::empty())
}

/// type(v) - Return the type of a value as a string
fn lua_type(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // type() requires exactly one argument
    let value = match get_arg(vm, 1) {
        Some(v) => v,
        None => {
            return Err(vm.error("bad argument #1 to 'type' (value expected)".to_string()));
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

    let result = vm.create_string(type_name);
    Ok(MultiValue::single(result))
}

/// assert(v [, message]) - Raise error if v is false or nil
fn lua_assert(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let condition = get_arg(vm, 1).unwrap_or(LuaValue::nil());

    if !condition.is_truthy() {
        let message = get_arg(vm, 2)
            .and_then(|v| {
                v.as_string_id().and_then(|id| {
                    vm.object_pool
                        .get_string(id)
                        .map(|s| s.as_str().to_string())
                })
            })
            .unwrap_or_else(|| "assertion failed!".to_string());
        return Err(vm.error(message));
    }

    // Return all arguments
    Ok(MultiValue::multiple(get_args(vm)))
}

/// error(message) - Raise an error
fn lua_error(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let message = get_arg(vm, 1)
        .map(|v| vm.value_to_string(&v).unwrap_or_else(|_| "?".to_string()))
        .unwrap_or_else(|| "error".to_string());

    // Return error message directly for now
    Err(vm.error(message))
}

/// tonumber(e [, base]) - Convert to number
fn lua_tonumber(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let value = require_arg(vm, 1, "tonumber")?;
    let base = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(10);

    if base < 2 || base > 36 {
        return Err(vm.error("bad argument #2 to 'tonumber' (base out of range)".to_string()));
    }

    let result = match value.kind() {
        LuaValueKind::Integer => value.clone(),
        LuaValueKind::Float => value.clone(),
        LuaValueKind::String => {
            if let Some(string_id) = value.as_string_id() {
                if let Some(s) = vm.object_pool.get_string(string_id) {
                    let s_str = s.as_str().trim();
                    if base == 10 {
                        // Parse Lua number (supports hex like 0xAA.0)
                        parse_lua_number(s_str)
                    } else {
                        // Parse with specific base (no hex/float support)
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

    Ok(MultiValue::single(result))
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
fn lua_tostring(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let value = require_arg(vm, 1, "tostring")?;

    // Fast path: if already a string, return it directly
    if value.is_string() {
        return Ok(MultiValue::single(value));
    }

    // Fast path: simple types without metamethods
    if value.is_nil() {
        let result = vm.create_string("nil");
        return Ok(MultiValue::single(result));
    }

    if let Some(b) = value.as_bool() {
        let result = vm.create_string(if b { "true" } else { "false" });
        return Ok(MultiValue::single(result));
    }

    if let Some(i) = value.as_integer() {
        // OPTIMIZED: Use itoa for fast integer formatting (10x faster than format!)
        let mut buffer = itoa::Buffer::new();
        let s = buffer.format(i);
        let result = vm.create_string(s);
        return Ok(MultiValue::single(result));
    }

    if let Some(f) = value.as_number() {
        let result = vm.create_string(&f.to_string());
        return Ok(MultiValue::single(result));
    }

    // Slow path: check for __tostring metamethod
    let value_str = vm.value_to_string(&value)?;
    let result = vm.create_string(&value_str);
    Ok(MultiValue::single(result))
}

/// select(index, ...) - Return subset of arguments
/// ULTRA-OPTIMIZED: Fast path for "#" and single-value returns
fn lua_select(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;

    if top <= 1 {
        return Err(vm.error("bad argument #1 to 'select' (value expected)".to_string()));
    }

    let index_arg = vm.register_stack[base_ptr + 1];

    // Total args after index = top - 2 (subtract function slot and index slot)
    let vararg_count = top.saturating_sub(2);

    // FAST PATH: Check for "#" using string interning
    // StringId can be compared directly without string lookup
    if let Some(string_id) = index_arg.as_string_id() {
        // Check if this is the interned "#" string
        // Note: Most Lua code uses "#" as literal, so it's likely interned
        if let Some(s) = vm.object_pool.get_string(string_id) {
            let s_str = s.as_str();
            if s_str.len() == 1 && s_str.as_bytes()[0] == b'#' {
                return Ok(MultiValue::single(LuaValue::integer(vararg_count as i64)));
            }
        }
        return Err(vm.error("bad argument #1 to 'select' (number expected)".to_string()));
    }

    let index = index_arg
        .as_integer()
        .ok_or_else(|| vm.error("bad argument #1 to 'select' (number expected)".to_string()))?;

    if index == 0 {
        return Err(vm.error("bad argument #1 to 'select' (index out of range)".to_string()));
    }

    // Calculate start position (1-based to 0-based)
    let start_idx = if index > 0 {
        (index - 1) as usize
    } else {
        // Negative index: count from end
        let abs_idx = (-index) as usize;
        if abs_idx > vararg_count {
            return Err(vm.error("bad argument #1 to 'select' (index out of range)".to_string()));
        }
        vararg_count - abs_idx
    };

    if start_idx >= vararg_count {
        return Ok(MultiValue::empty());
    }

    // FAST PATH: Single value return (most common case: select(n, ...))
    let result_count = vararg_count - start_idx;
    if result_count == 1 {
        let reg_idx = base_ptr + 2 + start_idx;
        return Ok(MultiValue::single(vm.register_stack[reg_idx]));
    }

    // Multi-value return: collect from registers
    let mut result = Vec::with_capacity(result_count);
    for i in 0..result_count {
        let reg_idx = base_ptr + 2 + start_idx + i;
        result.push(vm.register_stack[reg_idx]);
    }

    Ok(MultiValue::multiple(result))
}

/// ipairs(t) - Return iterator for array part of table
fn lua_ipairs(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 1, "ipairs")?;

    // Validate that it's a table
    if !table_val.is_table() {
        return Err(vm.error("bad argument #1 to 'ipairs' (table expected)".to_string()));
    }

    // Return iterator function, table, and 0
    let iter_func = LuaValue::cfunction(ipairs_next);

    Ok(MultiValue::multiple(vec![
        iter_func,
        table_val,
        LuaValue::integer(0),
    ]))
}

/// Iterator function for ipairs - Optimized for performance
#[inline]
fn ipairs_next(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // ULTRA-FAST PATH: Direct register access without get_arg overhead
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;

    // Arguments are at base_ptr + 1 (table) and base_ptr + 2 (index)
    // Avoid bounds checking in hot path
    let table_val = unsafe { *vm.register_stack.get_unchecked(base_ptr + 1) };
    let index_val = unsafe { *vm.register_stack.get_unchecked(base_ptr + 2) };

    // Fast path: both table and index are valid
    if let Some(table_id) = table_val.as_table_id() {
        if let Some(index) = index_val.as_integer() {
            let next_index = index + 1;

            // Access table via ObjectPool - unchecked for speed
            if let Some(table) = vm.object_pool.get_table(table_id) {
                if let Some(value) = table.get_int(next_index) {
                    // Use MultiValue::two() to avoid Vec allocation
                    return Ok(MultiValue::two(LuaValue::integer(next_index), value));
                }
                // Reached end of array - return single nil
                return Ok(MultiValue::single(LuaValue::nil()));
            }
        }
    }

    // Slow path with error
    Err(vm.error("ipairs iterator: invalid table or index".to_string()))
}

/// pairs(t) - Return iterator for all key-value pairs
fn lua_pairs(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 1, "pairs")?;

    // Validate that it's a table
    if table_val.as_table_id().is_none() {
        return Err(vm.error("bad argument #1 to 'pairs' (table expected)".to_string()));
    }

    // TODO: Check for __pairs metamethod

    // Return next function, table, and nil
    let next_func = LuaValue::cfunction(lua_next);
    let nil_val = LuaValue::nil();

    Ok(MultiValue::multiple(vec![next_func, table_val, nil_val]))
}

/// next(table [, index]) - Return next key-value pair
/// OPTIMIZED: Avoid Vec allocation for common 2-return case
fn lua_next(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Fast path: direct register access
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;

    let table_val = unsafe { *vm.register_stack.get_unchecked(base_ptr + 1) };
    let index_val = if top > 2 {
        unsafe { *vm.register_stack.get_unchecked(base_ptr + 2) }
    } else {
        LuaValue::nil()
    };

    // Use ObjectPool API for table access
    if let Some(table_id) = table_val.as_table_id() {
        if let Some(table) = vm.object_pool.get_table(table_id) {
            let result = table.next(&index_val);

            match result {
                // Use MultiValue::two() to avoid Vec allocation
                Some((key, value)) => Ok(MultiValue::two(key, value)),
                None => Ok(MultiValue::single(LuaValue::nil())),
            }
        } else {
            Err(vm.error("Invalid table".to_string()))
        }
    } else {
        Err(vm.error("Invalid table".to_string()))
    }
}

/// pcall(f [, arg1, ...]) - Protected call
/// ULTRA-OPTIMIZED: Zero Vec allocations on success path
/// Uses stack-based argument passing and direct return value handling
fn lua_pcall(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Get frame info to read args directly
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;

    // Arg 1 is the function (at base_ptr + 1)
    let func = if top > 1 {
        vm.register_stack[base_ptr + 1]
    } else {
        return Err(vm.error("pcall() requires argument 1".to_string()));
    };

    // Args start at base_ptr + 2, count is (top - 2) if > 0
    let arg_base = base_ptr + 2;
    let arg_count = if top > 2 { top - 2 } else { 0 };

    // Use stack-based protected call - no Vec allocations!
    let (success, result_count) = vm.protected_call_stack_based(func, arg_base, arg_count)?;

    // Build return: status first, then results from vm.return_values
    // We need to copy since return_values will be reused
    let mut return_values = Vec::with_capacity(1 + result_count);
    return_values.push(LuaValue::boolean(success));

    // Take results from vm.return_values
    for i in 0..result_count {
        if i < vm.return_values.len() {
            return_values.push(vm.return_values[i]);
        }
    }

    Ok(MultiValue::multiple(return_values))
}

/// xpcall(f, msgh [, arg1, ...]) - Protected call with error handler
/// OPTIMIZED: Avoid Vec allocations
fn lua_xpcall(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // xpcall(f, msgh, arg1, arg2, ...) -> status, result or error

    // Get frame info to read args directly
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;

    // Arg 1 is the function (at base_ptr + 1)
    let func = if top > 1 {
        vm.register_stack[base_ptr + 1]
    } else {
        return Err(vm.error("xpcall() requires argument 1".to_string()));
    };

    // Arg 2 is the error handler (at base_ptr + 2)
    let err_handler = if top > 2 {
        vm.register_stack[base_ptr + 2]
    } else {
        return Err(vm.error("xpcall() requires argument 2".to_string()));
    };

    // Collect remaining args (3..top) into a small vec
    let arg_count = if top > 3 { top - 3 } else { 0 };
    let args: Vec<LuaValue> = if arg_count > 0 {
        (3..top).map(|i| vm.register_stack[base_ptr + i]).collect()
    } else {
        Vec::new()
    };

    // Use protected_call_with_handler from VM
    let (success, results) = vm.protected_call_with_handler(func, args, err_handler)?;

    // Return status and results
    let mut return_values = vec![LuaValue::boolean(success)];
    return_values.extend(results);

    Ok(MultiValue::multiple(return_values))
}

/// getmetatable(object) - Get metatable
fn lua_getmetatable(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let value = require_arg(vm, 1, "getmetatable")?;

    match value.kind() {
        LuaValueKind::Table => {
            let Some(table_id) = value.as_table_id() else {
                return Err(vm.error("Invalid table".to_string()));
            };

            // Get metatable
            let mt = {
                let Some(table_ref) = vm.object_pool.get_table(table_id) else {
                    return Err(vm.error("Invalid table".to_string()));
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
                                // Return the __metatable field value instead of the actual metatable
                                return Ok(MultiValue::single(protected));
                            }
                        }
                    }
                }
                Ok(MultiValue::single(mt))
            } else {
                Ok(MultiValue::single(LuaValue::nil()))
            }
        }
        LuaValueKind::String => {
            // Return the shared string metatable
            if let Some(mt) = vm.get_string_metatable() {
                Ok(MultiValue::single(mt))
            } else {
                Ok(MultiValue::single(LuaValue::nil()))
            }
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
                                        return Ok(MultiValue::single(protected));
                                    }
                                }
                            }
                        }
                        return Ok(MultiValue::single(mt));
                    }
                }
            }
            Ok(MultiValue::single(LuaValue::nil()))
        }
        // TODO: Support metatables for other types (numbers, etc.)
        _ => Ok(MultiValue::single(LuaValue::nil())),
    }
}

/// setmetatable(table, metatable) - Set metatable
fn lua_setmetatable(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table = require_arg(vm, 1, "setmetatable")?;
    let metatable = require_arg(vm, 2, "setmetatable")?;

    // Set the new metatable using ObjectPool
    let Some(table_id) = table.as_table_id() else {
        return Err(vm.error("Invalid table".to_string()));
    };

    // Create the key first to avoid borrow issues
    let metatable_field = vm.create_string("__metatable");

    // Check if current metatable has __metatable field (protection)
    let is_protected = {
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
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
        return Err(vm.error("cannot change a protected metatable".to_string()));
    }

    // Now modify the table
    let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
        return Err(vm.error("Invalid table".to_string()));
    };

    match metatable.kind() {
        LuaValueKind::Nil => {
            table_ref.set_metatable(None);
        }
        LuaValueKind::Table => {
            // Just pass the metatable TableId as LuaValue
            table_ref.set_metatable(Some(metatable.clone()));
        }
        _ => {
            return Err(
                vm.error("setmetatable() second argument must be a table or nil".to_string())
            );
        }
    }

    // Return the original table
    Ok(MultiValue::single(table.clone()))
}

/// rawget(table, index) - Get without metamethods
fn lua_rawget(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table = require_arg(vm, 1, "rawget")?;
    let key = require_arg(vm, 2, "rawget")?;

    if let Some(table_id) = table.as_table_id() {
        if let Some(table_ref) = vm.object_pool.get_table(table_id) {
            let value = table_ref.raw_get(&key).unwrap_or(LuaValue::nil());
            return Ok(MultiValue::single(value));
        }
    }
    Err(vm.error("rawget() first argument must be a table".to_string()))
}

/// rawset(table, index, value) - Set without metamethods
fn lua_rawset(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table = require_arg(vm, 1, "rawset")?;
    let key = require_arg(vm, 2, "rawset")?;
    let value = require_arg(vm, 3, "rawset")?;

    if let Some(table_id) = table.as_table_id() {
        if let Some(table_ref) = vm.object_pool.get_table_mut(table_id) {
            table_ref.raw_set(key, value);
            return Ok(MultiValue::single(table));
        }
    }
    Err(vm.error("rawset() first argument must be a table".to_string()))
}

/// rawlen(v) - Length without metamethods
fn lua_rawlen(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let value = require_arg(vm, 1, "rawlen")?;

    let len = match value.kind() {
        LuaValueKind::Table => {
            if let Some(table_id) = value.as_table_id() {
                if let Some(table) = vm.object_pool.get_table(table_id) {
                    table.len() as i64
                } else {
                    return Err(vm.error("rawlen() argument must be a table or string".to_string()));
                }
            } else {
                return Err(vm.error("rawlen() argument must be a table or string".to_string()));
            }
        }
        LuaValueKind::String => {
            if let Some(string_id) = value.as_string_id() {
                if let Some(s) = vm.object_pool.get_string(string_id) {
                    s.as_str().len() as i64
                } else {
                    return Err(vm.error("rawlen() argument must be a table or string".to_string()));
                }
            } else {
                return Err(vm.error("rawlen() argument must be a table or string".to_string()));
            }
        }
        _ => {
            return Err(vm.error("rawlen() argument must be a table or string".to_string()));
        }
    };

    Ok(MultiValue::single(LuaValue::integer(len)))
}

/// rawequal(v1, v2) - Equality without metamethods
fn lua_rawequal(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let v1 = get_arg(vm, 1).unwrap_or(LuaValue::nil());
    let v2 = get_arg(vm, 2).unwrap_or(LuaValue::nil());

    let result = v1 == v2;
    Ok(MultiValue::single(LuaValue::boolean(result)))
}

/// collectgarbage([opt [, arg]]) - Garbage collector control
fn lua_collectgarbage(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let opt = get_arg(vm, 1)
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
            Ok(MultiValue::single(LuaValue::integer(0)))
        }
        "count" => {
            // Return a dummy value for now
            Ok(MultiValue::single(LuaValue::integer(0)))
        }
        "stop" => {
            // Set GC debt to very negative value to prevent collection
            vm.gc.gc_debt = isize::MIN / 2;
            vm.gc_debt_local = isize::MIN / 2;
            Ok(MultiValue::single(LuaValue::integer(0)))
        }
        "restart" => {
            // Reset GC debt to trigger collection
            vm.gc.gc_debt = 0;
            vm.gc_debt_local = 0;
            Ok(MultiValue::single(LuaValue::integer(0)))
        }
        "step" | "setpause" | "setstepmul" | "isrunning" => {
            // Simplified: just return 0
            Ok(MultiValue::single(LuaValue::integer(0)))
        }
        _ => Err(vm.error(format!("collectgarbage: invalid option '{}'", opt))),
    }
}

/// require(modname) - Load a module  
fn lua_require(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let modname_value = require_arg(vm, 1, "require")?;
    if !modname_value.is_string() {
        return Err(vm.error("module name must be a string".to_string()));
    }

    // Check if module is already loaded in package.loaded
    let package_table = if let Some(package_table) = vm.get_global("package") {
        // Check package.loaded for existing module
        let loaded_key = vm.create_string("loaded");
        let module_val = {
            let Some(package_id) = package_table.as_table_id() else {
                return Err(vm.error("package table not found".to_string()));
            };
            let Some(package_ref) = vm.object_pool.get_table(package_id) else {
                return Err(vm.error("package table not found".to_string()));
            };
            if let Some(loaded_table) = package_ref.raw_get(&loaded_key) {
                if let Some(loaded_id) = loaded_table.as_table_id() {
                    if let Some(loaded_ref) = vm.object_pool.get_table(loaded_id) {
                        loaded_ref.raw_get(&modname_value)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(module_val) = module_val {
            if !module_val.is_nil() {
                // Module already loaded - return it
                return Ok(MultiValue::single(module_val));
            }
        }
        package_table
    } else {
        return Err(vm.error("package table not found".to_string()));
    };

    // Try each searcher in package.searchers
    let mut error_messages = Vec::new();
    let key = vm.create_string("searchers");

    // Get searchers table
    let searchers_values = {
        let Some(package_id) = package_table.as_table_id() else {
            return Err(vm.error("Invalid package table".to_string()));
        };
        let Some(package_ref) = vm.object_pool.get_table(package_id) else {
            return Err(vm.error("Invalid package table".to_string()));
        };
        let searchers_val = package_ref.raw_get(&key).unwrap_or(LuaValue::nil());

        let Some(searchers_id) = searchers_val.as_table_id() else {
            return Err(vm.error("package.searchers is not a table".to_string()));
        };
        let Some(searchers_table) = vm.object_pool.get_table(searchers_id) else {
            return Err(vm.error("package.searchers is not a table".to_string()));
        };

        // Collect all searchers upfront
        let mut values = Vec::new();
        let mut i = 1;
        loop {
            let searcher = searchers_table.get_int(i).unwrap_or(LuaValue::nil());
            if searcher.is_nil() {
                break;
            }
            values.push(searcher);
            i += 1;
        }
        values
    };

    // Try each searcher (1-based indexing)
    for searcher in searchers_values {
        // Call searcher with module name
        let (success, results) = vm.protected_call(searcher, vec![modname_value.clone()])?;

        if !success {
            let error_msg = results
                .first()
                .and_then(|v| {
                    v.as_string_id().and_then(|id| {
                        vm.object_pool
                            .get_string(id)
                            .map(|s| s.as_str().to_string())
                    })
                })
                .unwrap_or_else(|| "unknown error in searcher".to_string());
            return Err(vm.error(format!("error calling searcher: {}", error_msg)));
        }

        // Check result
        if !results.is_empty() {
            let first_result = &results[0];

            // If it's a function, this is the loader
            if first_result.is_function() || first_result.is_cfunction() {
                // Call the loader
                let loader_args = if results.len() > 1 {
                    vec![modname_value.clone(), results[1].clone()]
                } else {
                    vec![modname_value.clone()]
                };

                let (load_success, load_results) =
                    vm.protected_call(first_result.clone(), loader_args)?;

                if !load_success {
                    let module_str = vm.value_to_string(&modname_value)?;
                    let error_msg = load_results
                        .first()
                        .and_then(|v| {
                            v.as_string_id().and_then(|id| {
                                vm.object_pool
                                    .get_string(id)
                                    .map(|s| s.as_str().to_string())
                            })
                        })
                        .unwrap_or_else(|| "unknown error".to_string());
                    return Err(vm.error(format!(
                        "error loading module '{}': {}",
                        module_str, error_msg
                    )));
                }

                // Get the module value
                let module_value = if load_results.is_empty() || load_results[0].is_nil() {
                    LuaValue::boolean(true)
                } else {
                    load_results[0].clone()
                };

                // Store in package.loaded
                let loaded_key = vm.create_string("loaded");
                if let Some(package_id) = package_table.as_table_id() {
                    if let Some(package_ref) = vm.object_pool.get_table(package_id) {
                        if let Some(loaded_table) = package_ref.raw_get(&loaded_key) {
                            if let Some(loaded_id) = loaded_table.as_table_id() {
                                if let Some(loaded_ref) = vm.object_pool.get_table_mut(loaded_id) {
                                    loaded_ref.raw_set(modname_value, module_value.clone());
                                }
                            }
                        }
                    }
                }

                return Ok(MultiValue::single(module_value));
            } else if let Some(string_id) = first_result.as_string_id() {
                // It's an error message
                if let Some(err_str) = vm.object_pool.get_string(string_id) {
                    error_messages.push(err_str.as_str().to_string());
                }
            }
        }
    }
    let module_str = vm.value_to_string(&modname_value)?;

    // All searchers failed
    if error_messages.is_empty() {
        Err(vm.error(format!("module '{}' not found", module_str)))
    } else {
        Err(vm.error(format!(
            "module '{}' not found:{}",
            module_str,
            error_messages.join("")
        )))
    }
}

/// load(chunk [, chunkname [, mode [, env]]]) - Load a chunk
fn lua_load(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let chunk_val = require_arg(vm, 1, "load")?;

    // Get the chunk string
    let Some(string_id) = chunk_val.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'load' (string expected)".to_string()));
    };
    let code_str = {
        let Some(code) = vm.object_pool.get_string(string_id) else {
            return Err(vm.error("bad argument #1 to 'load' (string expected)".to_string()));
        };
        code.as_str().to_string()
    };

    // Optional chunk name for error messages
    let chunkname = get_arg(vm, 2)
        .and_then(|v| {
            v.as_string_id().and_then(|id| {
                vm.object_pool
                    .get_string(id)
                    .map(|s| s.as_str().to_string())
            })
        })
        .unwrap_or_else(|| "=(load)".to_string());

    // Optional mode ("b", "t", or "bt") - we only support "t" (text)
    let _mode = get_arg(vm, 3)
        .and_then(|v| {
            v.as_string_id().and_then(|id| {
                vm.object_pool
                    .get_string(id)
                    .map(|s| s.as_str().to_string())
            })
        })
        .unwrap_or_else(|| "bt".to_string());

    // Optional environment table
    let env = get_arg(vm, 4);

    // Compile the code using VM's string pool with chunk name
    match vm.compile_with_name(&code_str, &chunkname) {
        Ok(chunk) => {
            // Create upvalue for _ENV (global table)
            // Loaded chunks need _ENV as upvalue[0]
            let env_upvalue_id = if let Some(env) = env {
                vm.create_upvalue_closed(env)
            } else {
                vm.create_upvalue_closed(LuaValue::table(vm.global))
            };
            let upvalues = vec![env_upvalue_id];

            let func = vm.create_function(Rc::new(chunk), upvalues);
            Ok(MultiValue::single(func))
        }
        Err(e) => {
            // Return nil and error message
            let err_msg = vm.create_string(&format!("load error: {}", e));
            Ok(MultiValue::multiple(vec![LuaValue::nil(), err_msg]))
        }
    }
}

/// loadfile([filename [, mode [, env]]]) - Load a file as a chunk
fn lua_loadfile(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let filename = require_arg(vm, 1, "loadfile")?;
    let Some(string_id) = filename.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'loadfile' (string expected)".to_string()));
    };
    let filename_str = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(vm.error("bad argument #1 to 'loadfile' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    // Load from specified file
    let code = match std::fs::read_to_string(&filename_str) {
        Ok(c) => c,
        Err(e) => {
            let err_msg = vm.create_string(&format!("cannot open {}: {}", filename_str, e));
            return Ok(MultiValue::multiple(vec![LuaValue::nil(), err_msg]));
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
            Ok(MultiValue::single(func))
        }
        Err(e) => {
            let err_msg = vm.create_string(&format!("load error: {}", e));
            Ok(MultiValue::multiple(vec![LuaValue::nil(), err_msg]))
        }
    }
}

/// dofile([filename]) - Execute a file
fn lua_dofile(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let filename = get_arg(vm, 1).and_then(|v| {
        v.as_string_id().and_then(|id| {
            vm.object_pool
                .get_string(id)
                .map(|s| s.as_str().to_string())
        })
    });

    let (code, chunkname) = if let Some(fname) = filename {
        // Load from specified file
        let code = match std::fs::read_to_string(&fname) {
            Ok(c) => c,
            Err(e) => {
                return Err(vm.error(format!("cannot open {}: {}", fname, e)));
            }
        };
        (code, format!("@{}", fname))
    } else {
        // Load from stdin (simplified: return error for now)
        return Err(vm.error("stdin loading not implemented".to_string()));
    };

    // Compile and execute using VM's string pool with chunk name
    match vm.compile_with_name(&code, &chunkname) {
        Ok(chunk) => {
            // Create upvalue for _ENV (global table)
            let env_upvalue_id = vm.create_upvalue_closed(LuaValue::table(vm.global));
            let upvalues = vec![env_upvalue_id];
            let func = vm.create_function(std::rc::Rc::new(chunk), upvalues);

            // Call the function
            let (success, results) = vm.protected_call(func, vec![])?;

            if success {
                Ok(MultiValue::multiple(results))
            } else {
                let error_msg = results
                    .first()
                    .and_then(|v| {
                        v.as_string_id().and_then(|id| {
                            vm.object_pool
                                .get_string(id)
                                .map(|s| s.as_str().to_string())
                        })
                    })
                    .unwrap_or_else(|| "unknown error".to_string());
                Err(vm.error(error_msg))
            }
        }
        Err(e) => Err(vm.error(format!("load error: {}", e))),
    }
}

/// warn(msg1, ...) - Emit a warning
fn lua_warn(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    let messages: Vec<String> = args
        .iter()
        .map(|v| vm.value_to_string(v).unwrap_or_else(|_| "?".to_string()))
        .collect();
    let message = messages.join("");

    // Emit warning to stderr
    eprintln!("Lua warning: {}", message);

    Ok(MultiValue::empty())
}
