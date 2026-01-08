use crate::lua_value::LuaValue;
/// Tests for C function calling
use crate::lua_vm::{LuaResult, LuaState, LuaVM, SafeOption};

/// C function with no return value
fn test_no_return(_state: &mut LuaState) -> LuaResult<usize> {
    Ok(0)
}

#[test]
fn test_call_c_function_basic() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Register a simple C function
    let c_func = LuaValue::cfunction(test_no_return);
    vm.set_global("test_func", c_func);

    // Call it from Lua
    let result = vm.execute_string(
        r#"
        test_func()
        return 42
        "#,
    );

    // Should not error
    assert!(result.is_ok(), "C function call failed: {:?}", result.err());
}

#[test]
fn test_call_c_function_in_expression() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Register C function
    let c_func = LuaValue::cfunction(test_no_return);
    vm.set_global("cfunc", c_func);

    // Use in expression
    let result = vm.execute_string(
        r#"
        local x = cfunc()
        assert(x == nil)
        "#,
    );

    assert!(
        result.is_ok(),
        "C function in expression failed: {:?}",
        result.err()
    );
}

#[test]
fn test_call_c_function_multiple_times() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Register C function
    let c_func = LuaValue::cfunction(test_no_return);
    vm.set_global("cfunc", c_func);

    // Call multiple times
    let result = vm.execute_string(
        r#"
        for i = 1, 10 do
            cfunc()
        end
        "#,
    );

    assert!(
        result.is_ok(),
        "Multiple C function calls failed: {:?}",
        result.err()
    );
}

#[test]
fn test_c_function_in_tail_call() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Register C function
    let c_func = LuaValue::cfunction(test_no_return);
    vm.set_global("cfunc", c_func);

    // Use in tail call position
    let result = vm.execute_string(
        r#"
        local function wrapper()
            return cfunc()
        end
        wrapper()
        "#,
    );

    assert!(
        result.is_ok(),
        "C function tail call failed: {:?}",
        result.err()
    );
}
