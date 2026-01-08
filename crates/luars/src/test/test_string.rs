// Tests for string library functions
use crate::*;

#[test]
fn test_string_len() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(string.len("hello") == 5)
        assert(string.len("") == 0)
        assert(#"hello" == 5)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_sub() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(string.sub("hello", 2, 4) == "ell")
        assert(string.sub("hello", 2) == "ello")
        assert(string.sub("hello", -2) == "lo")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_upper_lower() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(string.upper("hello") == "HELLO")
        assert(string.lower("WORLD") == "world")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_rep() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(string.rep("ab", 3) == "ababab")
        assert(string.rep("x", 0) == "")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_reverse() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(string.reverse("hello") == "olleh")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_byte_char() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(string.byte("A") == 65)
        assert(string.char(65) == "A")
        assert(string.char(65, 66, 67) == "ABC")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_format() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(string.format("%d", 42) == "42")
        assert(string.format("%s", "hello") == "hello")
        assert(string.format("%d %s", 10, "test") == "10 test")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_find() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local i, j = string.find("hello world", "world")
        assert(i == 7)
        assert(j == 11)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_match() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local m = string.match("hello 123", "%d+")
        assert(m == "123")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_gmatch() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local words = {}
        for w in string.gmatch("one two three", "%w+") do
            table.insert(words, w)
        end
        assert(words[1] == "one")
        assert(words[2] == "two")
        assert(words[3] == "three")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_gsub() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local s, n = string.gsub("hello world", "l", "L")
        assert(s == "heLLo worLd")
        assert(n == 3)
        
        local s2, n2 = string.gsub("hello", "l", "L", 1)
        assert(s2 == "heLlo")
        assert(n2 == 1)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_pack_unpack() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local packed = string.pack("bhi", 127, 32767, 2147483647)
        assert(type(packed) == "string")
        
        local b, h, i = string.unpack("bhi", packed)
        assert(b == 127)
        assert(h == 32767)
        assert(i == 2147483647)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_string_packsize() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(string.packsize("b") == 1)
        assert(string.packsize("h") == 2)
        assert(string.packsize("i") == 4)
        assert(string.packsize("bhi") == 7)
    "#,
    );

    assert!(result.is_ok());
}
