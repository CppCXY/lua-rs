// Basic library (_G global functions)
// Implements: print, type, assert, error, tonumber, tostring,
// select, ipairs, pairs, next, pcall, xpcall, getmetatable, setmetatable,
// rawget, rawset, rawlen, rawequal, collectgarbage, dofile, loadfile, load

use crate::lib_registry::{LibraryModule, get_arg, get_args, require_arg};
use crate::lua_value::{LuaValue, LuaValueKind, MultiValue};
use crate::lua_vm::LuaVM;
use crate::{LuaString, LuaTable};
use std::cell::RefCell;
use std::rc::Rc;

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
        "load" => lua_load,
        "loadfile" => lua_loadfile,
        "dofile" => lua_dofile,
        "warn" => lua_warn,
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
        LuaValueKind::Thread => "thread",
    };

    let result = vm.create_string(type_name.to_string());
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// assert(v [, message]) - Raise error if v is false or nil
fn lua_assert(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let condition = get_arg(vm, 0).unwrap_or(LuaValue::nil());

    if !condition.is_truthy() {
        let message = get_arg(vm, 1)
            .and_then(|v| v.as_string_rc())
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
            unsafe {
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
    unsafe {
        if let Some(s) = index_arg.as_string() {
            if s.as_str() == "#" {
                return Ok(MultiValue::single(LuaValue::integer(
                    (args.len() - 1) as i64,
                )));
            }
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
        .as_table_rc()
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
        .as_table_rc()
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
    let table_val = require_arg(vm, 0, "pairs")?;
    let table = unsafe {
        table_val
            .as_table()
            .ok_or_else(|| "bad argument #1 to 'pairs' (table expected)".to_string())?
    };

    // TODO: Check for __pairs metamethod

    // Return next function, table, and nil
    let next_func = LuaValue::cfunction(lua_next);
    let table_val = unsafe {
        let rc = Rc::from_raw(table as *const RefCell<LuaTable>);
        let clone = rc.clone();
        std::mem::forget(rc);
        LuaValue::from_table_rc(clone)
    };
    let nil_val = LuaValue::nil();

    Ok(MultiValue::multiple(vec![next_func, table_val, nil_val]))
}

/// next(table [, index]) - Return next key-value pair
fn lua_next(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let table_val = require_arg(vm, 0, "next")?;

    let table = table_val
        .as_table_rc()
        .ok_or_else(|| "bad argument #1 to 'next' (table expected)".to_string())?;

    let index_val = get_arg(vm, 1).unwrap_or(LuaValue::nil());

    // Use the table's built-in next() method which maintains proper iteration order
    let result = table.borrow().next(&index_val);

    match result {
        Some((key, value)) => Ok(MultiValue::multiple(vec![key, value])),
        None => Ok(MultiValue::single(LuaValue::nil())),
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
    use crate::lib_registry::get_arg;

    let value = get_arg(vm, 0).ok_or("getmetatable() requires 1 argument")?;

    match value.kind() {
        LuaValueKind::Table => unsafe {
            if let Some(t) = value.as_table() {
                if let Some(mt) = t.borrow().get_metatable() {
                    Ok(MultiValue::single(LuaValue::from_table_rc(mt)))
                } else {
                    Ok(MultiValue::single(LuaValue::nil()))
                }
            } else {
                Ok(MultiValue::single(LuaValue::nil()))
            }
        },
        // TODO: Support metatables for other types (userdata, strings, etc.)
        _ => Ok(MultiValue::single(LuaValue::nil())),
    }
}

/// setmetatable(table, metatable) - Set metatable
fn lua_setmetatable(vm: &mut LuaVM) -> Result<MultiValue, String> {
    use crate::lib_registry::get_arg;

    let table = get_arg(vm, 0).ok_or("setmetatable() requires 2 arguments")?;
    let metatable = get_arg(vm, 1).ok_or("setmetatable() requires 2 arguments")?;

    // First argument must be a table
    unsafe {
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
                        let mt_rc = Rc::from_raw(mt as *const RefCell<LuaTable>);
                        let mt_clone = mt_rc.clone();
                        std::mem::forget(mt_rc);
                        t.borrow_mut().set_metatable(Some(mt_clone));
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
}

/// rawget(table, index) - Get without metamethods
fn lua_rawget(vm: &mut LuaVM) -> Result<MultiValue, String> {
    use crate::lib_registry::get_arg;

    let table = get_arg(vm, 0).ok_or("rawget() requires 2 arguments")?;
    let key = get_arg(vm, 1).ok_or("rawget() requires 2 arguments")?;

    unsafe {
        if let Some(t) = table.as_table() {
            let value = t.borrow().raw_get(&key).unwrap_or(LuaValue::nil());
            Ok(MultiValue::single(value))
        } else {
            Err("rawget() first argument must be a table".to_string())
        }
    }
}

/// rawset(table, index, value) - Set without metamethods
fn lua_rawset(vm: &mut LuaVM) -> Result<MultiValue, String> {
    use crate::lib_registry::get_arg;

    let table = get_arg(vm, 0).ok_or("rawset() requires 3 arguments")?;
    let key = get_arg(vm, 1).ok_or("rawset() requires 3 arguments")?;
    let value = get_arg(vm, 2).ok_or("rawset() requires 3 arguments")?;

    unsafe {
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
}

/// rawlen(v) - Length without metamethods
fn lua_rawlen(vm: &mut LuaVM) -> Result<MultiValue, String> {
    use crate::lib_registry::get_arg;

    let value = get_arg(vm, 0).ok_or("rawlen() requires 1 argument")?;

    let len = unsafe {
        match value.kind() {
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
        .and_then(|v| v.as_string_rc())
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
        .and_then(|v| v.as_string_rc())
        .ok_or("require: module name must be a string")?;

    let modname_str = modname.as_str();

    // Check if module is already loaded in package.loaded
    unsafe {
        if let Some(package_table) = vm.get_global("package") {
            if let Some(package_rc) = package_table.as_table() {
                let loaded_key = vm.create_string("loaded".to_string());
                if let Some(loaded_table) = package_rc
                    .borrow()
                    .raw_get(&LuaValue::from_string_rc(loaded_key))
                {
                    if let Some(loaded_rc) = loaded_table.as_table() {
                        let mod_key = vm.create_string(modname_str.to_string());
                        if let Some(module_value) = loaded_rc
                            .borrow()
                            .raw_get(&LuaValue::from_string_rc(mod_key))
                        {
                            if !module_value.is_nil() {
                                return Ok(MultiValue::single(module_value));
                            }
                        }
                    }
                }
            }
        }
    }

    // Try each searcher in package.searchers
    let mut error_messages = Vec::new();

    unsafe {
        let package_table = vm
            .get_global("package")
            .ok_or_else(|| "package table not found".to_string())?;

        let package_rc = package_table
            .as_table()
            .ok_or_else(|| "package is not a table".to_string())?;

        let searchers_val = package_rc
            .borrow()
            .raw_get(&LuaValue::from_string_rc(
                vm.create_string("searchers".to_string()),
            ))
            .unwrap_or(LuaValue::nil());

        let searchers_table = searchers_val
            .as_table()
            .ok_or_else(|| "package.searchers is not a table".to_string())?;

        // Try each searcher (1-based indexing)
        let mut i = 1;
        loop {
            let searcher_key = LuaValue::integer(i);
            let searcher = searchers_table
                .borrow()
                .raw_get(&searcher_key)
                .unwrap_or(LuaValue::nil());

            if searcher.is_nil() {
                break; // No more searchers
            }

            // Call searcher with module name
            let modname_val = LuaValue::from_string_rc(vm.create_string(modname_str.to_string()));
            let (success, results) = vm.protected_call(searcher.clone(), vec![modname_val]);

            if !success {
                let error_msg = results
                    .first()
                    .and_then(|v| v.as_string())
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| "unknown error in searcher".to_string());
                return Err(format!("error calling searcher: {}", error_msg));
            }

            // Check result
            if !results.is_empty() {
                let first_result = &results[0];

                // If it's a function, this is the loader
                if first_result.is_function() || first_result.is_cfunction() {
                    // Call the loader
                    let modname_arg =
                        LuaValue::from_string_rc(vm.create_string(modname_str.to_string()));
                    let loader_args = if results.len() > 1 {
                        vec![modname_arg, results[1].clone()]
                    } else {
                        vec![modname_arg]
                    };

                    let (load_success, load_results) =
                        vm.protected_call(first_result.clone(), loader_args);

                    if !load_success {
                        let error_msg = load_results
                            .first()
                            .and_then(|v| v.as_string())
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_else(|| "unknown error".to_string());
                        return Err(format!(
                            "error loading module '{}': {}",
                            modname_str, error_msg
                        ));
                    }

                    // Get the module value
                    let module_value = if load_results.is_empty() || load_results[0].is_nil() {
                        LuaValue::boolean(true)
                    } else {
                        load_results[0].clone()
                    };

                    // Store in package.loaded
                    let loaded_key = vm.create_string("loaded".to_string());
                    if let Some(loaded_table) = package_rc
                        .borrow()
                        .raw_get(&LuaValue::from_string_rc(loaded_key))
                    {
                        if let Some(loaded_rc) = loaded_table.as_table() {
                            let mod_key = vm.create_string(modname_str.to_string());
                            loaded_rc
                                .borrow_mut()
                                .raw_set(LuaValue::from_string_rc(mod_key), module_value.clone());
                        }
                    }

                    return Ok(MultiValue::single(module_value));
                } else if let Some(err_str) = first_result.as_string() {
                    // It's an error message
                    error_messages.push(err_str.as_str().to_string());
                }
            }

            i += 1;
        }
    }

    // All searchers failed
    if error_messages.is_empty() {
        Err(format!("module '{}' not found", modname_str))
    } else {
        Err(format!(
            "module '{}' not found:{}",
            modname_str,
            error_messages.join("")
        ))
    }
}

/// load(chunk [, chunkname [, mode [, env]]]) - Load a chunk
fn lua_load(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let chunk_val = require_arg(vm, 0, "load")?;

    // Get the chunk string
    let code = unsafe {
        chunk_val
            .as_string()
            .ok_or_else(|| "bad argument #1 to 'load' (string expected)".to_string())?
            .as_str()
            .to_string()
    };

    // Optional chunk name for error messages
    let _chunkname = get_arg(vm, 1)
        .and_then(|v| unsafe { v.as_string().map(|s| s.as_str().to_string()) })
        .unwrap_or_else(|| "=(load)".to_string());

    // Optional mode ("b", "t", or "bt") - we only support "t" (text)
    let _mode = get_arg(vm, 2)
        .and_then(|v| unsafe { v.as_string().map(|s| s.as_str().to_string()) })
        .unwrap_or_else(|| "bt".to_string());

    // Optional environment table
    let _env = get_arg(vm, 3);

    // Compile the code
    match crate::Compiler::compile(&code) {
        Ok(chunk) => {
            let func = LuaValue::from_function_rc(std::rc::Rc::new(crate::LuaFunction {
                chunk: std::rc::Rc::new(chunk),
                upvalues: vec![],
            }));
            Ok(MultiValue::single(func))
        }
        Err(e) => {
            // Return nil and error message
            let err_msg = vm.create_string(format!("load error: {}", e));
            Ok(MultiValue::multiple(vec![
                LuaValue::nil(),
                LuaValue::from_string_rc(err_msg),
            ]))
        }
    }
}

/// loadfile([filename [, mode [, env]]]) - Load a file as a chunk
fn lua_loadfile(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let filename =
        get_arg(vm, 0).and_then(|v| unsafe { v.as_string().map(|s| s.as_str().to_string()) });

    let code = if let Some(fname) = filename {
        // Load from specified file
        match std::fs::read_to_string(&fname) {
            Ok(c) => c,
            Err(e) => {
                let err_msg = vm.create_string(format!("cannot open {}: {}", fname, e));
                return Ok(MultiValue::multiple(vec![
                    LuaValue::nil(),
                    LuaValue::from_string_rc(err_msg),
                ]));
            }
        }
    } else {
        // Load from stdin (simplified: return nil for now)
        let err_msg = vm.create_string("stdin loading not implemented".to_string());
        return Ok(MultiValue::multiple(vec![
            LuaValue::nil(),
            LuaValue::from_string_rc(err_msg),
        ]));
    };

    // Compile the code
    match crate::Compiler::compile(&code) {
        Ok(chunk) => {
            let func = LuaValue::from_function_rc(std::rc::Rc::new(crate::LuaFunction {
                chunk: std::rc::Rc::new(chunk),
                upvalues: vec![],
            }));
            Ok(MultiValue::single(func))
        }
        Err(e) => {
            let err_msg = vm.create_string(format!("load error: {}", e));
            Ok(MultiValue::multiple(vec![
                LuaValue::nil(),
                LuaValue::from_string_rc(err_msg),
            ]))
        }
    }
}

/// dofile([filename]) - Execute a file
fn lua_dofile(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let filename =
        get_arg(vm, 0).and_then(|v| unsafe { v.as_string().map(|s| s.as_str().to_string()) });

    let code = if let Some(fname) = filename {
        // Load from specified file
        match std::fs::read_to_string(&fname) {
            Ok(c) => c,
            Err(e) => return Err(format!("cannot open {}: {}", fname, e)),
        }
    } else {
        // Load from stdin (simplified: return error for now)
        return Err("stdin loading not implemented".to_string());
    };

    // Compile and execute
    match crate::Compiler::compile(&code) {
        Ok(chunk) => {
            let func = LuaValue::from_function_rc(std::rc::Rc::new(crate::LuaFunction {
                chunk: std::rc::Rc::new(chunk),
                upvalues: vec![],
            }));

            // Call the function
            let (success, results) = vm.protected_call(func, vec![]);

            if success {
                Ok(MultiValue::multiple(results))
            } else {
                let error_msg = unsafe {
                    results
                        .first()
                        .and_then(|v| v.as_string())
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_else(|| "unknown error".to_string())
                };
                Err(error_msg)
            }
        }
        Err(e) => Err(format!("load error: {}", e)),
    }
}

/// warn(msg1, ...) - Emit a warning
fn lua_warn(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    let args = get_args(_vm);

    let messages: Vec<String> = args.iter().map(|v| v.to_string_repr()).collect();
    let message = messages.join("");

    // Emit warning to stderr
    eprintln!("Lua warning: {}", message);

    Ok(MultiValue::empty())
}
