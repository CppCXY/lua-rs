// Package library
// Implements: config, cpath, loaded, loadlib, path, preload, searchers, searchpath

use std::rc::Rc;

use crate::lib_registry::{LibraryModule, require_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

pub fn create_package_lib() -> LibraryModule {
    let mut module = LibraryModule::new("package");

    // Add functions
    module = module.with_function("loadlib", package_loadlib);
    module = module.with_function("searchpath", package_searchpath);

    // Add value fields
    module = module.with_value("loaded", create_loaded_table);
    module = module.with_value("preload", create_preload_table);
    module = module.with_value("path", create_path_string);
    module = module.with_value("cpath", create_cpath_string);
    module = module.with_value("config", create_config_string);
    module = module.with_value("searchers", create_searchers_table);

    module
}

// Create the package.loaded table
fn create_loaded_table(vm: &mut LuaVM) -> LuaValue {
    vm.create_table(0, 0)
}

// Create the package.preload table
fn create_preload_table(vm: &mut LuaVM) -> LuaValue {
    vm.create_table(0, 0)
}

// Create package.path string
fn create_path_string(vm: &mut LuaVM) -> LuaValue {
    let path = "./?.lua;./?/init.lua";
    vm.create_string(path)
}

// Create package.cpath string
fn create_cpath_string(vm: &mut LuaVM) -> LuaValue {
    let cpath = "./?.so;./?.dll;./?.dylib";
    vm.create_string(cpath)
}

// Create package.config string
fn create_config_string(vm: &mut LuaVM) -> LuaValue {
    #[cfg(windows)]
    let config = "\\\n;\n?\n!\n-";
    #[cfg(not(windows))]
    let config = "/\n;\n?\n!\n-";

    vm.create_string(config)
}

// Create package.searchers table with 4 standard searchers
fn create_searchers_table(vm: &mut LuaVM) -> LuaValue {
    let searchers = vm.create_table(4, 0);

    // Use new API: get_table_mut for mutable access
    if let Some(table_id) = searchers.as_table_id() {
        if let Some(searchers_ref) = vm.object_pool.get_table_mut(table_id) {
            searchers_ref.raw_set(LuaValue::integer(1), LuaValue::cfunction(searcher_preload));
            searchers_ref.raw_set(LuaValue::integer(2), LuaValue::cfunction(searcher_lua));
            searchers_ref.raw_set(LuaValue::integer(3), LuaValue::cfunction(searcher_c));
            searchers_ref.raw_set(LuaValue::integer(4), LuaValue::cfunction(searcher_allinone));
        }
    }

    searchers
}

// Searcher 1: Check package.preload
fn searcher_preload(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let modname_val = require_arg(vm, 1, "preload searcher")?;
    let Some(modname_id) = modname_val.as_string_id() else {
        return Err(vm.error("module name expected".to_string()));
    };
    let modname_str = {
        let Some(s) = vm.object_pool.get_string(modname_id) else {
            return Err(vm.error("module name expected".to_string()));
        };
        s.as_str().to_string()
    };

    let Some(package_table) = vm.get_global("package") else {
        return Err(vm.error("package table not found".to_string()));
    };

    let Some(package_id) = package_table.as_table_id() else {
        return Err(vm.error("Invalid package table".to_string()));
    };

    let preload_key = vm.create_string("preload");
    let preload_val = {
        let Some(pkg_table) = vm.object_pool.get_table(package_id) else {
            return Err(vm.error("Invalid package table".to_string()));
        };
        pkg_table.raw_get(&preload_key, &vm.object_pool).unwrap_or(LuaValue::nil())
    };

    let Some(preload_id) = preload_val.as_table_id() else {
        return Err(vm.error("package.preload is not a table".to_string()));
    };

    let loader = {
        let Some(preload_table) = vm.object_pool.get_table(preload_id) else {
            return Err(vm.error("package.preload is not a table".to_string()));
        };
        preload_table
            .raw_get(&modname_val, &vm.object_pool)
            .unwrap_or(LuaValue::nil())
    };

    if loader.is_nil() {
        let err = format!("\n\tno field package.preload['{}']", modname_str);
        Ok(MultiValue::single(vm.create_string(&err)))
    } else {
        Ok(MultiValue::single(loader))
    }
}

// Searcher 2: Search package.path
fn searcher_lua(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let modname_val = require_arg(vm, 1, "Lua searcher")?;
    let Some(modname_id) = modname_val.as_string_id() else {
        return Err(vm.error("module name expected".to_string()));
    };
    let modname_str = {
        let Some(s) = vm.object_pool.get_string(modname_id) else {
            return Err(vm.error("module name expected".to_string()));
        };
        s.as_str().to_string()
    };

    let Some(package_table) = vm.get_global("package") else {
        return Err(vm.error("package table not found".to_string()));
    };

    let Some(package_id) = package_table.as_table_id() else {
        return Err(vm.error("Invalid package table".to_string()));
    };

    let path_key = vm.create_string("path");
    let path_str = {
        let Some(pkg_table) = vm.object_pool.get_table(package_id) else {
            return Err(vm.error("Invalid package table".to_string()));
        };
        let Some(path_value) = pkg_table.raw_get(&path_key, &vm.object_pool) else {
            return Err(vm.error("package.path not found".to_string()));
        };
        let Some(path_id) = path_value.as_string_id() else {
            return Err(vm.error("package.path is not a string".to_string()));
        };
        let Some(path) = vm.object_pool.get_string(path_id) else {
            return Err(vm.error("package.path is not a string".to_string()));
        };
        path.as_str().to_string()
    };

    // Search for the file
    let result = search_path(&modname_str, &path_str, ".", "/")?;

    match result {
        Some(filepath) => Ok(MultiValue::multiple(vec![
            LuaValue::cfunction(lua_file_loader),
            vm.create_string(&filepath),
        ])),
        None => {
            let err = format!(
                "\n\tno file '{}'",
                path_str
                    .split(';')
                    .map(|template| { template.replace('?', &modname_str.replace('.', "/")) })
                    .collect::<Vec<_>>()
                    .join("'\n\tno file '")
            );
            Ok(MultiValue::single(vm.create_string(&err)))
        }
    }
}

// Loader function for Lua files (called by searcher_lua)
// Called as: loader(modname, filepath)
fn lua_file_loader(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // First arg is modname, second arg is filepath (passed by searcher)
    let _modname_val = require_arg(vm, 1, "Lua file loader")?;
    let filepath_val = require_arg(vm, 2, "Lua file loader")?;

    let Some(filepath_id) = filepath_val.as_string_id() else {
        return Err(vm.error("file path must be a string".to_string()));
    };
    let filepath_str = {
        let Some(s) = vm.object_pool.get_string(filepath_id) else {
            return Err(vm.error("file path must be a string".to_string()));
        };
        s.as_str().to_string()
    };

    if !std::fs::metadata(&filepath_str).is_ok() {
        return Ok(MultiValue::empty());
    }

    // Read the file
    let source = match std::fs::read_to_string(&filepath_str) {
        Ok(s) => s,
        Err(e) => {
            return Err(vm.error(format!("cannot open file '{}': {}", filepath_str, e)));
        }
    };

    // Compile it using VM's string pool with chunk name
    let chunkname = format!("@{}", filepath_str);
    let chunk = vm.compile_with_name(&source, &chunkname)?;

    // Create a function from the chunk with _ENV upvalue
    let env_upvalue_id = vm.create_upvalue_closed(LuaValue::table(vm.global));
    let func = vm.create_function(Rc::new(chunk), vec![env_upvalue_id]);

    // Call the function to execute the module
    let (success, results) = vm.protected_call(func, vec![])?;

    if !success {
        let error_msg = results
            .first()
            .and_then(|v| v.as_string_id())
            .and_then(|id| vm.object_pool.get_string(id))
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "unknown error".to_string());
        return Err(vm.error(format!(
            "error loading module '{}': {}",
            filepath_str, error_msg
        )));
    }

    // Get the result value - if the module returns a value, use it
    // Otherwise return true (standard Lua behavior)
    let module_value = if results.is_empty() || results[0].is_nil() {
        LuaValue::boolean(true)
    } else {
        results[0].clone()
    };

    Ok(MultiValue::single(module_value))
}

// Searcher 3: Search package.cpath (C libraries)
fn searcher_c(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    return Ok(MultiValue::single(LuaValue::nil()));
}

// Searcher 4: all-in-one loader (stub)
fn searcher_allinone(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    return Ok(MultiValue::single(LuaValue::nil()));
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

fn package_loadlib(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let err = vm.create_string("loadlib not implemented");
    Ok(MultiValue::multiple(vec![LuaValue::nil(), err]))
}

fn package_searchpath(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let name_val = require_arg(vm, 1, "searchpath")?;
    let path_val = require_arg(vm, 2, "searchpath")?;

    let Some(name_id) = name_val.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'searchpath' (string expected)".to_string()));
    };
    let name_str = {
        let Some(s) = vm.object_pool.get_string(name_id) else {
            return Err(vm.error("bad argument #1 to 'searchpath' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    let Some(path_id) = path_val.as_string_id() else {
        return Err(vm.error("bad argument #2 to 'searchpath' (string expected)".to_string()));
    };
    let path_str = {
        let Some(s) = vm.object_pool.get_string(path_id) else {
            return Err(vm.error("bad argument #2 to 'searchpath' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    match search_path(&name_str, &path_str, ".", "/")? {
        Some(filepath) => Ok(MultiValue::single(vm.create_string(&filepath))),
        None => {
            let searchname = name_str.replace(".", "/");
            let err = format!(
                "\n\tno file '{}'",
                path_str
                    .split(';')
                    .map(|template| { template.replace('?', &searchname) })
                    .collect::<Vec<_>>()
                    .join("'\n\tno file '")
            );
            Ok(MultiValue::multiple(vec![
                LuaValue::nil(),
                vm.create_string(&err),
            ]))
        }
    }
}
