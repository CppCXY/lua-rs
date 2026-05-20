// Tests for package library and module system
use crate::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn luaopen_test_install_module(l: &mut LuaState) -> LuaResult<usize> {
    let table = l.create_table(0, 1)?;
    let key = l.create_string("value")?;
    l.global_state_mut()
        .raw_set(&table, key, LuaValue::integer(42));
    l.push_value(table)?;
    Ok(1)
}

#[test]
fn test_package_loaded() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        assert(type(package.loaded) == "table")
        assert(package.loaded.string ~= nil)
        assert(package.loaded.table ~= nil)
        assert(package.loaded.math ~= nil)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_package_preload() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        assert(type(package.preload) == "table")
        
        package.preload['testmod'] = function()
            return {value = 42}
        end
        
        local mod = require('testmod')
        assert(mod.value == 42)
    "#,
    );

    if let Err(e) = &result {
        let error_msg = vm.main_state().get_error_message(*e);
        eprintln!("Error: {:?}, Message: {}", e, error_msg);
    }
    assert!(result.is_ok());
}

#[test]
fn test_package_path() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        assert(type(package.path) == "string")
        assert(#package.path > 0)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_package_cpath() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        assert(type(package.cpath) == "string")
        assert(#package.cpath > 0)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_package_config() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        assert(type(package.config) == "string")
        local lines = 0
        for line in package.config:gmatch("[^\n]+") do
            lines = lines + 1
        end
        assert(lines == 5)
    "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_package_searchers() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        assert(type(package.searchers) == "table")
        assert(type(package.searchers[1]) == "function")  -- preload searcher
        assert(type(package.searchers[2]) == "function")  -- lua file searcher
        assert(type(package.searchers[3]) == "function")  -- C module searcher
        assert(type(package.searchers[4]) == "function")  -- all-in-one C searcher
        assert(package.searchers[5] == nil)  -- we have 4 searchers total
    "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_package_searchpath() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local path, err = package.searchpath("string", package.path)
        -- Either finds a file or returns error message
        assert(path ~= nil or err ~= nil)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_require_preload() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        package.preload['mymodule'] = function()
            local M = {}
            M.name = "MyModule"
            M.version = "1.0"
            function M.hello()
                return "Hello!"
            end
            return M
        end
        
        local mod = require('mymodule')
        assert(mod.name == "MyModule")
        assert(mod.version == "1.0")
        assert(mod.hello() == "Hello!")
    "#,
    );

    if let Err(e) = &result {
        let error_msg = vm.main_state().get_error_message(*e);
        panic!("Error: {:?}, Message: {}", e, error_msg);
    }
    assert!(result.is_ok());
}

#[test]
fn test_require_cache() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local load_count = 0
        package.preload['cached'] = function()
            load_count = load_count + 1
            return {count = load_count}
        end
        
        local m1 = require('cached')
        local m2 = require('cached')
        
        assert(m1 == m2)
        assert(load_count == 1)
        assert(package.loaded['cached'] == m1)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_require_error() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local ok, err = pcall(require, 'nonexistent_module_xyz')
        assert(ok == false)
        assert(type(err) == "string")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_require_missing_module_reports_call_site_file_and_line() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "luars-require-missing-{}-{}.lua",
        std::process::id(),
        unique
    ));
    std::fs::write(&path, "require \"definitely_missing_module_xyz\"\n").unwrap();

    let err = vm.main_state().dofile(path.to_str().unwrap()).unwrap_err();
    let message = vm.main_state().get_error_message(err);
    let filename = path.file_name().unwrap().to_string_lossy();

    assert!(message.contains(&format!("{}:1:", filename)), "{message}");
    assert!(message.contains("module 'definitely_missing_module_xyz' not found"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_require_return_value() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        -- Module returning nil should store true
        package.preload['nilmod'] = function()
            return nil
        end
        
        local m = require('nilmod')
        assert(m == true)
        assert(package.loaded['nilmod'] == true)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_install_library_supports_library_module() {
    let mut lua = crate::Lua::new(SafeOption::default());
    lua.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let module = crate::lua_module!("hostlib", {
        "answer" => |l| {
            l.push_value(LuaValue::integer(42))?;
            Ok(1)
        }
    });

    lua.install_library(module).unwrap();

    let result = lua.execute(
        r#"
        assert(type(hostlib) == "table")
        assert(hostlib.answer() == 42)
    "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_install_library_supports_preload_modules() {
    let mut lua = crate::Lua::new(SafeOption::default());
    lua.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    lua.install_library(
        crate::lua_preload_module!("test_install_module" => luaopen_test_install_module),
    )
    .unwrap();

    let result = lua.execute(
        r#"
        local mod = require('test_install_module')
        assert(type(mod) == "table")
        assert(mod.value == 42)
    "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result.err());
}
