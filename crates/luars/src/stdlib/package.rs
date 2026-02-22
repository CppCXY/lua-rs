// Package library
// Implements: config, cpath, loaded, loadlib, path, preload, searchers, searchpath

use std::rc::Rc;

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState};

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
    searchers_table.raw_seti(3, LuaValue::cfunction(searcher_c));
    searchers_table.raw_seti(4, LuaValue::cfunction(searcher_c_all_in_one));

    // Set all fields in package table
    l.raw_set(&package_table, loaded_key, loaded_table);
    l.raw_set(&package_table, preload_key, preload_table);
    l.raw_set(&package_table, path_key, path_value);
    l.raw_set(&package_table, cpath_key, cpath_value);
    l.raw_set(&package_table, config_key, config_value);
    l.raw_set(&package_table, searchers_key, searchers_table_value);

    // Add package itself to package.loaded (normally lib_registry does this,
    // but package.loaded doesn't exist yet when the package module is first loaded)
    let package_mod_key = l.create_string("package")?;
    l.raw_set(&loaded_table, package_mod_key, package_table);

    // Store loaded table and package table in registry for use by require
    // This matches standard Lua's LUA_LOADED_TABLE ("_LOADED") and upvalue approach
    let vm = l.vm_mut();
    vm.registry_set("_LOADED", loaded_table)?;
    vm.registry_set("_PRELOAD", preload_table)?;
    // Store the original package table so require can find searchers
    // even if the global 'package' is reassigned
    vm.registry_set("_PACKAGE", package_table)?;

    Ok(())
}

// Helper to get the original package table from registry
fn get_package_from_registry(l: &mut LuaState) -> LuaResult<LuaValue> {
    let vm = l.vm_mut();
    vm.registry_get("_PACKAGE")?
        .ok_or_else(|| vm.main_state().error("package table not found".to_string()))
}

// Searcher 1: Check package.preload
fn searcher_preload(l: &mut LuaState) -> LuaResult<usize> {
    let modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("module name expected".to_string()))?;

    // Get preload table from registry
    let vm = l.vm_mut();
    let preload_val = vm.registry_get("_PRELOAD")?.unwrap_or(LuaValue::nil());

    let Some(preload_table) = preload_val.as_table_mut() else {
        return Err(l.error("package.preload is not a table".to_string()));
    };

    let loader = preload_table
        .raw_get(&modname_val)
        .unwrap_or(LuaValue::nil());

    if loader.is_nil() {
        // Return error message like Lua 5.5
        let modname_str = modname_val.as_str().unwrap_or("?");
        let err_msg =
            l.create_string(&format!("\n\tno field package.preload['{}']", modname_str))?;
        l.push_value(err_msg)?;
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

    // Get the original package table from registry to access package.path
    let package_val = get_package_from_registry(l)?;

    let Some(package_table) = package_val.as_table() else {
        return Err(l.error("Invalid package table".to_string()));
    };

    let path_key = l.create_string("path")?;

    let Some(path_value) = package_table.raw_get(&path_key) else {
        return Err(l.error("'package.path' must be a string".to_string()));
    };
    let Some(path_str) = path_value.as_str() else {
        return Err(l.error("'package.path' must be a string".to_string()));
    };

    // Search for the file, using platform directory separator
    let dirsep = std::path::MAIN_SEPARATOR_STR;
    let result = search_path(modname, path_str, ".", dirsep)?;

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
    let modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("module name expected".to_string()))?;
    let filepath_val = l
        .get_arg(2)
        .ok_or_else(|| l.error("file path expected".to_string()))?;

    let Some(filepath_str) = filepath_val.as_str() else {
        return Err(l.error("file path must be a string".to_string()));
    };

    if std::fs::metadata(filepath_str).is_err() {
        return Ok(0);
    }

    // Read the file
    let source = match std::fs::read_to_string(filepath_str) {
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
    let func = vm.create_function(
        Rc::new(chunk),
        crate::lua_value::UpvalueStore::from_vec(vec![env_upvalue]),
    )?;

    // Call the function to execute the module and get its return value
    // The module should return its exports (usually a table)
    // Pass modname and filepath as arguments so the module can access them via ...
    l.push_value(func)?;
    l.push_value(modname_val)?;
    l.push_value(filepath_val)?;
    let func_idx = l.get_top() - 3;
    let (success, result_count) = l.pcall_stack_based(func_idx, 2)?;

    if !success {
        // Module threw an error
        let error_val = l.stack_get(func_idx).unwrap_or_default();
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

// Searcher 3: Search package.cpath for C modules (not supported, always returns error)
fn searcher_c(l: &mut LuaState) -> LuaResult<usize> {
    let modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("module name expected".to_string()))?;

    let Some(modname) = modname_val.as_str() else {
        return Err(l.error("module name expected".to_string()));
    };

    let package_val = get_package_from_registry(l)?;

    let Some(package_table) = package_val.as_table() else {
        return Err(l.error("Invalid package table".to_string()));
    };

    let cpath_key = l.create_string("cpath")?;

    let Some(cpath_value) = package_table.raw_get(&cpath_key) else {
        return Err(l.error("'package.cpath' must be a string".to_string()));
    };
    let Some(cpath_str) = cpath_value.as_str() else {
        return Err(l.error("'package.cpath' must be a string".to_string()));
    };

    // We don't support C modules, but we need to return proper error message
    let err = format!(
        "\n\tno file '{}'",
        cpath_str
            .split(';')
            .map(|template| { template.replace('?', &modname.replace('.', "/")) })
            .collect::<Vec<_>>()
            .join("'\n\tno file '")
    );
    let err_str = l.create_string(&err)?;
    l.push_value(err_str)?;
    Ok(1)
}

// Searcher 4: Search package.cpath for "all-in-one" C module
fn searcher_c_all_in_one(l: &mut LuaState) -> LuaResult<usize> {
    let modname_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("module name expected".to_string()))?;

    let Some(modname) = modname_val.as_str() else {
        return Err(l.error("module name expected".to_string()));
    };

    // If modname has no '.', it's a root module, return nothing
    if !modname.contains('.') {
        return Ok(0);
    }

    // For all-in-one loader, we search for the root module name
    // e.g., for "a.b.c", we search for "a"
    let root_modname = modname.split('.').next().unwrap_or(modname);

    let package_val = get_package_from_registry(l)?;

    let Some(package_table) = package_val.as_table() else {
        return Err(l.error("Invalid package table".to_string()));
    };

    let cpath_key = l.create_string("cpath")?;

    let Some(cpath_value) = package_table.raw_get(&cpath_key) else {
        return Err(l.error("'package.cpath' must be a string".to_string()));
    };
    let Some(cpath_str) = cpath_value.as_str() else {
        return Err(l.error("'package.cpath' must be a string".to_string()));
    };

    // We don't support C modules, but we need to return proper error message
    let err = format!(
        "\n\tno file '{}'",
        cpath_str
            .split(';')
            .map(|template| { template.replace('?', &root_modname.replace('.', "/")) })
            .collect::<Vec<_>>()
            .join("'\n\tno file '")
    );
    let err_str = l.create_string(&err)?;
    l.push_value(err_str)?;
    Ok(1)
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

    #[cfg(windows)]
    let default_rep = "\\";
    #[cfg(not(windows))]
    let default_rep = "/";

    let rep = if let Some(rep_val) = &rep_val {
        rep_val.as_str().unwrap_or(default_rep)
    } else {
        default_rep
    };

    match search_path(name_str, path_str, sep, rep)? {
        Some(filepath) => {
            let filepath_str = l.create_string(&filepath)?;
            l.push_value(filepath_str)?;
            Ok(1)
        }
        None => {
            let searchname = name_str.replace(sep, rep);
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
