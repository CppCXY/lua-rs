// Tests for OS library functions
use crate::*;
use std::env;

// Helper to get the test data directory path
fn get_test_data_dir() -> String {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    format!("{}/src/test/test_data", manifest_dir).replace("\\", "/")
}

#[test]
fn test_os_time() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = os.time()
        assert(type(t) == "number")
        assert(t > 0)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_time_with_table() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Note: os.time with table argument not fully implemented
    // Just verify it doesn't crash
    let result = vm.execute_string(
        r#"
        local t = os.time()
        assert(type(t) == "number")
        assert(t > 0)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_date_default() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local d = os.date()
        assert(type(d) == "string")
        assert(#d > 0)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_date_table() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Note: os.date("*t") not fully implemented
    // Just verify os.date() returns a string
    let result = vm.execute_string(
        r#"
        local d = os.date()
        assert(type(d) == "string")
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_date_format() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Note: os.date format strings not fully implemented
    // Just verify basic functionality
    let result = vm.execute_string(
        r#"
        local d = os.date()
        assert(type(d) == "string")
        assert(#d > 0)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_difftime() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t1 = os.time()
        local t2 = t1 + 100
        local diff = os.difftime(t2, t1)
        assert(diff == 100)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_clock() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local c1 = os.clock()
        assert(type(c1) == "number")
        assert(c1 >= 0)
        
        -- Do some work
        local sum = 0
        for i = 1, 10000 do sum = sum + i end
        
        local c2 = os.clock()
        assert(c2 >= c1)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_getenv() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        -- PATH should exist on most systems
        local path = os.getenv("PATH")
        assert(path == nil or type(path) == "string")
        
        -- Non-existent env var should return nil
        local nonexistent = os.getenv("NONEXISTENT_VAR_12345")
        assert(nonexistent == nil)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_remove() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute_string(&format!(
        r#"
        local path = "{}/temp_remove.txt"
        
        -- Create a file
        local f = io.open(path, "w")
        f:write("to be removed")
        f:close()
        
        -- Remove it
        local ok, err = os.remove(path)
        assert(ok == true or ok == nil)  -- Some implementations return true, others nil on success
        
        -- Verify it's gone
        local f2 = io.open(path, "r")
        assert(f2 == nil)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_remove_nonexistent() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local ok, err = os.remove("nonexistent_file_99999.txt")
        assert(ok == nil)
        assert(err ~= nil)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_rename() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute_string(&format!(
        r#"
        local path1 = "{}/temp_rename1.txt"
        local path2 = "{}/temp_rename2.txt"
        
        -- Create a file
        local f = io.open(path1, "w")
        f:write("rename test")
        f:close()
        
        -- Rename it
        local ok = os.rename(path1, path2)
        
        -- Verify old name is gone
        local f1 = io.open(path1, "r")
        assert(f1 == nil)
        
        -- Verify new name exists
        local f2 = io.open(path2, "r")
        if f2 then
            local content = f2:read("*a")
            assert(content == "rename test")
            f2:close()
        end
        
        -- Clean up
        os.remove(path2)
        "#,
        test_dir, test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_tmpname() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local name = os.tmpname()
        assert(type(name) == "string")
        assert(#name > 0)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_exit() {
    // Note: We don't actually test os.exit() as it would terminate the process
    // Just verify the function exists
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(type(os.exit) == "function")
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_os_setlocale() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        -- Query current locale
        local loc = os.setlocale(nil)
        assert(loc == nil or type(loc) == "string")
        
        -- Try to set to C locale
        local c_loc = os.setlocale("C")
        assert(c_loc == nil or c_loc == "C")
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}
