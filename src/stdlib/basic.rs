// Basic library (_G global functions)
// Implements: print, type, assert, error, tonumber, tostring,
// select, ipairs, pairs, next, pcall, xpcall, getmetatable, setmetatable,
// rawget, rawset, rawlen, rawequal, collectgarbage, dofile, loadfile, load

use crate::lib_registry::{LibraryModule, get_arg, get_args, require_arg};
use crate::value::{LuaValue, MultiValue};
use crate::vm::VM;

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
        "_VERSION" => lua_version,
    })
}

/// print(...) - Print values to stdout
fn lua_print(vm: &mut VM) -> Result<MultiValue, String> {
    let args = get_args(vm);

    let output: Vec<String> = args.iter().map(|v| v.to_string_repr()).collect();

    if !output.is_empty() {
        println!("{}", output.join("\t"));
    } else {
        println!();
    }

    Ok(MultiValue::empty())
}

/// type(v) - Return the type of a value as a string
fn lua_type(vm: &mut VM) -> Result<MultiValue, String> {
    let value = get_arg(vm, 0).unwrap_or(LuaValue::Nil);

    let type_name = match value {
        LuaValue::Nil => "nil",
        LuaValue::Boolean(_) => "boolean",
        LuaValue::Integer(_) | LuaValue::Float(_) => "number",
        LuaValue::String(_) => "string",
        LuaValue::Table(_) => "table",
        LuaValue::Function(_) | LuaValue::CFunction(_) => "function",
        LuaValue::Userdata(_) => "userdata",
    };

    let result = vm.create_string(type_name.to_string());
    Ok(MultiValue::single(LuaValue::String(result)))
}

/// assert(v [, message]) - Raise error if v is false or nil
fn lua_assert(vm: &mut VM) -> Result<MultiValue, String> {
    let condition = get_arg(vm, 0).unwrap_or(LuaValue::Nil);

    if !condition.is_truthy() {
        let message = get_arg(vm, 1)
            .and_then(|v| v.as_string())
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "assertion failed!".to_string());
        return Err(message);
    }

    // Return all arguments
    Ok(MultiValue::multiple(get_args(vm)))
}

/// error(message [, level]) - Raise an error
fn lua_error(vm: &mut VM) -> Result<MultiValue, String> {
    let message = get_arg(vm, 0)
        .map(|v| v.to_string_repr())
        .unwrap_or_else(|| "error".to_string());

    Err(message)
}

/// tonumber(e [, base]) - Convert to number
fn lua_tonumber(vm: &mut VM) -> Result<MultiValue, String> {
    let value = get_arg(vm, 0).unwrap_or(LuaValue::Nil);
    let base = get_arg(vm, 1).and_then(|v| v.as_integer()).unwrap_or(10);

    if base < 2 || base > 36 {
        return Err(format!("bad argument #2 to 'tonumber' (base out of range)"));
    }

    let result = match value {
        LuaValue::Integer(i) => LuaValue::Integer(i),
        LuaValue::Float(f) => LuaValue::Float(f),
        LuaValue::String(s) => {
            let s = s.as_str().trim();
            if base == 10 {
                // Try integer first, then float
                if let Ok(i) = s.parse::<i64>() {
                    LuaValue::Integer(i)
                } else if let Ok(f) = s.parse::<f64>() {
                    LuaValue::Float(f)
                } else {
                    LuaValue::Nil
                }
            } else {
                // Parse with specific base
                if let Ok(i) = i64::from_str_radix(s, base as u32) {
                    LuaValue::Integer(i)
                } else {
                    LuaValue::Nil
                }
            }
        }
        _ => LuaValue::Nil,
    };

    Ok(MultiValue::single(result))
}

/// tostring(v) - Convert to string
fn lua_tostring(vm: &mut VM) -> Result<MultiValue, String> {
    let value = get_arg(vm, 0).unwrap_or(LuaValue::Nil);

    // Check for __tostring metamethod
    let value_str = vm.value_to_string(&value)?;
    let result = vm.create_string(value_str);
    Ok(MultiValue::single(LuaValue::String(result)))
}

/// select(index, ...) - Return subset of arguments
fn lua_select(vm: &mut VM) -> Result<MultiValue, String> {
    let index_arg = require_arg(vm, 0, "select")?;
    let args = get_args(vm);

    // Handle "#" special case
    if let Some(s) = index_arg.as_string() {
        if s.as_str() == "#" {
            return Ok(MultiValue::single(LuaValue::Integer(
                (args.len() - 1) as i64,
            )));
        }
    }

    let index = index_arg
        .as_integer()
        .ok_or_else(|| "bad argument #1 to 'select' (number expected)".to_string())?;

    if index == 0 {
        return Err("bad argument #1 to 'select' (index out of range)".to_string());
    }

    let start = if index > 0 {
        (index - 1) as usize
    } else {
        (args.len() as i64 + index) as usize
    };

    if start >= args.len() - 1 {
        return Ok(MultiValue::empty());
    }

    // Return args from start+1 onwards (skip the index argument itself)
    let result: Vec<LuaValue> = args.iter().skip(start + 1).cloned().collect();
    Ok(MultiValue::multiple(result))
}

/// ipairs(t) - Return iterator for array part of table
fn lua_ipairs(vm: &mut VM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "ipairs")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'ipairs' (table expected)".to_string())?;

    // Return iterator function, table, and 0
    let iter_func = LuaValue::CFunction(ipairs_next);

    Ok(MultiValue::multiple(vec![
        iter_func,
        LuaValue::Table(table),
        LuaValue::Integer(0),
    ]))
}

/// Iterator function for ipairs
fn ipairs_next(vm: &mut VM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "ipairs iterator")?
        .as_table()
        .ok_or_else(|| "ipairs iterator: table expected".to_string())?;

    let index = require_arg(vm, 1, "ipairs iterator")?
        .as_integer()
        .ok_or_else(|| "ipairs iterator: number expected".to_string())?;

    let next_index = index + 1;
    let key = LuaValue::Integer(next_index);

    let value = table.borrow().raw_get(&key);

    if let Some(value) = value {
        if value.is_nil() {
            return Ok(MultiValue::single(LuaValue::Nil));
        }
        Ok(MultiValue::multiple(vec![
            LuaValue::Integer(next_index),
            value,
        ]))
    } else {
        Ok(MultiValue::empty())
    }
}

/// pairs(t) - Return iterator for all key-value pairs
fn lua_pairs(vm: &mut VM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "pairs")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'pairs' (table expected)".to_string())?;

    // TODO: Check for __pairs metamethod

    // Return next function, table, and nil
    let next_func = LuaValue::CFunction(lua_next);

    Ok(MultiValue::multiple(vec![
        next_func,
        LuaValue::Table(table),
        LuaValue::Nil,
    ]))
}

/// next(table [, index]) - Return next key-value pair
fn lua_next(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() < 2 {
        return Err("next requires at least 1 argument".to_string());
    }

    let table_val = &registers[1];
    let Some(table) = table_val.as_table() else {
        return Err("next expects a table as first argument".to_string());
    };

    let index_val = if registers.len() >= 3 {
        &registers[2]
    } else {
        &LuaValue::Nil
    };

    let table_ref = table.borrow();

    // Get all key-value pairs
    let pairs: Vec<_> = table_ref.iter_all().collect();

    if pairs.is_empty() {
        // Empty table
        return Ok(MultiValue::single(LuaValue::Nil));
    }

    // If index is nil, return first key-value pair
    if index_val.is_nil() {
        let (key, value) = &pairs[0];
        return Ok(MultiValue::multiple(vec![key.clone(), value.clone()]));
    }

    // Find current key position and return next
    for (i, (key, _value)) in pairs.iter().enumerate() {
        if key == index_val {
            if i + 1 < pairs.len() {
                let (next_key, next_value) = &pairs[i + 1];
                return Ok(MultiValue::multiple(vec![
                    next_key.clone(),
                    next_value.clone(),
                ]));
            } else {
                // No more keys
                return Ok(MultiValue::single(LuaValue::Nil));
            }
        }
    }

    Err("invalid key to 'next'".to_string())
}

/// pcall(f [, arg1, ...]) - Protected call
fn lua_pcall(vm: &mut VM) -> Result<MultiValue, String> {
    // TODO: Implement proper protected call
    // For now, just return success = false
    let msg = vm.create_string("pcall not yet implemented".to_string());
    Ok(MultiValue::multiple(vec![
        LuaValue::Boolean(false),
        LuaValue::String(msg),
    ]))
}

/// xpcall(f, msgh [, arg1, ...]) - Protected call with error handler
fn lua_xpcall(vm: &mut VM) -> Result<MultiValue, String> {
    // TODO: Implement proper protected call with error handler
    let msg = vm.create_string("xpcall not yet implemented".to_string());
    Ok(MultiValue::multiple(vec![
        LuaValue::Boolean(false),
        LuaValue::String(msg),
    ]))
}

/// getmetatable(object) - Get metatable
fn lua_getmetatable(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 1 {
        return Ok(MultiValue::single(LuaValue::Nil));
    }

    let value = &registers[1];

    match value {
        LuaValue::Table(t) => {
            if let Some(mt) = t.borrow().get_metatable() {
                Ok(MultiValue::single(LuaValue::Table(mt)))
            } else {
                Ok(MultiValue::single(LuaValue::Nil))
            }
        }
        // TODO: Support metatables for other types (userdata, strings, etc.)
        _ => Ok(MultiValue::single(LuaValue::Nil)),
    }
}

/// setmetatable(table, metatable) - Set metatable
fn lua_setmetatable(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 2 {
        return Err("setmetatable() requires 2 arguments".to_string());
    }

    let table = &registers[1];
    let metatable = &registers[2];

    // First argument must be a table
    if let LuaValue::Table(t) = table {
        // Check if current metatable has __metatable field (protected)
        if let Some(mt) = t.borrow().get_metatable() {
            let metatable_key = LuaValue::String(std::rc::Rc::new(crate::value::LuaString::new(
                "__metatable".to_string(),
            )));
            if mt.borrow().raw_get(&metatable_key).is_some() {
                return Err("cannot change a protected metatable".to_string());
            }
        }

        // Set the new metatable
        match metatable {
            LuaValue::Nil => {
                t.borrow_mut().set_metatable(None);
            }
            LuaValue::Table(mt) => {
                t.borrow_mut().set_metatable(Some(mt.clone()));
            }
            _ => {
                return Err("setmetatable() second argument must be a table or nil".to_string());
            }
        }

        // Return the original table
        Ok(MultiValue::single(table.clone()))
    } else {
        Err("setmetatable() first argument must be a table".to_string())
    }
}

/// rawget(table, index) - Get without metamethods
fn lua_rawget(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 2 {
        return Err("rawget() requires 2 arguments".to_string());
    }

    let table = &registers[1];
    let key = &registers[2];

    if let LuaValue::Table(t) = table {
        let value = t.borrow().raw_get(key).unwrap_or(LuaValue::Nil);
        Ok(MultiValue::single(value))
    } else {
        Err("rawget() first argument must be a table".to_string())
    }
}

/// rawset(table, index, value) - Set without metamethods
fn lua_rawset(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 3 {
        return Err("rawset() requires 3 arguments".to_string());
    }

    let table = &registers[1];
    let key = &registers[2];
    let value = &registers[3];

    if let LuaValue::Table(t) = table {
        if matches!(key, LuaValue::Nil) {
            return Err("table index is nil".to_string());
        }

        t.borrow_mut().raw_set(key.clone(), value.clone());
        Ok(MultiValue::single(table.clone()))
    } else {
        Err("rawset() first argument must be a table".to_string())
    }
}

/// rawlen(v) - Length without metamethods
fn lua_rawlen(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 1 {
        return Err("rawlen() requires 1 argument".to_string());
    }

    let value = &registers[1];

    let len = match value {
        LuaValue::Table(t) => t.borrow().len() as i64,
        LuaValue::String(s) => s.as_str().len() as i64,
        _ => {
            return Err("rawlen() argument must be a table or string".to_string());
        }
    };

    Ok(MultiValue::single(LuaValue::Integer(len)))
}

/// rawequal(v1, v2) - Equality without metamethods
fn lua_rawequal(vm: &mut VM) -> Result<MultiValue, String> {
    let v1 = get_arg(vm, 0).unwrap_or(LuaValue::Nil);
    let v2 = get_arg(vm, 1).unwrap_or(LuaValue::Nil);

    let result = vm.values_equal(&v1, &v2);
    Ok(MultiValue::single(LuaValue::Boolean(result)))
}

/// collectgarbage([opt [, arg]]) - Garbage collector control
fn lua_collectgarbage(vm: &mut VM) -> Result<MultiValue, String> {
    let opt = get_arg(vm, 0)
        .and_then(|v| v.as_string())
        .map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "collect".to_string());

    match opt.as_str() {
        "collect" => {
            vm.collect_garbage();
            Ok(MultiValue::single(LuaValue::Integer(0)))
        }
        "count" => {
            // Return a dummy value for now
            Ok(MultiValue::single(LuaValue::Integer(0)))
        }
        "stop" | "restart" | "step" | "setpause" | "setstepmul" | "isrunning" => {
            // Simplified: just return 0
            Ok(MultiValue::single(LuaValue::Integer(0)))
        }
        _ => Err(format!("collectgarbage: invalid option '{}'", opt)),
    }
}

/// _VERSION - Lua version string
fn lua_version(vm: &mut VM) -> Result<MultiValue, String> {
    let version = vm.create_string("Lua 5.4".to_string());
    Ok(MultiValue::single(LuaValue::String(version)))
}
