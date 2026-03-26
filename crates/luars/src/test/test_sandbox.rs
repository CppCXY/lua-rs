use std::time::Duration;

use crate::{LuaVM, LuaValue, SafeOption, SandboxConfig, Stdlib};

#[test]
fn test_execute_sandboxed_isolates_globals() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let results = vm
        .execute_sandboxed(
            "sandbox_value = 42; return sandbox_value, _G == _ENV",
            &SandboxConfig::default(),
        )
        .unwrap();

    assert_eq!(results[0].as_integer(), Some(42));
    assert!(results[1].bvalue());
    assert!(vm.get_global("sandbox_value").unwrap().is_none());
}

#[test]
fn test_sandbox_blocks_dangerous_basic_functions_by_default() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let results = vm
        .execute_sandboxed(
            "return require, load, loadfile, dofile, collectgarbage",
            &SandboxConfig::default(),
        )
        .unwrap();

    assert!(results.iter().all(|value| value.is_nil()));
}

#[test]
fn test_load_sandboxed_uses_own_env() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.set_global("shared_value", crate::LuaValue::integer(7))
        .unwrap();

    let config = SandboxConfig::default();
    let func = vm
        .load_sandboxed(
            "local local_only = 11; return shared_value, local_only",
            &config,
        )
        .unwrap();
    let results: Vec<crate::LuaValue> = vm.call_raw(func, vec![]).unwrap();

    assert!(results[0].is_nil());
    assert_eq!(results[1].as_integer(), Some(11));
}

#[test]
fn test_sandbox_can_enable_package_require_explicitly() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let config = SandboxConfig::default()
        .with_stdlib(Stdlib::Package)
        .allow_require();
    let results = vm
        .execute_sandboxed(
            "local p = require('math'); return type(p), type(require)",
            &config,
        )
        .unwrap();

    assert_eq!(results[0].as_str(), Some("table"));
    assert_eq!(results[1].as_str(), Some("function"));
}

#[test]
fn test_sandbox_can_inject_custom_globals() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let config = SandboxConfig::default().with_global("answer", LuaValue::integer(99));
    let results = vm
        .execute_sandboxed("return answer, _G.answer == answer", &config)
        .unwrap();

    assert_eq!(results[0].as_integer(), Some(99));
    assert!(results[1].bvalue());
    assert!(vm.get_global("answer").unwrap().is_none());
}

#[test]
fn test_sandbox_instruction_limit_stops_infinite_loops() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let config = SandboxConfig::default().with_instruction_limit(1_000);
    let err = vm
        .execute_sandboxed("while true do end", &config)
        .unwrap_err();
    let full = vm.into_full_error(err);

    assert!(full.message.contains("sandbox instruction limit exceeded"));
}

#[test]
fn test_sandbox_memory_limit_blocks_runtime_allocations() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let config = SandboxConfig::default().with_memory_limit(0);
    let err = vm
        .execute_sandboxed("local t = {}; return t", &config)
        .unwrap_err();
    let full = vm.into_full_error(err);

    assert!(matches!(full.kind, crate::lua_vm::LuaError::OutOfMemory));
}

#[test]
fn test_sandbox_timeout_stops_infinite_loops() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    let config = SandboxConfig::default().with_timeout(Duration::ZERO);
    let err = vm
        .execute_sandboxed("while true do end", &config)
        .unwrap_err();
    let full = vm.into_full_error(err);

    assert!(full.message.contains("sandbox timeout exceeded"));
}
