// Tests for basic library functions
use crate::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn test_print() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Successful assertion
    let result = vm.main_state().execute(
        r#"
        local a, b, c = assert(true, "test", 123)
        assert(a == true)
        assert(b == "test")
        assert(c == 123)
    "#,
    );
    assert!(result.is_ok());

    // Failed assertion
    let result = vm.main_state().execute(
        r#"
        assert(false, "This should fail")
    "#,
    );
    assert!(result.is_err());
}

#[test]
fn test_error() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        error("Custom error message")
    "#,
    );

    assert!(result.is_err());
}

#[test]
fn test_pcall() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local f = load("return 10 + 20")
        assert(type(f) == "function")
        assert(f() == 30)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_string_dump_load_binary_constant() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local src = function()
            return string.char(255, 0, 65), 42
        end

        local dumped = string.dump(src)
        local restored = assert(load(dumped))
        local payload, n = restored()

        assert(n == 42)
        assert(#payload == 3)
        assert(string.byte(payload, 1) == 255)
        assert(string.byte(payload, 2) == 0)
        assert(string.byte(payload, 3) == 65)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_load_rejects_binary_when_bytecode_loading_disabled() {
    let option = SafeOption {
        allow_load_bytecode: false,
        ..Default::default()
    };

    let mut vm = GlobalState::new(option);
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local src = function() return 42 end
        local dumped = string.dump(src)
        local restored, err = load(dumped)

        assert(restored == nil)
        assert(type(err) == "string")
        assert(string.find(err, "bytecode loading is disabled", 1, true) ~= nil)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_dofile_rejects_binary_when_bytecode_loading_disabled() {
    let mut builder_vm = GlobalState::new(SafeOption::default());
    let chunk = builder_vm.main_state().compile_chunk("return 42").unwrap();
    let bytes = serialize_chunk(&chunk, false).unwrap();

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "luars-bytecode-disabled-{}-{}.luac",
        std::process::id(),
        unique
    ));
    std::fs::write(&path, bytes).unwrap();
    let option = SafeOption {
        allow_load_bytecode: false,
        ..Default::default()
    };

    let mut vm = GlobalState::new(option);

    let err = vm.main_state().dofile(path.to_str().unwrap()).unwrap_err();
    let message = vm.main_state().get_error_message(err);
    assert!(message.contains("bytecode loading is disabled"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_warn() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        warn("This is a warning")
        warn("Multiple", " ", "parts")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_collectgarbage() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        collectgarbage("collect")
        collectgarbage("count")
    "#,
    );

    assert!(result.is_ok());
}
