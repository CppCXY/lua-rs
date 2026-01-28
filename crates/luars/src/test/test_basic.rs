// Tests for basic library functions
use crate::*;

#[test]
fn test_print() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute_string(
        r#"
        print("Hello, World!")
        print(1, 2, 3)
        print()
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_type() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(type(nil) == "nil")
        assert(type(true) == "boolean")
        assert(type(42) == "number")
        assert(type(3.14) == "number")
        assert(type("hello") == "string")
        assert(type({}) == "table")
        assert(type(print) == "function")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_tonumber() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(tonumber("123") == 123)
        assert(tonumber("3.14") == 3.14)
        assert(tonumber("FF", 16) == 255)
        assert(tonumber("invalid") == nil)
        assert(tonumber(42) == 42)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_tostring() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(tostring(123) == "123")
        assert(tostring(true) == "true")
        assert(tostring(nil) == "nil")
        local s = tostring({})
        assert(type(s) == "string")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_assert() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Successful assertion
    let result = vm.execute_string(
        r#"
        local a, b, c = assert(true, "test", 123)
        assert(a == true)
        assert(b == "test")
        assert(c == 123)
    "#,
    );
    assert!(result.is_ok());

    // Failed assertion
    let result = vm.execute_string(
        r#"
        assert(false, "This should fail")
    "#,
    );
    assert!(result.is_err());
}

#[test]
fn test_error() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        error("Custom error message")
    "#,
    );

    assert!(result.is_err());
}

#[test]
fn test_pcall() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        -- Successful call
        local ok, result = pcall(function() return 42 end)
        assert(ok == true)
        assert(result == 42)
        
        -- Failed call
        local ok, err = pcall(function() error("test error") end)
        assert(ok == false)
        assert(type(err) == "string")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_xpcall() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local handler_called = false
        local function handler(err)
            handler_called = true
            return "handled: " .. tostring(err)
        end
        
        local ok, result = xpcall(function()
            error("test error")
        end, handler)
        
        assert(ok == false)
        assert(handler_called == true)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_select() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r##"
        assert(select("#", 1, 2, 3) == 3)
        local a, b = select(2, "a", "b", "c")
        assert(a == "b")
        assert(b == "c")
    "##,
    );

    assert!(result.is_ok());
}

#[test]
fn test_ipairs() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {10, 20, 30}
        local sum = 0
        for i, v in ipairs(t) do
            sum = sum + v
        end
        assert(sum == 60)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("test_ipairs error: {:?}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_pairs() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {a = 1, b = 2, c = 3}
        local count = 0
        for k, v in pairs(t) do
            count = count + 1
        end
        assert(count == 3)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_next() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {a = 1, b = 2}
        local k1, v1 = next(t, nil)
        assert(k1 ~= nil)
        assert(v1 ~= nil)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_rawget_rawset() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {}
        rawset(t, "key", "value")
        assert(rawget(t, "key") == "value")
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_rawlen() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(rawlen("hello") == 5)
        assert(rawlen({1,2,3}) == 3)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_rawequal() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(rawequal(1, 1) == true)
        assert(rawequal(1, 2) == false)
        local t1 = {}
        local t2 = {}
        assert(rawequal(t1, t1) == true)
        assert(rawequal(t1, t2) == false)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_getmetatable_setmetatable() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {}
        local mt = {__index = function() return 42 end}
        setmetatable(t, mt)
        assert(getmetatable(t) == mt)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_load() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local f = load("return 10 + 20")
        assert(type(f) == "function")
        assert(f() == 30)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_warn() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        warn("This is a warning")
        warn("Multiple", " ", "parts")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_collectgarbage() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        collectgarbage("collect")
        collectgarbage("count")
    "#,
    );

    assert!(result.is_ok());
}
