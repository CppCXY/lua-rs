use crate::{LuaResult, LuaValue, lua_value::LuaTableImpl, lua_vm::LuaState};

/// require(modname) - Load a module  
/// Simplified implementation - loads from package.preload or package.path
pub fn lua_require(l: &mut LuaState) -> LuaResult<usize> {
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
        s.to_string()
    };

    // Get package table
    let package_table = l
        .get_global("package")
        .ok_or_else(|| l.error("package table not found".to_string()))?;

    let Some(package_id) = package_table.as_table_id() else {
        return Err(l.error("package must be a table".to_string()));
    };

    // Get package.loaded
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
                // false means loading, prevent recursive require
                return Err(l.error(format!(
                    "loop or previous error loading module '{}'",
                    modname_str
                )));
            }
        } else {
            // Non-nil, non-false value means already loaded
            l.push_value(already_loaded)?;
            return Ok(1);
        }
    }

    // Mark module as being loaded to prevent recursion
    {
        let vm = l.vm_mut();
        if let Some(loaded_table) = vm.object_pool.get_table_mut(loaded_id) {
            loaded_table.raw_set(&modname_val, LuaValue::boolean(false));
        }
    }

    // Get package.searchers
    let searchers_key = l.create_string("searchers");
    let searchers_val = {
        let vm = l.vm_mut();
        let Some(pkg_table) = vm.object_pool.get_table(package_id) else {
            return Err(l.error("package must be a table".to_string()));
        };
        pkg_table.raw_get(&searchers_key)
    };

    let Some(searchers_id) = searchers_val.and_then(|v| v.as_table_id()) else {
        return Err(l.error("package.searchers must be a table".to_string()));
    };

    // Collect error messages from searchers
    let mut error_messages = Vec::new();

    // Try each searcher (iterate until we hit nil)
    let mut i = 1;
    loop {
        let searcher = {
            let vm = l.vm_mut();
            let Some(searchers_table) = vm.object_pool.get_table(searchers_id) else {
                return Err(l.error("package.searchers must be a table".to_string()));
            };
            searchers_table.get_int(i as i64).unwrap_or(LuaValue::nil())
        };

        if searcher.is_nil() {
            break;
        }

        // Call searcher(modname)
        l.push_value(searcher)?;
        l.push_value(modname_val)?;
        let func_idx = l.get_top() - 2;

        let (success, _) = l.pcall_stack_based(func_idx, 1)?;

        // pcall_stack_based 对C函数可能不正确更新 stack_top
        // 手动检查结果（searchers 最多返回 2 个值）
        let mut result_count = 0;
        if l.stack_get(func_idx).is_some() {
            result_count = 1;
            if l.stack_get(func_idx + 1).is_some() {
                result_count = 2;
            }
        }

        if !success {
            // Searcher threw an error
            let error_msg = l.stack_get(func_idx).unwrap_or(LuaValue::nil());
            if let Some(err_id) = error_msg.as_string_id() {
                let vm = l.vm_mut();
                if let Some(err_str) = vm.object_pool.get_string(err_id) {
                    error_messages.push(err_str.to_string());
                }
            }
            l.set_top(func_idx);
            i += 1;
            continue;
        }

        if result_count == 0 {
            l.set_top(func_idx);
            i += 1;
            continue;
        }

        // Get first result (loader or error message)
        let first_result = l.stack_get(func_idx).unwrap_or(LuaValue::nil());

        // If result is nil or false, searcher didn't find the module
        if first_result.is_nil() || (first_result.as_boolean() == Some(false)) {
            l.set_top(func_idx);
            i += 1;
            continue;
        }

        // Check if result is a function (loader found!)
        let is_function = first_result.is_function() || first_result.is_cfunction();

        // If result is not a function, it must be a string error message
        if !is_function {
            if let Some(msg_id) = first_result.as_string_id() {
                let vm = l.vm_mut();
                if let Some(msg_str) = vm.object_pool.get_string(msg_id) {
                    error_messages.push(msg_str.to_string());
                }
            }
            l.set_top(func_idx);
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

        // Call loader(modname, loader_data)
        // 注意：不要在获取 loader 后立即清理栈，因为 LuaValue 可能包含对栈的引用
        // 我们直接在现有栈上继续操作

        // 清理 searcher 的参数和结果，准备调用 loader
        l.set_top(func_idx);

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
            let error_msg = if let Some(err_id) = error_val.as_string_id() {
                let vm = l.vm_mut();
                if let Some(err_str) = vm.object_pool.get_string(err_id) {
                    err_str.to_string()
                } else {
                    "error loading module".to_string()
                }
            } else {
                "error loading module".to_string()
            };
            return Err(l.error(format!(
                "error loading module '{}': {}",
                modname_str, error_msg
            )));
        }

        // Get the result from loader
        let module_result = if loader_result_count > 0 {
            l.stack_get(loader_func_idx).unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        };

        // If loader returned nil, use true instead
        let final_result = if module_result.is_nil() {
            LuaValue::boolean(true)
        } else {
            module_result
        };

        // Store in package.loaded
        {
            let vm = l.vm_mut();
            if let Some(loaded_table) = vm.object_pool.get_table_mut(loaded_id) {
                loaded_table.raw_set(&modname_val, final_result);
            }
        }

        // Clean up stack and return result
        l.set_top(loader_func_idx);
        l.push_value(final_result)?;
        return Ok(1);
    }

    // No searcher found the module
    // Clean up the false marker from package.loaded
    {
        let vm = l.vm_mut();
        if let Some(loaded_table) = vm.object_pool.get_table_mut(loaded_id) {
            loaded_table.raw_set(&modname_val, LuaValue::nil());
        }
    }

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
