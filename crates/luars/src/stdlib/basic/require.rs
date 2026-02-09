use crate::{LuaResult, LuaValue, lua_vm::LuaState};

/// require(modname) - Load a module
/// Implementation following standard Lua 5.5 semantics (loadlib.c ll_require)
pub fn lua_require(l: &mut LuaState) -> LuaResult<usize> {
    let modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'require' (string expected)".to_string()))?;

    let Some(modname_str) = modname_val.as_str() else {
        return Err(l.error("bad argument #1 to 'require' (string expected)".to_string()));
    };

    // Get package.loaded from registry (like standard Lua's LUA_LOADED_TABLE)
    // This ensures require works even if the global 'package' is reassigned
    let vm = l.vm_mut();
    let loaded_val = vm
        .registry_get("_LOADED")?
        .ok_or_else(|| vm.main_state().error("package.loaded not found".to_string()))?;

    // Get the original package table from registry for searchers
    let package_table_value = vm
        .registry_get("_PACKAGE")?
        .ok_or_else(|| vm.main_state().error("package table not found".to_string()))?;

    let Some(loaded_table) = loaded_val.as_table_mut() else {
        return Err(l.error("package.loaded must be a table".to_string()));
    };

    // Check if module is already loaded (using lua_toboolean semantics)
    // nil and false both mean "not loaded"
    let already_loaded = loaded_table
        .raw_get(&modname_val)
        .unwrap_or(LuaValue::nil());

    let is_loaded = match &already_loaded {
        v if v.is_nil() => false,
        v if v.as_boolean() == Some(false) => false,
        _ => true,
    };

    if is_loaded {
        // Package is already loaded, return it (only 1 value for cached)
        l.push_value(already_loaded)?;
        return Ok(1);
    }

    // Get package.searchers from the original package table (stored in registry)
    let searchers_key = l.create_string("searchers")?;
    let Some(package_table) = package_table_value.as_table() else {
        return Err(l.error("package must be a table".to_string()));
    };
    let searchers_val = match package_table.raw_get(&searchers_key) {
        Some(v) => v,
        None => return Err(l.error("package.searchers not found".to_string())),
    };

    let Some(searchers_table) = searchers_val.as_table() else {
        return Err(l.error("package.searchers must be a table".to_string()));
    };

    // Collect error messages from searchers
    let mut error_messages = Vec::new();

    // Try each searcher (iterate until we hit nil)
    let mut i = 1;
    loop {
        let searcher = searchers_table
            .raw_geti(i as i64)
            .unwrap_or(LuaValue::nil());

        if searcher.is_nil() {
            break;
        }

        // Call searcher(modname)
        l.push_value(searcher)?;
        l.push_value(modname_val)?;
        let func_idx = l.get_top() - 2;

        let (success, result_count) = l.pcall_stack_based(func_idx, 1)?;

        if !success {
            // Searcher threw an error
            let error_msg = l.stack_get(func_idx).unwrap_or(LuaValue::nil());
            if let Some(err) = error_msg.as_str() {
                error_messages.push(err.to_string());
            }
            l.set_top(func_idx)?;
            i += 1;
            continue;
        }

        if result_count == 0 {
            l.set_top(func_idx)?;
            i += 1;
            continue;
        }

        // Get first result (loader or error message)
        let first_result = l.stack_get(func_idx).unwrap_or(LuaValue::nil());

        // If result is nil or false, searcher didn't find the module
        if first_result.is_nil() || (first_result.as_boolean() == Some(false)) {
            l.set_top(func_idx)?;
            i += 1;
            continue;
        }

        // Check if result is a function (loader found!)
        let is_function = first_result.is_function() || first_result.is_cfunction();

        // If result is not a function, it must be a string error message
        if !is_function {
            if let Some(msg) = first_result.as_str() {
                error_messages.push(msg.to_string());
            }

            l.set_top(func_idx)?;
            i += 1;
            continue;
        }

        // Found a loader! Get loader and optional data
        let loader = first_result;
        let loader_data = if result_count >= 2 {
            l.stack_get(func_idx + 1).unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        };

        // Call loader(modname, loader_data) to get the module
        // Clean stack and prepare to call loader
        l.set_top(func_idx)?;

        l.push_value(loader)?;
        l.push_value(modname_val)?;
        if !loader_data.is_nil() {
            l.push_value(loader_data)?;
        }
        let loader_func_idx = l.get_top() - if loader_data.is_nil() { 2 } else { 3 };
        let loader_nargs = if loader_data.is_nil() { 1 } else { 2 };

        let (loader_success, loader_result_count) =
            l.pcall_stack_based(loader_func_idx, loader_nargs)?;

        if !loader_success {
            // Loader failed
            let error_val = l.stack_get(loader_func_idx).unwrap_or(LuaValue::nil());
            let error_msg = if let Some(err) = error_val.as_str() {
                err.to_string()
            } else {
                "error loading module".to_string()
            };
            return Err(l.error(format!(
                "error loading module '{}': {}",
                modname_str, error_msg
            )));
        }

        // Get the loader result
        let loader_result = if loader_result_count > 0 {
            l.stack_get(loader_func_idx).unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        };

        // Standard Lua semantics (from loadlib.c):
        // 1. If loader returned non-nil, set LOADED[name] = returned_value
        if !loader_result.is_nil() {
            l.raw_set(&loaded_val, modname_val, loader_result);
        }

        // 2. If LOADED[name] is still nil, set LOADED[name] = true
        let current_loaded = loaded_val
            .as_table()
            .and_then(|t| t.raw_get(&modname_val))
            .unwrap_or(LuaValue::nil());

        let final_result = if current_loaded.is_nil() {
            let val = LuaValue::boolean(true);
            l.raw_set(&loaded_val, modname_val, val);
            val
        } else {
            current_loaded
        };

        // Clean up stack and return module result + loader_data
        l.set_top(loader_func_idx)?;
        l.push_value(final_result)?;
        l.push_value(loader_data)?;
        return Ok(2);
    }

    // No searcher found the module
    let error_msg = if error_messages.is_empty() {
        format!("module '{}' not found", modname_str)
    } else {
        format!(
            "module '{}' not found:{}",
            modname_str,
            error_messages.join("")
        )
    };

    Err(l.error(error_msg))
}
