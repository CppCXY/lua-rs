// Basic library (_G global functions)
// Implements: print, type, assert, error, tonumber, tostring,
// select, ipairs, pairs, next, pcall, xpcall, getmetatable, setmetatable,
// rawget, rawset, rawlen, rawequal, collectgarbage, dofile, loadfile, load

use crate::lib_registry::{LibraryModule, get_arg, get_args, require_arg};
use crate::lua_value::{LuaValue, LuaValueKind, MultiValue, LuaUpvalue};
use crate::lua_vm::{LuaError, LuaResult, LuaVM};

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
fn lua_print(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    let output: Vec<String> = args
        .iter()
        .map(|v| vm.value_to_string(v).unwrap_or_else(|_| v.to_string_repr()))
        .collect();

    if !output.is_empty() {
        println!("{}", output.join("\t"));
    } else {
        println!();
    }

    Ok(MultiValue::empty())
}

/// type(v) - Return the type of a value as a string
fn lua_type(vm: &mut LuaVM) -> LuaResult<MultiValue> {
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

    let result = vm.create_string(type_name);
    Ok(MultiValue::single(result))
}

/// assert(v [, message]) - Raise error if v is false or nil
fn lua_assert(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let condition = get_arg(vm, 0).unwrap_or(LuaValue::nil());

    if !condition.is_truthy() {
        let message = get_arg(vm, 1)
            .and_then(|v| v.as_lua_string().map(|s| s.as_str().to_string()))
            .unwrap_or_else(|| "assertion failed!".to_string());
        return Err(LuaError::RuntimeError(message));
    }

    // Return all arguments
    Ok(MultiValue::multiple(get_args(vm)))
}

/// error(message) - Raise an error
fn lua_error(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let message = get_arg(vm, 0)
        .map(|v| {
            vm.value_to_string(&v)
                .unwrap_or_else(|_| v.to_string_repr())
        })
        .unwrap_or_else(|| "error".to_string());

    // Return error message directly for now
    Err(LuaError::RuntimeError(message))
}

/// tonumber(e [, base]) - Convert to number
fn lua_tonumber(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let value = get_arg(vm, 0).unwrap_or(LuaValue::nil());
    let base = get_arg(vm, 1).and_then(|v| v.as_integer()).unwrap_or(10);

    if base < 2 || base > 36 {
        return Err(LuaError::RuntimeError(
            "bad argument #2 to 'tonumber' (base out of range)".to_string(),
        ));
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
fn lua_tostring(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let value = get_arg(vm, 0).unwrap_or(LuaValue::nil());

    // Check for __tostring metamethod
    let value_str = vm.value_to_string(&value)?;
    let result = vm.create_string(&value_str);
    Ok(MultiValue::single(result))
}

/// select(index, ...) - Return subset of arguments
fn lua_select(vm: &mut LuaVM) -> LuaResult<MultiValue> {
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

    let index = index_arg.as_integer().ok_or_else(|| {
        LuaError::RuntimeError("bad argument #1 to 'select' (number expected)".to_string())
    })?;

    if index == 0 {
        return Err(LuaError::RuntimeError(
            "bad argument #1 to 'select' (index out of range)".to_string(),
        ));
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
fn lua_ipairs(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 0, "ipairs")?;

    // Validate that it's a table
    if table_val.as_table_id().is_none() {
        return Err(LuaError::RuntimeError(
            "bad argument #1 to 'ipairs' (table expected)".to_string(),
        ));
    }

    // Return iterator function, table, and 0
    let iter_func = LuaValue::cfunction(ipairs_next);

    Ok(MultiValue::multiple(vec![
        iter_func,
        table_val,
        LuaValue::integer(0),
    ]))
}

/// Iterator function for ipairs - Ultra-optimized for performance
#[inline]
fn ipairs_next(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Ultra-fast path: direct argument access without validation
    let table_val = if let Some(val) = get_arg(vm, 0) {
        val
    } else {
        return Err(LuaError::RuntimeError(
            "ipairs iterator: table expected".to_string(),
        ));
    };

    let index_val = if let Some(val) = get_arg(vm, 1) {
        val
    } else {
        return Err(LuaError::RuntimeError(
            "ipairs iterator: index expected".to_string(),
        ));
    };

    // Fast type check using direct pointer (ZERO ObjectPool lookup!)
    if let Some(table_ptr) = table_val.as_table_ptr() {
        if let Some(index) = index_val.as_integer() {
            let next_index = index + 1;

            // Direct table access via pointer
            unsafe {
                let table = (*table_ptr).borrow();
                if let Some(value) = table.get_int(next_index) {
                    drop(table);
                    return Ok(MultiValue::multiple(vec![
                        LuaValue::integer(next_index),
                        value,
                    ]));
                }
                // Reached end of array
                return Ok(MultiValue::single(LuaValue::nil()));
            }
        }
    }

    // Slow path with proper validation
    Err(LuaError::RuntimeError(
        "ipairs iterator: invalid table or index".to_string(),
    ))
}

/// pairs(t) - Return iterator for all key-value pairs
fn lua_pairs(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 0, "pairs")?;

    // Validate that it's a table
    if table_val.as_table_id().is_none() {
        return Err(LuaError::RuntimeError(
            "bad argument #1 to 'pairs' (table expected)".to_string(),
        ));
    }

    // TODO: Check for __pairs metamethod

    // Return next function, table, and nil
    let next_func = LuaValue::cfunction(lua_next);
    let nil_val = LuaValue::nil();

    Ok(MultiValue::multiple(vec![next_func, table_val, nil_val]))
}

/// next(table [, index]) - Return next key-value pair
fn lua_next(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 0, "next")?;
    let index_val = get_arg(vm, 1).unwrap_or(LuaValue::nil());

    // Use the table's built-in next() method which maintains proper iteration order
    let result = vm
        .get_table(&table_val)
        .ok_or(LuaError::RuntimeError("Invalid table".to_string()))?
        .borrow()
        .next(&index_val);

    match result {
        Some((key, value)) => Ok(MultiValue::multiple(vec![key, value])),
        None => Ok(MultiValue::single(LuaValue::nil())),
    }
}

/// pcall(f [, arg1, ...]) - Protected call
fn lua_pcall(vm: &mut LuaVM) -> LuaResult<MultiValue> {
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
    let (success, results) = vm.protected_call(func, args)?;

    // Return status and results
    let mut return_values = vec![LuaValue::boolean(success)];
    return_values.extend(results);

    Ok(MultiValue::multiple(return_values))
}

/// xpcall(f, msgh [, arg1, ...]) - Protected call with error handler
fn lua_xpcall(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // xpcall(f, msgh, arg1, arg2, ...) -> status, result or error

    eprintln!("[lua_xpcall] Called");
    let all_args = get_args(vm);
    eprintln!("[lua_xpcall] Total args: {}", all_args.len());
    for (i, arg) in all_args.iter().enumerate() {
        eprintln!("[lua_xpcall] Arg {}: {:?}", i, arg.kind());
    }

    // Get the function to call (argument 0)
    let func = require_arg(vm, 0, "xpcall")?;
    eprintln!("[lua_xpcall] func type: {:?}", func.kind());

    // Get the error handler (argument 1)
    let err_handler = require_arg(vm, 1, "xpcall")?;
    eprintln!("[lua_xpcall] err_handler type: {:?}", err_handler.kind());

    // Get all arguments after the function and error handler
    let all_args = get_args(vm);
    let args: Vec<LuaValue> = if all_args.len() > 2 {
        all_args[2..].to_vec()
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
    let value = require_arg(vm, 0, "getmetatable")?;

    match value.kind() {
        LuaValueKind::Table => {
            let table_ref = vm
                .get_table(&value)
                .ok_or(LuaError::RuntimeError("Invalid table".to_string()))?;
            if let Some(mt) = table_ref.borrow().get_metatable() {
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
        // TODO: Support metatables for other types (userdata, numbers, etc.)
        _ => Ok(MultiValue::single(LuaValue::nil())),
    }
}

/// setmetatable(table, metatable) - Set metatable
fn lua_setmetatable(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table = require_arg(vm, 0, "setmetatable")?;
    let metatable = require_arg(vm, 1, "setmetatable")?;

    // First argument must be a table
    if table.is_table() {
        let table_ref = vm
            .get_table(&table)
            .ok_or(LuaError::RuntimeError("Invalid table".to_string()))?;
        // Set the new metatable
        match metatable.kind() {
            LuaValueKind::Nil => {
                table_ref.borrow_mut().set_metatable(None);
            }
            LuaValueKind::Table => {
                // Just pass the metatable TableId as LuaValue
                table_ref
                    .borrow_mut()
                    .set_metatable(Some(metatable.clone()));
            }
            _ => {
                return Err(LuaError::RuntimeError(
                    "setmetatable() second argument must be a table or nil".to_string(),
                ));
            }
        }

        // Return the original table
        Ok(MultiValue::single(table.clone()))
    } else {
        Err(LuaError::RuntimeError(
            "setmetatable() first argument must be a table".to_string(),
        ))
    }
}

/// rawget(table, index) - Get without metamethods
fn lua_rawget(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table = require_arg(vm, 0, "rawget")?;
    let key = require_arg(vm, 1, "rawget")?;

    if table.is_table() {
        let table_ref = vm
            .get_table(&table)
            .ok_or(LuaError::RuntimeError("Invalid table".to_string()))?;
        let value = table_ref.borrow().raw_get(&key).unwrap_or(LuaValue::nil());
        Ok(MultiValue::single(value))
    } else {
        Err(LuaError::RuntimeError(
            "rawget() first argument must be a table".to_string(),
        ))
    }
}

/// rawset(table, index, value) - Set without metamethods
fn lua_rawset(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table = require_arg(vm, 0, "rawset")?;
    let key = require_arg(vm, 1, "rawset")?;
    let value = require_arg(vm, 2, "rawset")?;

    if table.is_table() {
        if key.is_nil() {
            return Err(LuaError::RuntimeError("table index is nil".to_string()));
        }

        let table_ref = vm
            .get_table(&table)
            .ok_or(LuaError::RuntimeError("Invalid table".to_string()))?;
        table_ref.borrow_mut().raw_set(key.clone(), value.clone());
        Ok(MultiValue::single(table.clone()))
    } else {
        Err(LuaError::RuntimeError(
            "rawset() first argument must be a table".to_string(),
        ))
    }
}

/// rawlen(v) - Length without metamethods
fn lua_rawlen(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let value = require_arg(vm, 0, "rawlen")?;

    let len = unsafe {
        match value.kind() {
            LuaValueKind::Table => value.as_lua_table().unwrap().borrow().len() as i64,
            LuaValueKind::String => {
                if let Some(s) = value.as_string() {
                    s.as_str().len() as i64
                } else {
                    return Err(LuaError::RuntimeError(
                        "rawlen() argument must be a table or string".to_string(),
                    ));
                }
            }
            _ => {
                return Err(LuaError::RuntimeError(
                    "rawlen() argument must be a table or string".to_string(),
                ));
            }
        }
    };

    Ok(MultiValue::single(LuaValue::integer(len)))
}

/// rawequal(v1, v2) - Equality without metamethods
fn lua_rawequal(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let v1 = get_arg(vm, 0).unwrap_or(LuaValue::nil());
    let v2 = get_arg(vm, 1).unwrap_or(LuaValue::nil());

    let result = v1 == v2;
    Ok(MultiValue::single(LuaValue::boolean(result)))
}

/// collectgarbage([opt [, arg]]) - Garbage collector control
fn lua_collectgarbage(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let opt = get_arg(vm, 0)
        .and_then(|v| v.as_lua_string().map(|s| s.as_str().to_string()))
        .unwrap_or_else(|| "=(load)".to_string());

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
        _ => Err(LuaError::RuntimeError(format!(
            "collectgarbage: invalid option '{}'",
            opt
        ))),
    }
}

/// _VERSION - Lua version string
fn lua_version(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let version = vm.create_string("Lua 5.4");
    Ok(MultiValue::single(version))
}

/// require(modname) - Load a module  
fn lua_require(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let modname_str = require_arg(vm, 0, "require")?;
    if !modname_str.is_string() {
        return Err(LuaError::RuntimeError(
            "module name must be a string".to_string(),
        ));
    }

    // Check if module is already loaded in package.loaded
    if let Some(package_table) = vm.get_global("package") {
        if let Some(_package_id) = package_table.as_table_id() {
            let loaded_key = vm.create_string("loaded");
            let package_ref = vm
                .get_table(&package_table)
                .ok_or(LuaError::RuntimeError("Invalid package table".to_string()))?;
            if let Some(loaded_table) = package_ref.borrow().raw_get(&loaded_key) {
                if loaded_table.is_table() {
                    if let Some(module_value) = vm
                        .get_table(&loaded_table)
                        .ok_or(LuaError::RuntimeError("Invalid loaded table".to_string()))?
                        .borrow()
                        .raw_get(&modname_str)
                    {
                        if !module_value.is_nil() {
                            return Ok(MultiValue::single(module_value));
                        }
                    }
                }
                // Module not in loaded table - continue to searchers
            }
        }
    }

    // Try each searcher in package.searchers
    let mut error_messages = Vec::new();

    let package_table = vm
        .get_global("package")
        .ok_or_else(|| LuaError::RuntimeError("package table not found".to_string()))?;

    let key = vm.create_string("searchers");
    let package_ref = vm
        .get_table(&package_table)
        .ok_or(LuaError::RuntimeError("Invalid package table".to_string()))?;
    let searchers_val = package_ref
        .borrow()
        .raw_get(&key)
        .unwrap_or(LuaValue::nil());

    let searchers_table_val = searchers_val;
    let _searchers_id = searchers_table_val
        .as_table_id()
        .ok_or_else(|| LuaError::RuntimeError("package.searchers is not a table".to_string()))?;

    // Try each searcher (1-based indexing)
    let mut i = 1;
    loop {
        let searcher_key = LuaValue::integer(i);
        let searchers_ref = vm
            .get_table(&searchers_table_val)
            .ok_or(LuaError::RuntimeError(
                "Invalid searchers table".to_string(),
            ))?;
        let searcher = searchers_ref
            .borrow()
            .raw_get(&searcher_key)
            .unwrap_or(LuaValue::nil());

        if searcher.is_nil() {
            break; // No more searchers
        }

        // Call searcher with module name
        let (success, results) = vm.protected_call(searcher.clone(), vec![modname_str.clone()])?;

        if !success {
            let error_msg = results
                .first()
                .and_then(|v| v.as_lua_string())
                .map(|s| s.as_str().to_string())
                .unwrap_or_else(|| "unknown error in searcher".to_string());
            return Err(LuaError::RuntimeError(format!(
                "error calling searcher: {}",
                error_msg
            )));
        }

        // Check result
        if !results.is_empty() {
            let first_result = &results[0];

            // If it's a function, this is the loader
            if first_result.is_function() || first_result.is_cfunction() {
                // Call the loader
                let loader_args = if results.len() > 1 {
                    vec![modname_str.clone(), results[1].clone()]
                } else {
                    vec![modname_str.clone()]
                };

                let (load_success, load_results) =
                    vm.protected_call(first_result.clone(), loader_args)?;

                if !load_success {
                    let error_msg = load_results
                        .first()
                        .and_then(|v| v.as_lua_string())
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_else(|| "unknown error".to_string());
                    return Err(LuaError::RuntimeError(format!(
                        "error loading module '{}': {}",
                        modname_str, error_msg
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
                let package_ref = vm
                    .get_table(&package_table)
                    .ok_or(LuaError::RuntimeError("Invalid package table".to_string()))?;
                if let Some(loaded_table) = package_ref.borrow().raw_get(&loaded_key) {
                    if let Some(_loaded_id) = loaded_table.as_table_id() {
                        let loaded_ref = vm
                            .get_table(&loaded_table)
                            .ok_or(LuaError::RuntimeError("Invalid loaded table".to_string()))?;
                        loaded_ref
                            .borrow_mut()
                            .raw_set(modname_str, module_value.clone());
                    }
                }

                return Ok(MultiValue::single(module_value));
            } else if let Some(err_str) = first_result.as_lua_string() {
                // It's an error message
                error_messages.push(err_str.as_str().to_string());
            }
        }

        i += 1;
    }

    // All searchers failed
    if error_messages.is_empty() {
        Err(LuaError::RuntimeError(format!(
            "module '{}' not found",
            modname_str
        )))
    } else {
        Err(LuaError::RuntimeError(format!(
            "module '{}' not found:{}",
            modname_str,
            error_messages.join("")
        )))
    }
}

/// load(chunk [, chunkname [, mode [, env]]]) - Load a chunk
fn lua_load(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let chunk_val = require_arg(vm, 0, "load")?;

    // Get the chunk string
    let code = unsafe {
        chunk_val
            .as_string()
            .ok_or_else(|| {
                LuaError::RuntimeError("bad argument #1 to 'load' (string expected)".to_string())
            })?
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

    // Compile the code using VM's string pool
    match vm.compile(&code) {
        Ok(chunk) => {
            // Create upvalue for _ENV (global table)
            // Loaded chunks need _ENV as upvalue[0]
            let env_upvalue = LuaUpvalue::new_closed(vm.globals);
            let upvalues = vec![env_upvalue];
            
            let func = vm.create_function(std::rc::Rc::new(chunk), upvalues);
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
    let filename =
        get_arg(vm, 0).and_then(|v| unsafe { v.as_string().map(|s| s.as_str().to_string()) });

    let code = if let Some(fname) = filename {
        // Load from specified file
        match std::fs::read_to_string(&fname) {
            Ok(c) => c,
            Err(e) => {
                let err_msg = vm.create_string(&format!("cannot open {}: {}", fname, e));
                return Ok(MultiValue::multiple(vec![LuaValue::nil(), err_msg]));
            }
        }
    } else {
        // Load from stdin (simplified: return nil for now)
        let err_msg = vm.create_string("stdin loading not implemented");
        return Ok(MultiValue::multiple(vec![LuaValue::nil(), err_msg]));
    };

    // Compile the code using VM's string pool
    match vm.compile(&code) {
        Ok(chunk) => {
            let func = vm.create_function(std::rc::Rc::new(chunk), vec![]);
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
    let filename =
        get_arg(vm, 0).and_then(|v| unsafe { v.as_string().map(|s| s.as_str().to_string()) });

    let code = if let Some(fname) = filename {
        // Load from specified file
        match std::fs::read_to_string(&fname) {
            Ok(c) => c,
            Err(e) => {
                return Err(LuaError::RuntimeError(format!(
                    "cannot open {}: {}",
                    fname, e
                )));
            }
        }
    } else {
        // Load from stdin (simplified: return error for now)
        return Err(LuaError::RuntimeError(
            "stdin loading not implemented".to_string(),
        ));
    };

    // Compile and execute using VM's string pool
    match vm.compile(&code) {
        Ok(chunk) => {
            let func = vm.create_function(std::rc::Rc::new(chunk), vec![]);

            // Call the function
            let (success, results) = vm.protected_call(func, vec![])?;

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
                Err(LuaError::RuntimeError(error_msg))
            }
        }
        Err(e) => Err(LuaError::RuntimeError(format!("load error: {}", e))),
    }
}

/// warn(msg1, ...) - Emit a warning
fn lua_warn(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    let messages: Vec<String> = args
        .iter()
        .map(|v| vm.value_to_string(v).unwrap_or_else(|_| v.to_string_repr()))
        .collect();
    let message = messages.join("");

    // Emit warning to stderr
    eprintln!("Lua warning: {}", message);

    Ok(MultiValue::empty())
}
