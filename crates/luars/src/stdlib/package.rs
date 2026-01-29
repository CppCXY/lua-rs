// Package library
// Implements: config, cpath, loaded, loadlib, path, preload, searchers, searchpath

use std::rc::Rc;

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaError, LuaResult, LuaState};

pub fn create_package_lib() -> LibraryModule {
    crate::lib_module!("package", {
        "loadlib" => package_loadlib,
        "searchpath" => package_searchpath,
    })
    .with_initializer(init_package_fields)
}

// Initialize package library fields (called after module is loaded)
pub fn init_package_fields(l: &mut LuaState) -> LuaResult<()> {
    // Get package table (should already exist from module creation)
    let package_table = l
        .get_global("package")?
        .ok_or_else(|| l.error("package table not found".to_string()))?;

    if !package_table.is_table() {
        return Err(l.error("package must be a table".to_string()));
    };

    // Create all keys
    let loaded_key = l.create_string("loaded")?;
    let preload_key = l.create_string("preload")?;
    let path_key = l.create_string("path")?;
    let cpath_key = l.create_string("cpath")?;
    let config_key = l.create_string("config")?;
    let searchers_key = l.create_string("searchers")?;

    // Create all values
    let loaded_table = l.create_table(0, 0)?;
    let preload_table = l.create_table(0, 0)?;
    let path_value = l.create_string("./?.lua;./?/init.lua")?;
    let cpath_value = l.create_string("./?.so;./?.dll;./?.dylib")?;

    #[cfg(windows)]
    let config_str = "\\\n;\n?\n!\n-";
    #[cfg(not(windows))]
    let config_str = "/\n;\n?\n!\n-";
    let config_value = l.create_string(config_str)?;

    // Create searchers array
    let searchers_table_value = l.create_table(4, 0)?;
    let searchers_table = searchers_table_value.as_table_mut().unwrap();

    // Fill searchers array
    searchers_table.raw_seti(1, LuaValue::cfunction(searcher_preload));
    searchers_table.raw_seti(2, LuaValue::cfunction(searcher_lua));

    // Set all fields in package table
    l.raw_set(&package_table, loaded_key, loaded_table);
    l.raw_set(&package_table, preload_key, preload_table);
    l.raw_set(&package_table, path_key, path_value);
    l.raw_set(&package_table, cpath_key, cpath_value);
    l.raw_set(&package_table, config_key, config_value);
    l.raw_set(&package_table, searchers_key, searchers_table_value);

    Ok(())
}

// Searcher 1: Check package.preload
fn searcher_preload(l: &mut LuaState) -> LuaResult<usize> {
    let modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("module name expected".to_string()))?;

    // let Some(modname_id) = modname_val.as_string_id() else {
    //     return Err(l.error("module name expected".to_string()));
    // };

    // let modname_str = {
    //     let vm = l.vm_mut();
    //     let Some(s) = vm.object_pool.get_string(modname_id) else {
    //         return Err(l.error("module name expected".to_string()));
    //     };
    //     s.to_string()
    // };

    let package_table = l
        .get_global("package")?
        .ok_or_else(|| l.error("package table not found".to_string()))?;

    let Some(package_table) = package_table.as_table() else {
        return Err(l.error("Invalid package table".to_string()));
    };

    let preload_key = l.create_string("preload")?;
    let preload_val = package_table
        .raw_get(&preload_key)
        .unwrap_or(LuaValue::nil());

    let Some(preload_table) = preload_val.as_table_mut() else {
        return Err(l.error("package.preload is not a table".to_string()));
    };

    let loader = preload_table
        .raw_get(&modname_val)
        .unwrap_or(LuaValue::nil());

    if loader.is_nil() {
        l.push_value(LuaValue::boolean(false))?;
        Ok(1)
    } else {
        l.push_value(loader)?;
        let preload_str = l.create_string(":preload:")?;
        l.push_value(preload_str)?;
        Ok(2)
    }
}

// Searcher 2: Search package.path
fn searcher_lua(l: &mut LuaState) -> LuaResult<usize> {
    let modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("module name expected".to_string()))?;

    let Some(modname) = modname_val.as_str() else {
        return Err(l.error("module name expected".to_string()));
    };

    let package_val = l
        .get_global("package")?
        .ok_or_else(|| l.error("package table not found".to_string()))?;

    let Some(package_table) = package_val.as_table() else {
        return Err(l.error("Invalid package table".to_string()));
    };

    let path_key = l.create_string("path")?;

    let Some(path_value) = package_table.raw_get(&path_key) else {
        return Err(LuaError::RuntimeError);
    };
    let Some(path_str) = path_value.as_str() else {
        return Err(LuaError::RuntimeError);
    };

    // Search for the file
    let result = search_path(&modname, &path_str, ".", "/")?;

    match result {
        Some(filepath) => {
            l.push_value(LuaValue::cfunction(lua_file_loader))?;
            let filepath_str = l.create_string(&filepath)?;
            l.push_value(filepath_str)?;
            Ok(2)
        }
        None => {
            let err = format!(
                "\n\tno file '{}'",
                path_str
                    .split(';')
                    .map(|template| { template.replace('?', &modname.replace('.', "/")) })
                    .collect::<Vec<_>>()
                    .join("'\n\tno file '")
            );
            let err_str = l.create_string(&err)?;
            l.push_value(err_str)?;
            Ok(1)
        }
    }
}

// Loader function for Lua files (called by searcher_lua)
// Called as: loader(modname, filepath)
fn lua_file_loader(l: &mut LuaState) -> LuaResult<usize> {
    // First arg is modname, second arg is filepath (passed by searcher)
    let _modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("module name expected".to_string()))?;
    let filepath_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("file path expected".to_string()))?;

    let Some(filepath_str) = filepath_val.as_str() else {
        return Err(l.error("file path must be a string".to_string()));
    };

    if !std::fs::metadata(&filepath_str).is_ok() {
        return Ok(0);
    }

    // Read the file
    let source = match std::fs::read_to_string(&filepath_str) {
        Ok(s) => s,
        Err(e) => {
            return Err(l.error(format!("cannot open file '{}': {}", filepath_str, e)));
        }
    };

    let vm = l.vm_mut();

    // Compile it using VM's string pool with chunk name
    let chunkname = format!("@{}", filepath_str);
    let chunk = vm.compile_with_name(&source, &chunkname)?;

    // Create a function from the chunk with _ENV upvalue
    let env_upvalue = vm.create_upvalue_closed(vm.global)?;
    let func = vm.create_function(Rc::new(chunk), vec![env_upvalue])?;

    // Call the function to execute the module and get its return value
    // The module should return its exports (usually a table)
    l.push_value(func)?;
    let func_idx = l.get_top() - 1;
    let (success, result_count) = l.pcall_stack_based(func_idx, 0)?;

    if !success {
        // Module threw an error
        let error_val = l.stack_get(func_idx).unwrap_or(LuaValue::nil());
        let error_msg = if let Some(err) = error_val.as_str() {
            err.to_string()
        } else {
            "error loading module".to_string()
        };
        return Err(l.error(format!(
            "error loading module from '{}': {}",
            filepath_str, error_msg
        )));
    }

    // Return what the module returned (or nil if it returned nothing)
    if result_count > 0 {
        // Module returned a value, keep it on stack
        Ok(1)
    } else {
        // Module returned nothing, return nil
        l.push_value(LuaValue::nil())?;
        Ok(1)
    }
}

// Helper: Search for a file in path templates
fn search_path(name: &str, path: &str, sep: &str, rep: &str) -> LuaResult<Option<String>> {
    let searchname = name.replace(sep, rep);
    let templates: Vec<&str> = path.split(';').collect();

    for template in templates {
        let filepath = template.replace('?', &searchname);

        // Check if file exists
        if std::path::Path::new(&filepath).exists() {
            return Ok(Some(filepath));
        }
    }

    Ok(None)
}

fn package_loadlib(l: &mut LuaState) -> LuaResult<usize> {
    let err = l.create_string("loadlib not implemented")?;
    l.push_value(LuaValue::nil())?;
    l.push_value(err)?;
    Ok(2)
}

fn package_searchpath(l: &mut LuaState) -> LuaResult<usize> {
    let name_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'searchpath' (string expected)".to_string()))?;
    let path_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'searchpath' (string expected)".to_string()))?;

    let Some(name_str) = name_val.as_str() else {
        return Err(l.error("bad argument #1 to 'searchpath' (string expected)".to_string()));
    };

    let Some(path_str) = path_val.as_str() else {
        return Err(l.error("bad argument #2 to 'searchpath' (string expected)".to_string()));
    };

    // Optional sep and rep arguments
    let sep_val = l.get_arg(3);

    let sep = if let Some(sep_val) = &sep_val {
        sep_val.as_str().unwrap_or(".")
    } else {
        "."
    };

    let rep_val = l.get_arg(4);
    let rep = if let Some(rep_val) = &rep_val {
        rep_val.as_str().unwrap_or("/")
    } else {
        "/"
    };

    match search_path(&name_str, &path_str, &sep, &rep)? {
        Some(filepath) => {
            let filepath_str = l.create_string(&filepath)?;
            l.push_value(filepath_str)?;
            Ok(1)
        }
        None => {
            let searchname = name_str.replace(&sep, &rep);
            let err = format!(
                "\n\tno file '{}'",
                path_str
                    .split(';')
                    .map(|template| { template.replace('?', &searchname) })
                    .collect::<Vec<_>>()
                    .join("'\n\tno file '")
            );
            l.push_value(LuaValue::nil())?;
            let err_str = l.create_string(&err)?;
            l.push_value(err_str)?;
            Ok(2)
        }
    }
}
