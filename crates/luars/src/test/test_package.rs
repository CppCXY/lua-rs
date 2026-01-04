// Tests for package library and module system
use crate::*;

#[test]
fn test_package_loaded() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
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
        let error_msg = vm.get_error_message();
        eprintln!("Error: {:?}, Message: {}", e, error_msg);
    }
    assert!(result.is_ok());
}

#[test]
fn test_package_path() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(type(package.path) == "string")
        assert(#package.path > 0)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_package_cpath() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(type(package.cpath) == "string")
        assert(#package.cpath > 0)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_package_config() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(type(package.searchers) == "table")
        assert(type(package.searchers[1]) == "function")  -- preload searcher
        assert(type(package.searchers[2]) == "function")  -- lua file searcher
        assert(package.searchers[3] == nil)  -- we only have 2 searchers
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_package_searchpath() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
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
        let error_msg = vm.get_error_message();
        panic!("Error: {:?}, Message: {}", e, error_msg);
    }
    assert!(result.is_ok());
}

#[test]
fn test_require_cache() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local ok, err = pcall(require, 'nonexistent_module_xyz')
        assert(ok == false)
        assert(type(err) == "string")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_require_return_value() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
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
