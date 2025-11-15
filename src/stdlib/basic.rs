// Basic library (_G global functions)
// Implements: print, type, assert, error, tonumber, tostring,
// select, ipairs, pairs, next, pcall, xpcall, getmetatable, setmetatable,
// rawget, rawset, rawlen, rawequal, collectgarbage, dofile, loadfile, load

use crate::LuaString;
use crate::lib_registry::{LibraryModule, get_arg, get_args, require_arg};
use crate::lua_value::{LuaValue, LuaValueKind, MultiValue};
use crate::lua_vm::LuaVM;

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
        "require" => lua_require,
    })
}

/// print(...) - Print values to stdout
fn lua_print(vm: &mut LuaVM) -> Result<MultiValue, String> {
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
fn lua_type(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let value = get_arg(vm, 0).unwrap_or(LuaValue::nil());

    let type_name = match value.kind() {
        LuaValueKind::Nil => "nil",
        LuaValueKind::Boolean => "boolean",
        LuaValueKind::Integer | LuaValueKind::Float => "number",
        LuaValueKind::String => "string",
        LuaValueKind::Table => "table",
        LuaValueKind::Function | LuaValueKind::CFunction => "function",
        LuaValueKind::Userdata => "userdata",
    };

    let result = vm.create_string(type_name.to_string());
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// assert(v [, message]) - Raise error if v is false or nil
fn lua_assert(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let condition = get_arg(vm, 0).unwrap_or(LuaValue::nil());

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
fn lua_error(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let message = get_arg(vm, 0)
        .map(|v| v.to_string_repr())
        .unwrap_or_else(|| "error".to_string());

    // Optional level parameter (default = 1)
    // level 1: error at the function that called error()
    // level 2: error at the function that called the function that called error()
    let _level = get_arg(vm, 1).and_then(|v| v.as_integer()).unwrap_or(1);

    // Return error message directly for now
    Err(message)
}

/// tonumber(e [, base]) - Convert to number
fn lua_tonumber(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let value = get_arg(vm, 0).unwrap_or(LuaValue::nil());
    let base = get_arg(vm, 1).and_then(|v| v.as_integer()).unwrap_or(10);

    if base < 2 || base > 36 {
        return Err(format!("bad argument #2 to 'tonumber' (base out of range)"));
    }

    let result = match value.kind() {
        LuaValueKind::Integer => value.clone(),
        LuaValueKind::Float => value.clone(),
        LuaValueKind::String => {
            if let Some(s) = value.as_string() {
                let s_str = s.as_str().trim();
                if base == 10 {
                    // Try integer first, then float
                    if let Ok(i) = s_str.parse::<i64>() {
                        LuaValue::integer(i)
                    } else if let Ok(f) = s_str.parse::<f64>() {
                        LuaValue::float(f)
                    } else {
                        LuaValue::nil()
                    }
                } else {
                    // Parse with specific base
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

    Ok(MultiValue::single(result))
}

/// tostring(v) - Convert to string
fn lua_tostring(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let value = get_arg(vm, 0).unwrap_or(LuaValue::nil());

    // Check for __tostring metamethod
    let value_str = vm.value_to_string(&value)?;
    let result = vm.create_string(value_str);
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// select(index, ...) - Return subset of arguments
fn lua_select(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let index_arg = require_arg(vm, 0, "select")?;
    let args = get_args(vm);

    // Handle "#" special case
    if let Some(s) = index_arg.as_string() {
        if s.as_str() == "#" {
            return Ok(MultiValue::single(LuaValue::integer(
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
fn lua_ipairs(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "ipairs")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'ipairs' (table expected)".to_string())?;

    // Return iterator function, table, and 0
    let iter_func = LuaValue::cfunction(ipairs_next);

    Ok(MultiValue::multiple(vec![
        iter_func,
        LuaValue::from_table_rc(table),
        LuaValue::integer(0),
    ]))
}

/// Iterator function for ipairs
fn ipairs_next(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "ipairs iterator")?
        .as_table()
        .ok_or_else(|| "ipairs iterator: table expected".to_string())?;

    let index = require_arg(vm, 1, "ipairs iterator")?
        .as_integer()
        .ok_or_else(|| "ipairs iterator: number expected".to_string())?;

    let next_index = index + 1;
    let key = LuaValue::integer(next_index);

    let value = table.borrow().raw_get(&key);

    if let Some(value) = value {
        if value.is_nil() {
            return Ok(MultiValue::single(LuaValue::nil()));
        }
        Ok(MultiValue::multiple(vec![
            LuaValue::integer(next_index),
            value,
        ]))
    } else {
        Ok(MultiValue::empty())
    }
}

/// pairs(t) - Return iterator for all key-value pairs
fn lua_pairs(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "pairs")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'pairs' (table expected)".to_string())?;

    // TODO: Check for __pairs metamethod

    // Return next function, table, and nil
    let next_func = LuaValue::cfunction(lua_next);
    let table_val = LuaValue::from_table_rc(table);
    let nil_val = LuaValue::nil();

    Ok(MultiValue::multiple(vec![
        next_func,
        table_val,
        nil_val,
    ]))
}

/// next(table [, index]) - Return next key-value pair
fn lua_next(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let table_val = require_arg(vm, 0, "next")?;
    
    let table = table_val.as_table()
        .ok_or_else(|| "bad argument #1 to 'next' (table expected)".to_string())?;
    
    let index_val = get_arg(vm, 1).unwrap_or(LuaValue::nil());
    
    // Use the table's built-in next() method which maintains proper iteration order
    let result = table.borrow().next(&index_val);
    
    match result {
        Some((key, value)) => {
            Ok(MultiValue::multiple(vec![key, value]))
        }
        None => {
            Ok(MultiValue::single(LuaValue::nil()))
        }
    }
}

/// pcall(f [, arg1, ...]) - Protected call
fn lua_pcall(vm: &mut LuaVM) -> Result<MultiValue, String> {
    // pcall(f, arg1, arg2, ...) -> status, result or error

    // Get the function to call (argument 0)
    let func = require_arg(vm, 0, "pcall")?;

    // Get all arguments after the function
    let all_args = get_args(vm);
    let args: Vec<LuaValue> = if all_args.len() > 1 {
        all_args[1..].to_vec()
    } else {
        Vec::new()
    };

    // Use protected_call from VM
    let (success, results) = vm.protected_call(func, args);

    // Return status and results
    let mut return_values = vec![LuaValue::boolean(success)];
    return_values.extend(results);

    Ok(MultiValue::multiple(return_values))
}

/// xpcall(f, msgh [, arg1, ...]) - Protected call with error handler
fn lua_xpcall(vm: &mut LuaVM) -> Result<MultiValue, String> {
    // xpcall(f, msgh, arg1, arg2, ...) -> status, result or error
    
    // Get the function to call (argument 0)
    let func = require_arg(vm, 0, "xpcall")?;
    
    // Get the error handler (argument 1)
    let err_handler = require_arg(vm, 1, "xpcall")?;
    
    // Get all arguments after the function and error handler
    let all_args = get_args(vm);
    let args: Vec<LuaValue> = if all_args.len() > 2 {
        all_args[2..].to_vec()
    } else {
        Vec::new()
    };
    
    // Use protected_call_with_handler from VM
    let (success, results) = vm.protected_call_with_handler(func, args, err_handler);
    
    // Return status and results
    let mut return_values = vec![LuaValue::boolean(success)];
    return_values.extend(results);
    
    Ok(MultiValue::multiple(return_values))
}

/// getmetatable(object) - Get metatable
fn lua_getmetatable(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 1 {
        return Ok(MultiValue::single(LuaValue::nil()));
    }

    let value = &registers[1];

    match value.kind() {
        LuaValueKind::Table => {
            if let Some(t) = value.as_table() {
                if let Some(mt) = t.borrow().get_metatable() {
                    Ok(MultiValue::single(LuaValue::from_table_rc(mt)))
                } else {
                    Ok(MultiValue::single(LuaValue::nil()))
                }
            } else {
                Ok(MultiValue::single(LuaValue::nil()))
            }
        }
        // TODO: Support metatables for other types (userdata, strings, etc.)
        _ => Ok(MultiValue::single(LuaValue::nil())),
    }
}

/// setmetatable(table, metatable) - Set metatable
fn lua_setmetatable(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 2 {
        return Err("setmetatable() requires 2 arguments".to_string());
    }

    let table = &registers[1];
    let metatable = &registers[2];

    // First argument must be a table
    if let Some(t) = table.as_table() {
        // Check if current metatable has __metatable field (protected)
        if let Some(mt) = t.borrow().get_metatable() {
            let metatable_key = LuaValue::from_string_rc(std::rc::Rc::new(LuaString::new(
                "__metatable".to_string(),
            )));
            if mt.borrow().raw_get(&metatable_key).is_some() {
                return Err("cannot change a protected metatable".to_string());
            }
        }

        // Set the new metatable
        match metatable.kind() {
            LuaValueKind::Nil => {
                t.borrow_mut().set_metatable(None);
            }
            LuaValueKind::Table => {
                if let Some(mt) = metatable.as_table() {
                    t.borrow_mut().set_metatable(Some(mt.clone()));
                }
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
fn lua_rawget(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 2 {
        return Err("rawget() requires 2 arguments".to_string());
    }

    let table = &registers[1];
    let key = &registers[2];

    if let Some(t) = table.as_table() {
        let value = t.borrow().raw_get(key).unwrap_or(LuaValue::nil());
        Ok(MultiValue::single(value))
    } else {
        Err("rawget() first argument must be a table".to_string())
    }
}

/// rawset(table, index, value) - Set without metamethods
fn lua_rawset(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 3 {
        return Err("rawset() requires 3 arguments".to_string());
    }

    let table = &registers[1];
    let key = &registers[2];
    let value = &registers[3];

    if let Some(t) = table.as_table() {
        if key.is_nil() {
            return Err("table index is nil".to_string());
        }

        t.borrow_mut().raw_set(key.clone(), value.clone());
        Ok(MultiValue::single(table.clone()))
    } else {
        Err("rawset() first argument must be a table".to_string())
    }
}

/// rawlen(v) - Length without metamethods
fn lua_rawlen(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;

    if registers.len() <= 1 {
        return Err("rawlen() requires 1 argument".to_string());
    }

    let value = &registers[1];

    let len = match value.kind() {
        LuaValueKind::Table => {
            if let Some(t) = value.as_table() {
                t.borrow().len() as i64
            } else {
                return Err("rawlen() argument must be a table or string".to_string());
            }
        }
        LuaValueKind::String => {
            if let Some(s) = value.as_string() {
                s.as_str().len() as i64
            } else {
                return Err("rawlen() argument must be a table or string".to_string());
            }
        }
        _ => {
            return Err("rawlen() argument must be a table or string".to_string());
        }
    };

    Ok(MultiValue::single(LuaValue::integer(len)))
}

/// rawequal(v1, v2) - Equality without metamethods
fn lua_rawequal(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let v1 = get_arg(vm, 0).unwrap_or(LuaValue::nil());
    let v2 = get_arg(vm, 1).unwrap_or(LuaValue::nil());

    let result = vm.values_equal(&v1, &v2);
    Ok(MultiValue::single(LuaValue::boolean(result)))
}

/// collectgarbage([opt [, arg]]) - Garbage collector control
fn lua_collectgarbage(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let opt = get_arg(vm, 0)
        .and_then(|v| v.as_string())
        .map(|s| s.as_str().to_string())
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
        "stop" | "restart" | "step" | "setpause" | "setstepmul" | "isrunning" => {
            // Simplified: just return 0
            Ok(MultiValue::single(LuaValue::integer(0)))
        }
        _ => Err(format!("collectgarbage: invalid option '{}'", opt)),
    }
}

/// _VERSION - Lua version string
fn lua_version(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let version = vm.create_string("Lua 5.4".to_string());
    Ok(MultiValue::single(LuaValue::from_string_rc(version)))
}

/// require(modname) - Load a module  
fn lua_require(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let modname = get_arg(vm, 0)
        .and_then(|v| v.as_string())
        .ok_or("require: module name must be a string")?;
    
    let modname_str = modname.as_str();
    
    // Check if module is already loaded in package.loaded
    if let Some(package_table) = vm.get_global("package") {
        if let Some(package_rc) = package_table.as_table() {
            let loaded_key = vm.create_string("loaded".to_string());
            if let Some(loaded_table) = package_rc.borrow().raw_get(&LuaValue::from_string_rc(loaded_key)) {
                if let Some(loaded_rc) = loaded_table.as_table() {
                    let mod_key = vm.create_string(modname_str.to_string());
                    if let Some(module_value) = loaded_rc.borrow().raw_get(&LuaValue::from_string_rc(mod_key)) {
                        if !module_value.is_nil() {
                            return Ok(MultiValue::single(module_value));
                        }
                    }
                }
            }
        }
    }
    
    // Try to load the module from file
    let possible_paths = vec![
        format!("{}.lua", modname_str),
        format!("{}/init.lua", modname_str),
        format!("./{}.lua", modname_str),
        format!("./{}/init.lua", modname_str),
    ];
    
    for path in possible_paths {
        if let Ok(code) = std::fs::read_to_string(&path) {
            // Compile the module
            match crate::Compiler::compile(&code) {
                Ok(chunk) => {
                    // Create a closure from the chunk
                    let func = LuaValue::from_function_rc(std::rc::Rc::new(crate::LuaFunction {
                        chunk: std::rc::Rc::new(chunk),
                        upvalues: vec![],
                    }));
                    
                    // Call the module loader using protected_call
                    let (success, results) = vm.protected_call(func, vec![]);
                    
                    if !success {
                        let error_msg = results.first()
                            .and_then(|v| v.as_string())
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_else(|| "unknown error".to_string());
                        return Err(format!("error loading module '{}': {}", modname_str, error_msg));
                    }
                    
                    // Get the result value
                    let module_value = if results.is_empty() || results[0].is_nil() {
                        LuaValue::boolean(true)
                    } else {
                        results[0].clone()
                    };
                    
                    // Store the result in package.loaded
                    if let Some(package_table) = vm.get_global("package") {
                        if let Some(package_rc) = package_table.as_table() {
                            let loaded_key = vm.create_string("loaded".to_string());
                            if let Some(loaded_table) = package_rc.borrow().raw_get(&LuaValue::from_string_rc(loaded_key)) {
                                if let Some(loaded_rc) = loaded_table.as_table() {
                                    let mod_key = vm.create_string(modname_str.to_string());
                                    loaded_rc.borrow_mut().raw_set(
                                        LuaValue::from_string_rc(mod_key),
                                        module_value.clone()
                                    );
                                }
                            }
                        }
                    }
                    
                    return Ok(MultiValue::single(module_value));
                }
                Err(e) => return Err(format!("error compiling module '{}': {}", modname_str, e)),
            }
        }
    }
    
    Err(format!("module '{}' not found", modname_str))
}
