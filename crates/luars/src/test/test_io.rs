// Tests for I/O library functions
use crate::*;
use std::env;

// Helper to get the test data directory path
fn get_test_data_dir() -> String {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    format!("{}/src/test/test_data", manifest_dir).replace("\\", "/")
}

#[test]
fn test_io_open_read() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/sample.txt", "r")
        assert(f ~= nil, "Failed to open file")
        local content = f:read("*a")
        f:close()
        assert(content:find("Hello, World!") ~= nil)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_open_nonexistent() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local f, err = io.open("nonexistent_file_12345.txt", "r")
        assert(f == nil)
        assert(err ~= nil)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_lines_file() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local lines = {{}}
        for line in io.lines("{}/lines.txt") do
            table.insert(lines, line)
        end
        assert(#lines == 5)
        assert(lines[1] == "line1")
        assert(lines[5] == "line5")
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_read_line() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/sample.txt", "r")
        local line1 = f:read("*l")
        local line2 = f:read("*l")
        f:close()
        assert(line1 == "Hello, World!")
        assert(line2 == "This is line 2.")
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_read_number() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/binary.dat", "r")
        local n = f:read("*n")
        f:close()
        -- Should read the number at the start
        assert(type(n) == "number" or n == nil)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_read_bytes() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/binary.dat", "r")
        local bytes = f:read(4)
        f:close()
        assert(bytes == "0123")
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_write_temp() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local path = "{}/temp_write.txt"
        local f = io.open(path, "w")
        assert(f ~= nil)
        f:write("Test write")
        f:close()
        
        local f2 = io.open(path, "r")
        local content = f2:read("*a")
        f2:close()
        assert(content == "Test write")
        
        os.remove(path)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_seek_operations() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/binary.dat", "r")
        
        -- seek to position 5
        local pos = f:seek("set", 5)
        assert(pos == 5)
        
        -- read from position 5
        local char = f:read(1)
        assert(char == "5")
        
        -- seek relative
        f:seek("cur", 2)
        char = f:read(1)
        assert(char == "8")
        
        -- seek to end
        local size = f:seek("end", 0)
        assert(size == 16)
        
        f:close()
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_type_function() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/sample.txt", "r")
        assert(io.type(f) == "file")
        f:close()
        assert(io.type(f) == "closed file")
        assert(io.type("not a file") == nil)
        assert(io.type(123) == nil)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_flush() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local path = "{}/temp_flush.txt"
        local f = io.open(path, "w")
        f:write("flush test")
        f:flush()
        f:close()
        os.remove(path)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_tmpfile() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local f = io.tmpfile()
        if f then
            f:write("temp content")
            f:seek("set", 0)
            local content = f:read("*a")
            assert(content == "temp content")
            f:close()
        end
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_read_all() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/sample.txt", "r")
        local all = f:read("*a")
        f:close()
        assert(#all > 0)
        assert(all:find("End of file") ~= nil)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_file_setvbuf() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local path = "{}/temp_buf.txt"
        local f = io.open(path, "w")
        -- set buffering mode
        f:setvbuf("full", 1024)
        f:write("buffered")
        f:close()
        os.remove(path)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_multiple_reads() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/binary.dat", "r")
        local a = f:read(2)
        local b = f:read(2)
        local c = f:read(2)
        f:close()
        assert(a == "01")
        assert(b == "23")
        assert(c == "45")
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_append_mode() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local path = "{}/temp_append.txt"
        
        -- Write initial content
        local f = io.open(path, "w")
        f:write("first")
        f:close()
        
        -- Append
        f = io.open(path, "a")
        f:write("second")
        f:close()
        
        -- Verify
        f = io.open(path, "r")
        local content = f:read("*a")
        f:close()
        assert(content == "firstsecond")
        
        os.remove(path)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}

#[test]
fn test_io_read_eof() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let test_dir = get_test_data_dir();

    let result = vm.execute(&format!(
        r#"
        local f = io.open("{}/binary.dat", "r")
        -- Read all content
        local all = f:read("*a")
        -- Try to read more - should return empty string or nil
        local more = f:read("*a")
        f:close()
        assert(all == "0123456789ABCDEF")
        assert(more == "" or more == nil)
        "#,
        test_dir
    ));

    assert!(result.is_ok(), "Error: {:?}", result);
}
