// Package library
// Implements: config, cpath, loaded, loadlib, path, preload, searchers, searchpath

use crate::lib_registry::{LibraryModule, get_arg, require_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;

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
    let loaded = vm.create_table();
    LuaValue::from_table_rc(loaded)
}

// Create the package.preload table
fn create_preload_table(vm: &mut LuaVM) -> LuaValue {
    let preload = vm.create_table();
    LuaValue::from_table_rc(preload)
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
    let searchers = vm.create_table();

    searchers
        .borrow_mut()
        .raw_set(LuaValue::integer(1), LuaValue::cfunction(searcher_preload));
    searchers
        .borrow_mut()
        .raw_set(LuaValue::integer(2), LuaValue::cfunction(searcher_lua));
    searchers
        .borrow_mut()
        .raw_set(LuaValue::integer(3), LuaValue::cfunction(searcher_c));
    searchers
        .borrow_mut()
        .raw_set(LuaValue::integer(4), LuaValue::cfunction(searcher_allinone));

    LuaValue::from_table_rc(searchers)
}

// Searcher 1: Check package.preload
fn searcher_preload(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let modname_val = require_arg(vm, 0, "preload searcher")?;
    let modname = unsafe {
        modname_val
            .as_string()
            .ok_or_else(|| "module name expected".to_string())?
            .as_str()
            .to_string()
    };

    let package_table = vm
        .get_global("package")
        .ok_or_else(|| "package table not found".to_string())?;

    let package_rc = package_table
        .as_table()
        .ok_or_else(|| "package is not a table".to_string())?;

    let preload_val = package_rc
        .borrow()
        .raw_get(&vm.create_string("preload"))
        .unwrap_or(LuaValue::nil());

    let preload_table = match preload_val.as_table() {
        Some(t) => t,
        None => {
            let err = format!("\n\tno field package.preload['{}']", modname);
            return Ok(MultiValue::single(vm.create_string(&err)));
        }
    };

    let modname_key = vm.create_string(&modname);
    let loader = preload_table
        .borrow()
        .raw_get(&modname_key)
        .unwrap_or(LuaValue::nil());

    if loader.is_nil() {
        let err = format!("\n\tno field package.preload['{}']", modname);
        Ok(MultiValue::single(vm.create_string(&err)))
    } else {
        Ok(MultiValue::single(loader))
    }
}

// Searcher 2: Search package.path
fn searcher_lua(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let modname_val = require_arg(vm, 0, "Lua searcher")?;
    let modname = unsafe {
        modname_val
            .as_string()
            .ok_or_else(|| "module name expected".to_string())?
            .as_str()
            .to_string()
    };

    let path = unsafe {
        let package_table = vm
            .get_global("package")
            .ok_or_else(|| "package table not found".to_string())?;

        let package_rc = package_table
            .as_table()
            .ok_or_else(|| "package is not a table".to_string())?;

        let path_val = package_rc
            .borrow()
            .raw_get(&vm.create_string("path"))
            .unwrap_or(LuaValue::nil());

        path_val
            .as_string()
            .ok_or_else(|| "package.path is not a string".to_string())?
            .as_str()
            .to_string()
    };

    // Search for the file
    let result = search_path(&modname, &path, ".", "/")?;

    match result {
        Some(filepath) => Ok(MultiValue::multiple(vec![
            LuaValue::cfunction(lua_file_loader),
            vm.create_string(&filepath),
        ])),
        None => {
            let err = format!(
                "\n\tno file '{}'",
                path.split(';')
                    .map(|template| { template.replace('?', &modname.replace('.', "/")) })
                    .collect::<Vec<_>>()
                    .join("'\n\tno file '")
            );
            Ok(MultiValue::single(vm.create_string(&err)))
        }
    }
}

// Loader function for Lua files (called by searcher_lua)
fn lua_file_loader(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let _modname = get_arg(vm, 0);
    let filepath_val = require_arg(vm, 1, "Lua file loader")?;

    let filepath = unsafe {
        filepath_val
            .as_string()
            .ok_or_else(|| "filepath expected".to_string())?
            .as_str()
            .to_string()
    };

    // Read the file
    let source = std::fs::read_to_string(&filepath)
        .map_err(|e| format!("cannot read file '{}': {}", filepath, e))?;

    // Compile it
    let chunk = crate::Compiler::compile(&source)
        .map_err(|e| format!("error loading module '{}': {}", filepath, e))?;

    // Create a function from the chunk
    let func = LuaValue::from_function_rc(std::rc::Rc::new(crate::LuaFunction {
        chunk: std::rc::Rc::new(chunk),
        upvalues: vec![],
    }));

    // Call the function
    let (success, results) = vm.protected_call(func, vec![]);

    if !success {
        let error_msg = unsafe {
            results
                .first()
                .and_then(|v| v.as_string())
                .map(|s| s.as_str().to_string())
                .unwrap_or_else(|| "unknown error".to_string())
        };
        return Err(format!(
            "error loading module '{}': {}",
            filepath, error_msg
        ));
    }

    // Get the result value
    let module_value = if results.is_empty() || results[0].is_nil() {
        LuaValue::boolean(true)
    } else {
        results[0].clone()
    };

    Ok(MultiValue::single(module_value))
}

// Searcher 3: Search package.cpath (C libraries)
fn searcher_c(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let modname_val = require_arg(vm, 0, "C searcher")?;
    let modname = unsafe {
        modname_val
            .as_string()
            .ok_or_else(|| "module name expected".to_string())?
            .as_str()
            .to_string()
    };

    let cpath = unsafe {
        let package_table = vm
            .get_global("package")
            .ok_or_else(|| "package table not found".to_string())?;

        let package_rc = package_table
            .as_table()
            .ok_or_else(|| "package is not a table".to_string())?;

        let cpath_val = package_rc
            .borrow()
            .raw_get(&vm.create_string("cpath"))
            .unwrap_or(LuaValue::nil());

        cpath_val
            .as_string()
            .ok_or_else(|| "package.cpath is not a string".to_string())?
            .as_str()
            .to_string()
    };

    // For now, just return error message (C loader not implemented)
    let err = format!(
        "\n\tC loader not implemented\n\tno file '{}'",
        cpath
            .split(';')
            .map(|template| { template.replace('?', &modname.replace('.', "/")) })
            .collect::<Vec<_>>()
            .join("'\n\tno file '")
    );
    Ok(MultiValue::single(vm.create_string(&err)))
}

// Searcher 4: all-in-one loader (stub)
fn searcher_allinone(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let modname_val = require_arg(vm, 0, "all-in-one searcher")?;
    let modname = unsafe {
        modname_val
            .as_string()
            .ok_or_else(|| "module name expected".to_string())?
            .as_str()
            .to_string()
    };

    // Only try if this is a submodule (contains '.')
    if !modname.contains('.') {
        let err = format!("\n\tno module '{}' in all-in-one loader", modname);
        return Ok(MultiValue::single(vm.create_string(&err)));
    }

    let err = format!("\n\tall-in-one loader not fully implemented");
    Ok(MultiValue::single(vm.create_string(&err)))
}

// Helper: Search for a file in path templates
fn search_path(name: &str, path: &str, sep: &str, rep: &str) -> Result<Option<String>, String> {
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

fn package_loadlib(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let err = vm.create_string("loadlib not implemented");
    Ok(MultiValue::multiple(vec![LuaValue::nil(), err]))
}

fn package_searchpath(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let name_val = require_arg(vm, 0, "searchpath")?;
    let path_val = require_arg(vm, 1, "searchpath")?;
    let sep_val = get_arg(vm, 2).unwrap_or(vm.create_string("."));
    let rep_val = get_arg(vm, 3).unwrap_or(vm.create_string("/"));

    let name = unsafe {
        name_val
            .as_string()
            .ok_or_else(|| "bad argument #1 to 'searchpath' (string expected)".to_string())?
            .as_str()
            .to_string()
    };

    let path = unsafe {
        path_val
            .as_string()
            .ok_or_else(|| "bad argument #2 to 'searchpath' (string expected)".to_string())?
            .as_str()
            .to_string()
    };

    let sep = unsafe {
        sep_val
            .as_string()
            .ok_or_else(|| "bad argument #3 to 'searchpath' (string expected)".to_string())?
            .as_str()
            .to_string()
    };

    let rep = unsafe {
        rep_val
            .as_string()
            .ok_or_else(|| "bad argument #4 to 'searchpath' (string expected)".to_string())?
            .as_str()
            .to_string()
    };

    match search_path(&name, &path, &sep, &rep)? {
        Some(filepath) => Ok(MultiValue::single(vm.create_string(&filepath))),
        None => {
            let searchname = name.replace(&sep, &rep);
            let err = format!(
                "\n\tno file '{}'",
                path.split(';')
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
