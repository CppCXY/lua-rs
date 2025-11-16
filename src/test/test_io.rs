// Tests for I/O library functions
use crate::*;

#[test]
fn test_io_write_read() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local f = io.open("test_io.txt", "w")
        assert(f ~= nil)
        f:write("Hello, World!\n")
        f:write("Line 2\n")
        f:close()
        
        local f2 = io.open("test_io.txt", "r")
        local content = f2:read("*a")
        f2:close()
        
        assert(type(content) == "string")
        os.remove("test_io.txt")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_io_lines() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local f = io.open("test_lines.txt", "w")
        f:write("line1\n")
        f:write("line2\n")
        f:write("line3\n")
        f:close()
        
        local lines = {}
        for line in io.lines("test_lines.txt") do
            table.insert(lines, line)
        end
        
        assert(#lines == 3)
        assert(lines[1] == "line1")
        assert(lines[2] == "line2")
        assert(lines[3] == "line3")
        
        os.remove("test_lines.txt")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_file_read_modes() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local f = io.open("test_read.txt", "w")
        f:write("12345\n67890")
        f:close()
        
        local f2 = io.open("test_read.txt", "r")
        local line = f2:read("*l")
        assert(line == "12345")
        f2:close()
        
        os.remove("test_read.txt")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_file_seek() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local f = io.open("test_seek.txt", "w+")
        f:write("0123456789")
        
        local pos = f:seek("set", 5)
        assert(pos == 5)
        
        f:write("X")
        f:seek("set", 0)
        local content = f:read("*a")
        assert(content == "01234X6789")
        
        f:close()
        os.remove("test_seek.txt")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_file_flush() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local f = io.open("test_flush.txt", "w")
        f:write("test")
        f:flush()
        f:close()
        os.remove("test_flush.txt")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_io_type() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local f = io.open("test_type.txt", "w")
        assert(io.type(f) == "file")
        f:close()
        assert(io.type(f) == "closed file")
        os.remove("test_type.txt")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_io_tmpfile() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local f = io.tmpfile()
        assert(f ~= nil)
        f:write("temporary data")
        f:seek("set", 0)
        local content = f:read("*a")
        assert(content == "temporary data")
        f:close()
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_io_input_output() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        io.output("test_stdout.txt")
        io.write("test output\n")
        io.close()
        
        io.input("test_stdout.txt")
        local line = io.read("*l")
        assert(line == "test output")
        io.close()
        
        os.remove("test_stdout.txt")
    "#,
    );

    assert!(result.is_ok());
}
