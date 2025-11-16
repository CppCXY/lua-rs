// Tests for UTF-8 library functions
use crate::*;

#[test]
fn test_utf8_len() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        assert(utf8.len("hello") == 5)
        assert(utf8.len("") == 0)
        assert(utf8.len("世界") == 2)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_utf8_char() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        assert(utf8.char(65) == "A")
        assert(utf8.char(65, 66, 67) == "ABC")
        assert(utf8.char(0x4E16, 0x754C) == "世界")
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_utf8_codes() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local s = "ABC"
        local codes = {}
        for p, c in utf8.codes(s) do
            table.insert(codes, c)
        end
        assert(codes[1] == 65)
        assert(codes[2] == 66)
        assert(codes[3] == 67)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_utf8_codepoint() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        assert(utf8.codepoint("A") == 65)
        local a, b, c = utf8.codepoint("ABC", 1, 3)
        assert(a == 65)
        assert(b == 66)
        assert(c == 67)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_utf8_offset() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local s = "hello"
        assert(utf8.offset(s, 2) == 2)
        assert(utf8.offset(s, 5) == 5)
        assert(utf8.offset(s, -1) == 5)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_utf8_charpattern() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        assert(type(utf8.charpattern) == "string")
        assert(#utf8.charpattern > 0)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_utf8_multibyte() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local s = "Hello 世界"
        local len = utf8.len(s)
        assert(len == 8)
        
        local count = 0
        for p, c in utf8.codes(s) do
            count = count + 1
        end
        assert(count == 8)
    "#);
    
    assert!(result.is_ok());
}
