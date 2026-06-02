// Tests for API improvement proposals (P1–P11)
use crate::*;

use std::path::PathBuf;

// ============================
// P1: call / call_global
// ============================

#[test]
fn test_call_lua_function() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.main_state()
        .execute("function add(a, b) return a + b end")
        .unwrap();
    let func = vm.get_global("add").unwrap().unwrap();
    let result = vm
        .main_state()
        .call(func, vec![LuaValue::integer(3), LuaValue::integer(4)])
        .unwrap();
    let result = result[0].as_integer().unwrap();
    assert_eq!(result, 7);
}

#[test]
fn test_call_lua_function_raw() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.main_state()
        .execute("function add(a, b) return a + b end")
        .unwrap();
    let func = vm.get_global("add").unwrap().unwrap();
    let results = vm
        .main_state()
        .call(func, vec![LuaValue::integer(3), LuaValue::integer(4)])
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(7));
}

#[test]
fn test_call_global() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.main_state()
        .execute("function greet(name) return 'Hello, ' .. name end")
        .unwrap();
    let name = vm.create_string("World").unwrap();
    let result = {
        let state = vm.main_state();
        state.call_global("greet", vec![name]).unwrap()
    };
    let result = result[0].as_str().unwrap();
    assert_eq!(result, "Hello, World");
}

#[test]
fn test_call_global_not_found() {
    let mut vm = GlobalState::new(SafeOption::default());
    let result = vm.main_state().call_global("nonexistent", vec![]);
    assert!(result.is_err());
}

fn test_temp_dir() -> PathBuf {
    #[cfg(miri)]
    {
        if let Some(path) = std::env::var_os("TEMP") {
            return PathBuf::from(path);
        }
        if let Some(path) = std::env::var_os("TMP") {
            return PathBuf::from(path);
        }
        panic!("Miri tests require TEMP or TMP to be set");
    }

    #[cfg(not(miri))]
    {
        std::env::temp_dir()
    }
}

// ============================
// P2: register_function
// ============================

#[test]
fn test_register_function() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.register_function("rust_add", |state| {
        let a = state.get_arg(1).and_then(|v| v.as_integer()).unwrap_or(0);
        let b = state.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(0);
        state.push_value(LuaValue::integer(a + b))?;
        Ok(1)
    })
    .unwrap();

    let results = vm.main_state().execute("return rust_add(10, 20)").unwrap();
    assert_eq!(results[0].as_integer(), Some(30));
}

#[test]
fn test_register_function_typed() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.register_function_typed("rust_add_typed", |a: i64, b: i64| a + b)
        .unwrap();

    let results = vm
        .main_state()
        .execute("return rust_add_typed(10, 20)")
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(30));
}

#[test]
fn test_register_function_typed_userdata_ref() {
    #[derive(Debug)]
    struct Counter {
        count: i64,
    }

    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let counter = vm.create_any(Counter { count: 1 }).unwrap();
    vm.set_global("counter", counter).unwrap();

    vm.register_function_typed(
        "increment_typed",
        |mut counter: UserDataRef<Counter>, delta: i64| {
            let counter_ref = counter.get_mut().unwrap();
            counter_ref.count += delta;
            counter_ref.count
        },
    )
    .unwrap();

    let results = vm
        .main_state()
        .execute("return increment_typed(counter, 9)")
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(10));

    let counter: UserDataRef<Counter> = vm.main_state().get_global_as("counter").unwrap().unwrap();
    assert_eq!(counter.get().unwrap().count, 10);
}

#[test]
fn test_register_function_typed_high_arity() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.register_function_typed(
        "sum8",
        |a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64| {
            a + b + c + d + e + f + g + h
        },
    )
    .unwrap();

    let results = vm
        .main_state()
        .execute("return sum8(1, 2, 3, 4, 5, 6, 7, 8)")
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(36));
}

// ============================
// P3: load / load_with_name
// ============================

#[test]
fn test_load_and_call() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let func = vm.main_state().load("return 42").unwrap();
    let result = vm.main_state().call(func, vec![]).unwrap();
    let result = result[0].as_integer().unwrap();
    assert_eq!(result, 42);
}

#[test]
fn test_load_with_name() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let func = vm
        .main_state()
        .load_with_name("return 'hello'", "@my_script")
        .unwrap();
    let result = vm.main_state().call(func, vec![]).unwrap();
    let result = result[0].as_str().unwrap();
    assert_eq!(result, "hello");
}

#[test]
fn test_load_does_not_execute() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    // Load but don't execute — global should not be set
    let _func = vm.main_state().load("x = 999").unwrap();
    let x = vm.get_global("x").unwrap();
    assert!(x.is_none());
}

#[test]
fn test_explicit_close_of_default_output_restores_stdout() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let temp_path = test_temp_dir().join(format!(
        "luars_io_close_restore_{}_{}.tmp",
        std::process::id(),
        crate::platform_time::unix_nanos()
    ));
    let temp_path = temp_path.to_string_lossy().replace('\\', "\\\\");

    vm.main_state()
        .execute(&format!(
            "assert(io.output(\"{temp_path}\")); assert(io.close(io.output()))"
        ))
        .unwrap();

    let func = vm.main_state().load("return io.write({})").unwrap();
    let (ok, results) = vm.main_state().pcall(func, vec![]).unwrap();
    assert!(!ok);
    let err = results[0].as_str().unwrap();
    assert!(err.contains("bad argument #1 to 'write'"), "{err}");

    let _ = std::fs::remove_file(temp_path.replace("\\\\", "\\"));
}

#[cfg(feature = "shared-proto")]
#[test]
fn test_load_marks_chunk_short_strings_shared() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let func = vm
        .main_state()
        .load("local key = '0123456789abcdefghijklmnopqr'; return key")
        .unwrap();

    let function = func.as_function_ptr().unwrap();
    let chunk = function.as_ref().data.chunk();
    let key = chunk
        .constants
        .iter()
        .copied()
        .find(|value| value.as_str() == Some("0123456789abcdefghijklmnopqr"))
        .unwrap();

    assert!(key.is_short_string());
    assert!(key.as_string_ptr().unwrap().as_ref().header.is_shared());
    assert!(function.as_ref().data.proto().as_ref().header.is_shared());
}

#[cfg(feature = "shared-proto")]
#[test]
fn test_shared_proto_survives_vm_drop() {
    let proto = {
        let mut vm = GlobalState::new(SafeOption::default());
        vm.open_stdlib(Stdlib::All).unwrap();

        let func = vm
            .main_state()
            .load("local key = '0123456789abcdefghijklmnopqr'; return key")
            .unwrap();

        func.as_function_ptr().unwrap().as_ref().data.proto()
    };

    assert!(proto.as_ref().header.is_shared());

    let key = proto
        .as_ref()
        .data
        .constants
        .iter()
        .copied()
        .find(|value| value.as_str() == Some("0123456789abcdefghijklmnopqr"))
        .unwrap();

    assert!(key.as_string_ptr().unwrap().as_ref().header.is_shared());
}

#[cfg(feature = "shared-proto")]
#[test]
fn test_shared_proto_reuses_same_file_across_vms() {
    use std::io::Write;

    let path = test_temp_dir().join("lua_rs_shared_proto_cache.lua");
    {
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "return 'shared-proto-cache'").unwrap();
    }

    let proto1 = {
        let mut vm = GlobalState::new(SafeOption::default());
        vm.open_stdlib(Stdlib::All).unwrap();
        vm.main_state()
            .load_proto_from_file(path.to_str().unwrap())
            .unwrap()
    };

    let proto2 = {
        let mut vm = GlobalState::new(SafeOption::default());
        vm.open_stdlib(Stdlib::All).unwrap();
        vm.main_state()
            .load_proto_from_file(path.to_str().unwrap())
            .unwrap()
    };

    assert_eq!(proto1, proto2);

    std::fs::remove_file(&path).ok();
}

#[cfg(feature = "shared-proto")]
#[test]
fn test_shared_proto_reloads_when_file_changes() {
    use std::io::Write;

    let path = test_temp_dir().join("lua_rs_shared_proto_reload.lua");
    {
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "return 1").unwrap();
    }

    let proto1 = {
        let mut vm = GlobalState::new(SafeOption::default());
        vm.open_stdlib(Stdlib::All).unwrap();
        vm.main_state()
            .load_proto_from_file(path.to_str().unwrap())
            .unwrap()
    };

    {
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "return 123456789").unwrap();
    }

    let proto2 = {
        let mut vm = GlobalState::new(SafeOption::default());
        vm.open_stdlib(Stdlib::All).unwrap();
        vm.main_state()
            .load_proto_from_file(path.to_str().unwrap())
            .unwrap()
    };

    assert_ne!(proto1, proto2);

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_load_reuses_source_name_across_child_protos() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let source = r#"
        local function a() return 1 end
        local function b() return a() end
        return b
    "#;
    let chunk = vm.compile_with_name(source, source).unwrap();
    let root_source = chunk.source_name.as_ref().unwrap();
    assert!(root_source.len() > 16);
    assert_eq!(chunk.child_protos.len(), 2);

    for child in &chunk.child_protos {
        let child_source = child.as_ref().data.source_name.as_ref().unwrap();
        assert!(std::sync::Arc::ptr_eq(root_source, child_source));
    }
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    match vm.main_state().execute("error('boom')") {
        Err(e) => {
            let full = vm.main_state().get_full_error(e);
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
    let mut vm = GlobalState::new(SafeOption::default());
    vm.main_state().execute("x = 42").unwrap();
    let x: i64 = vm.main_state().get_global_as::<i64>("x").unwrap().unwrap();
    assert_eq!(x, 42);
}

#[test]
fn test_get_global_as_string() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.main_state().execute("name = 'Alice'").unwrap();
    let name: String = vm
        .main_state()
        .get_global_as::<String>("name")
        .unwrap()
        .unwrap();
    assert_eq!(name, "Alice");
}

#[test]
fn test_get_global_as_bool() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.main_state().execute("flag = true").unwrap();
    let flag: bool = vm
        .main_state()
        .get_global_as::<bool>("flag")
        .unwrap()
        .unwrap();
    assert!(flag);
}

#[test]
fn test_get_global_as_none() {
    let mut vm = GlobalState::new(SafeOption::default());
    let result = vm.main_state().get_global_as::<i64>("nonexistent").unwrap();
    assert!(result.is_none());
}

// ============================
// P8: open_stdlibs
// ============================

#[test]
fn test_open_stdlibs() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlibs(&[Stdlib::Math, Stdlib::String, Stdlib::Table])
        .unwrap();

    // Math should work
    let results = vm.main_state().execute("return math.abs(-5)").unwrap();
    assert_eq!(results[0].as_integer(), Some(5));

    // String should work
    let results = vm
        .main_state()
        .execute("return string.upper('hello')")
        .unwrap();
    assert_eq!(results[0].as_str(), Some("HELLO"));
}

// ============================
// P11: dofile (tested with a temp file)
// ============================

#[test]
fn test_dofile() {
    use std::io::Write;

    let dir = test_temp_dir();
    let path = dir.join("lua_rs_test_dofile.lua");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "return 1 + 2").unwrap();
    }

    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    let results = vm.main_state().dofile(path.to_str().unwrap()).unwrap();
    assert_eq!(results[0].as_integer(), Some(3));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_dofile_not_found() {
    let mut vm = GlobalState::new(SafeOption::default());
    let result = vm.main_state().dofile("nonexistent_file_12345.lua");
    assert!(result.is_err());
}

// ============================
// LuaState proxy tests
// ============================

#[test]
fn test_lua_state_load_proxy() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    // register_function that uses LuaState's load proxy
    vm.register_function("test_load", |state| {
        let func = state.load("return 77")?;
        state.push_value(func)?;
        Ok(1)
    })
    .unwrap();

    let results = vm
        .main_state()
        .execute("local f = test_load(); return f()")
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(77));
}

#[test]
fn test_lua_state_call_global_proxy() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.main_state()
        .execute("function double(x) return x * 2 end")
        .unwrap();

    vm.register_function("test_call_global", |state| {
        let results = state.call_global("double", vec![LuaValue::integer(21)])?;
        state.push_value(results[0])?;
        Ok(1)
    })
    .unwrap();

    let results = vm
        .main_state()
        .execute("return test_call_global()")
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(42));
}

// ========== Error Recovery Tests ==========

#[test]
fn test_execute_error_recovery() {
    // After a runtime error in execute(), the VM should remain usable.
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    // Cause a runtime error
    let err = vm.main_state().execute("error('boom')");
    assert!(err.is_err());

    // VM should still be usable — execute more code
    let results = vm.main_state().execute("return 1 + 2").unwrap();
    assert_eq!(results[0].as_integer(), Some(3));
}

#[test]
fn test_execute_error_preserves_globals() {
    // Globals set before an error should still be accessible after recovery.
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.main_state().execute("x = 42").unwrap();

    let err = vm.main_state().execute("error('fail')");
    assert!(err.is_err());

    let results = vm.main_state().execute("return x").unwrap();
    assert_eq!(results[0].as_integer(), Some(42));
}

#[test]
fn test_call_global_error_recovery() {
    // call_global error should not corrupt the VM state.
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.main_state()
        .execute("function bad() error('nope') end")
        .unwrap();
    vm.main_state()
        .execute("function good() return 99 end")
        .unwrap();

    let err = vm.main_state().call_global("bad", vec![]);
    assert!(err.is_err());

    // Should still work after the error
    let results = vm.main_state().call_global("good", vec![]).unwrap();
    assert_eq!(results[0].as_integer(), Some(99));
}

#[test]
fn test_multiple_errors_recovery() {
    // Multiple consecutive errors should all recover cleanly.
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    for i in 0..5 {
        let err = vm.main_state().execute(&format!("error('error {}')", i));
        assert!(err.is_err());
    }

    // VM should still work after many errors
    let results = vm.main_state().execute("return 'still alive'").unwrap();
    assert_eq!(results[0].as_str(), Some("still alive"));
}

#[test]
fn test_error_message_available_after_recovery() {
    // get_error_message should return the correct message after recovery.
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let err = vm
        .main_state()
        .execute("error('specific error message')")
        .unwrap_err();
    let msg = vm.main_state().get_error_message(err);
    assert!(
        msg.contains("specific error message"),
        "expected message to contain 'specific error message', got: {}",
        msg
    );
    assert!(
        !msg.contains("stack traceback"),
        "raw error message should not include traceback: {}",
        msg
    );

    // VM should be usable after getting the message
    let results = vm.main_state().execute("return true").unwrap();
    assert_eq!(results[0].as_boolean(), Some(true));
}

#[test]
fn test_pcall_require_returns_raw_message_without_traceback() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let results = vm
        .main_state()
        .execute(
            r#"
        package.path = "?.lua;?/?"
        package.cpath = "?.so;?/init"
        local st, msg = pcall(require, "XXX")
        return st, msg
    "#,
        )
        .unwrap();

    assert_eq!(results[0].as_boolean(), Some(false));
    let msg = results[1].as_str().unwrap_or_default();
    let expected = "module 'XXX' not found:\n\tno field package.preload['XXX']\n\tno file 'XXX.lua'\n\tno file 'XXX/XXX'\n\tno file 'XXX.so'\n\tno file 'XXX/init'";
    assert_eq!(msg, expected);
    assert!(!msg.contains("stack traceback"));
}

#[test]
fn test_pcall_stripped_debug_info_uses_unknown_location_without_traceback() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let results = vm
        .main_state()
        .execute(
            r#"
        local f = function (a) return a + 1 end
        f = assert(load(string.dump(f, true)))
        local st, msg = pcall(f, {})
        return st, msg
    "#,
        )
        .unwrap();

    assert_eq!(results[0].as_boolean(), Some(false));
    let msg = results[1].as_str().unwrap_or_default();
    assert!(
        msg.starts_with("?:?: attempt to perform arithmetic on a table value"),
        "expected stripped-debug-info message to start with '?:?:', got: {}",
        msg
    );
    assert!(!msg.contains("stack traceback"));
}

#[test]
fn test_deep_call_error_recovery() {
    // Error in deeply nested calls should clean up all frames.
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.main_state()
        .execute(
            r#"
        function a() return b() end
        function b() return c() end
        function c() error('deep error') end
    "#,
        )
        .unwrap();

    let err = vm.main_state().call_global("a", vec![]);
    assert!(err.is_err());

    // After deep error, simple calls should work
    let results = vm.main_state().execute("return 1 + 1").unwrap();
    assert_eq!(results[0].as_integer(), Some(2));

    // And function calls too
    vm.main_state()
        .execute("function simple() return 42 end")
        .unwrap();
    let results = vm.main_state().call_global("simple", vec![]).unwrap();
    assert_eq!(results[0].as_integer(), Some(42));
}
