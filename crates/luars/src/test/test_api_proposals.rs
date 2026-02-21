// Tests for API improvement proposals (P1–P11)
use crate::*;

// ============================
// P1: call / call_global
// ============================

#[test]
fn test_call_lua_function() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.execute("function add(a, b) return a + b end").unwrap();
    let func = vm.get_global("add").unwrap().unwrap();
    let results = vm
        .call(func, vec![LuaValue::integer(3), LuaValue::integer(4)])
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(7));
}

#[test]
fn test_call_global() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.execute("function greet(name) return 'Hello, ' .. name end")
        .unwrap();
    let name = vm.create_string("World").unwrap();
    let results = vm.call_global("greet", vec![name]).unwrap();
    assert_eq!(results[0].as_str(), Some("Hello, World"));
}

#[test]
fn test_call_global_not_found() {
    let mut vm = LuaVM::new(SafeOption::default());
    let result = vm.call_global("nonexistent", vec![]);
    assert!(result.is_err());
}

// ============================
// P2: register_function
// ============================

#[test]
fn test_register_function() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.register_function("rust_add", |state| {
        let a = state.get_arg(1).and_then(|v| v.as_integer()).unwrap_or(0);
        let b = state.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(0);
        state.push_value(LuaValue::integer(a + b))?;
        Ok(1)
    })
    .unwrap();

    let results = vm.execute("return rust_add(10, 20)").unwrap();
    assert_eq!(results[0].as_integer(), Some(30));
}

// ============================
// P3: load / load_with_name
// ============================

#[test]
fn test_load_and_call() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let func = vm.load("return 42").unwrap();
    let results = vm.call(func, vec![]).unwrap();
    assert_eq!(results[0].as_integer(), Some(42));
}

#[test]
fn test_load_with_name() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let func = vm.load_with_name("return 'hello'", "@my_script").unwrap();
    let results = vm.call(func, vec![]).unwrap();
    assert_eq!(results[0].as_str(), Some("hello"));
}

#[test]
fn test_load_does_not_execute() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    // Load but don't execute — global should not be set
    let _func = vm.load("x = 999").unwrap();
    let x = vm.get_global("x").unwrap();
    assert!(x.is_none());
}

// ============================
// P4: register_type_of on LuaVM (tested implicitly via existing userdata tests)
// ============================

// ============================
// P5: TableBuilder
// ============================

#[test]
fn test_table_builder_named_keys() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let host = vm.create_string("localhost").unwrap();
    let table = TableBuilder::new()
        .set("host", host)
        .set("port", LuaValue::integer(8080))
        .build(&mut vm)
        .unwrap();

    let host_key = vm.create_string("host").unwrap();
    let port_key = vm.create_string("port").unwrap();
    assert_eq!(
        vm.raw_get(&table, &host_key).unwrap().as_str(),
        Some("localhost")
    );
    assert_eq!(
        vm.raw_get(&table, &port_key).unwrap().as_integer(),
        Some(8080)
    );
}

#[test]
fn test_table_builder_array() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let table = TableBuilder::new()
        .push(LuaValue::integer(10))
        .push(LuaValue::integer(20))
        .push(LuaValue::integer(30))
        .build(&mut vm)
        .unwrap();

    vm.set_global("arr", table).unwrap();
    let results = vm.execute("return #arr, arr[1], arr[2], arr[3]").unwrap();
    assert_eq!(results[0].as_integer(), Some(3));
    assert_eq!(results[1].as_integer(), Some(10));
    assert_eq!(results[2].as_integer(), Some(20));
    assert_eq!(results[3].as_integer(), Some(30));
}

#[test]
fn test_table_builder_mixed() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let name = vm.create_string("test").unwrap();
    let table = TableBuilder::new()
        .push(LuaValue::integer(1))
        .push(LuaValue::integer(2))
        .set("name", name)
        .build(&mut vm)
        .unwrap();

    vm.set_global("t", table).unwrap();
    let results = vm.execute("return t[1], t[2], t.name").unwrap();
    assert_eq!(results[0].as_integer(), Some(1));
    assert_eq!(results[1].as_integer(), Some(2));
    assert_eq!(results[2].as_str(), Some("test"));
}

// ============================
// P6: LuaError Display / std::error::Error
// ============================

#[test]
fn test_lua_error_display() {
    let err = lua_vm::LuaError::RuntimeError;
    assert_eq!(format!("{}", err), "Runtime Error");
}

#[test]
fn test_lua_error_is_std_error() {
    fn assert_error<E: std::error::Error>(_: &E) {}
    let err = lua_vm::LuaError::RuntimeError;
    assert_error(&err);
}

#[test]
fn test_lua_full_error() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    match vm.execute("error('boom')") {
        Err(e) => {
            let full = vm.into_full_error(e);
            assert_eq!(full.kind(), lua_vm::LuaError::RuntimeError);
            assert!(
                full.message().contains("boom"),
                "message should contain 'boom': {}",
                full
            );
            // Display should work
            let display = format!("{}", full);
            assert!(display.contains("boom"));
        }
        Ok(_) => panic!("expected error"),
    }
}

#[test]
fn test_lua_full_error_is_std_error() {
    fn assert_error<E: std::error::Error>(_: &E) {}
    let full = lua_vm::lua_error::LuaFullError {
        kind: lua_vm::LuaError::CompileError,
        message: "syntax error".to_string(),
    };
    assert_error(&full);
}

// ============================
// P7: Typed global getters
// ============================

#[test]
fn test_get_global_as_integer() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute("x = 42").unwrap();
    let x: i64 = vm.get_global_as::<i64>("x").unwrap().unwrap();
    assert_eq!(x, 42);
}

#[test]
fn test_get_global_as_string() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.execute("name = 'Alice'").unwrap();
    let name: String = vm.get_global_as::<String>("name").unwrap().unwrap();
    assert_eq!(name, "Alice");
}

#[test]
fn test_get_global_as_bool() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute("flag = true").unwrap();
    let flag: bool = vm.get_global_as::<bool>("flag").unwrap().unwrap();
    assert!(flag);
}

#[test]
fn test_get_global_as_none() {
    let mut vm = LuaVM::new(SafeOption::default());
    let result = vm.get_global_as::<i64>("nonexistent").unwrap();
    assert!(result.is_none());
}

// ============================
// P8: open_stdlibs
// ============================

#[test]
fn test_open_stdlibs() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlibs(&[Stdlib::Math, Stdlib::String, Stdlib::Table])
        .unwrap();

    // Math should work
    let results = vm.execute("return math.abs(-5)").unwrap();
    assert_eq!(results[0].as_integer(), Some(5));

    // String should work
    let results = vm.execute("return string.upper('hello')").unwrap();
    assert_eq!(results[0].as_str(), Some("HELLO"));
}

// ============================
// P10: table_pairs / table_length
// ============================

#[test]
fn test_table_pairs() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let table = TableBuilder::new()
        .set("a", LuaValue::integer(1))
        .set("b", LuaValue::integer(2))
        .build(&mut vm)
        .unwrap();

    let pairs = vm.table_pairs(&table).unwrap();
    assert_eq!(pairs.len(), 2);

    // Check that both entries exist (order is not guaranteed)
    let keys: Vec<_> = pairs.iter().map(|(k, _)| k.as_str().unwrap()).collect();
    assert!(keys.contains(&"a"));
    assert!(keys.contains(&"b"));
}

#[test]
fn test_table_length() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let table = TableBuilder::new()
        .push(LuaValue::integer(10))
        .push(LuaValue::integer(20))
        .push(LuaValue::integer(30))
        .build(&mut vm)
        .unwrap();

    let len = vm.table_length(&table).unwrap();
    assert_eq!(len, 3);
}

// ============================
// P11: dofile (tested with a temp file)
// ============================

#[test]
fn test_dofile() {
    use std::io::Write;

    let dir = std::env::temp_dir();
    let path = dir.join("lua_rs_test_dofile.lua");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "return 1 + 2").unwrap();
    }

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    let results = vm.dofile(path.to_str().unwrap()).unwrap();
    assert_eq!(results[0].as_integer(), Some(3));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_dofile_not_found() {
    let mut vm = LuaVM::new(SafeOption::default());
    let result = vm.dofile("nonexistent_file_12345.lua");
    assert!(result.is_err());
}

// ============================
// LuaState proxy tests
// ============================

#[test]
fn test_lua_state_load_proxy() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    // register_function that uses LuaState's load proxy
    vm.register_function("test_load", |state| {
        let func = state.load("return 77")?;
        state.push_value(func)?;
        Ok(1)
    })
    .unwrap();

    let results = vm.execute("local f = test_load(); return f()").unwrap();
    assert_eq!(results[0].as_integer(), Some(77));
}

#[test]
fn test_lua_state_call_global_proxy() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.execute("function double(x) return x * 2 end").unwrap();

    vm.register_function("test_call_global", |state| {
        let results = state.call_global("double", vec![LuaValue::integer(21)])?;
        state.push_value(results[0])?;
        Ok(1)
    })
    .unwrap();

    let results = vm.execute("return test_call_global()").unwrap();
    assert_eq!(results[0].as_integer(), Some(42));
}
