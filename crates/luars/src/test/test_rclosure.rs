/// Tests for RClosure (Rust closures registered as Lua functions)
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState, LuaVM, SafeOption};

#[test]
fn test_rclosure_basic_no_return() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|_state: &mut LuaState| -> LuaResult<usize> { Ok(0) })
        .unwrap();
    vm.set_global("myfn", func).unwrap();

    let result = vm.execute_string("myfn(); return 42");
    assert!(result.is_ok(), "RClosure call failed: {:?}", result.err());
    let vals = result.unwrap();
    assert_eq!(vals[0].as_integer(), Some(42));
}

#[test]
fn test_rclosure_returns_value() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|state: &mut LuaState| -> LuaResult<usize> {
            state.push_value(LuaValue::integer(123))?;
            Ok(1)
        })
        .unwrap();
    vm.set_global("myfn", func).unwrap();

    let result = vm.execute_string("return myfn()");
    assert!(result.is_ok(), "RClosure call failed: {:?}", result.err());
    let vals = result.unwrap();
    assert_eq!(vals[0].as_integer(), Some(123));
}

#[test]
fn test_rclosure_reads_args() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|state: &mut LuaState| -> LuaResult<usize> {
            let a = state.get_arg(1).unwrap_or(LuaValue::nil());
            let b = state.get_arg(2).unwrap_or(LuaValue::nil());
            let sum = a.as_integer().unwrap_or(0) + b.as_integer().unwrap_or(0);
            state.push_value(LuaValue::integer(sum))?;
            Ok(1)
        })
        .unwrap();
    vm.set_global("add", func).unwrap();

    let result = vm.execute_string("return add(10, 32)");
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap()[0].as_integer(), Some(42));
}

#[test]
fn test_rclosure_captures_state() {
    use std::cell::Cell;
    use std::rc::Rc;

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let counter = Rc::new(Cell::new(0i64));
    let counter_clone = counter.clone();

    let func = vm
        .create_closure(move |state: &mut LuaState| -> LuaResult<usize> {
            let current = counter_clone.get();
            counter_clone.set(current + 1);
            state.push_value(LuaValue::integer(current + 1))?;
            Ok(1)
        })
        .unwrap();
    vm.set_global("next_id", func).unwrap();

    let result = vm.execute_string(
        r#"
        local a = next_id()
        local b = next_id()
        local c = next_id()
        return a, b, c
        "#,
    );
    assert!(result.is_ok(), "{:?}", result.err());
    let vals = result.unwrap();
    assert_eq!(vals[0].as_integer(), Some(1));
    assert_eq!(vals[1].as_integer(), Some(2));
    assert_eq!(vals[2].as_integer(), Some(3));
    assert_eq!(counter.get(), 3);
}

#[test]
fn test_rclosure_with_upvalues() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let upvalues = vec![LuaValue::integer(100)];
    let func = vm
        .create_closure_with_upvalues(
            |state: &mut LuaState| -> LuaResult<usize> {
                // Get the function value from the stack (func_idx position)
                // The upvalues are stored on the RClosureFunction, accessed via the function value
                let frame = &state.call_stack[state.call_depth() - 1];
                let func_idx = frame.base - frame.func_offset;
                let func_val = state.stack_get(func_idx).unwrap();
                let rclosure = func_val.as_rclosure().unwrap();
                let offset = rclosure.upvalues()[0].as_integer().unwrap_or(0);

                let arg = state.get_arg(1).and_then(|v| v.as_integer()).unwrap_or(0);
                state.push_value(LuaValue::integer(arg + offset))?;
                Ok(1)
            },
            upvalues,
        )
        .unwrap();
    vm.set_global("add_offset", func).unwrap();

    let result = vm.execute_string("return add_offset(42)");
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap()[0].as_integer(), Some(142));
}

#[test]
fn test_rclosure_multiple_returns() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|state: &mut LuaState| -> LuaResult<usize> {
            state.push_value(LuaValue::integer(1))?;
            state.push_value(LuaValue::integer(2))?;
            state.push_value(LuaValue::integer(3))?;
            Ok(3)
        })
        .unwrap();
    vm.set_global("three", func).unwrap();

    let result = vm.execute_string("return three()");
    assert!(result.is_ok(), "{:?}", result.err());
    let vals = result.unwrap();
    assert_eq!(vals.len(), 3);
    assert_eq!(vals[0].as_integer(), Some(1));
    assert_eq!(vals[1].as_integer(), Some(2));
    assert_eq!(vals[2].as_integer(), Some(3));
}

#[test]
fn test_rclosure_called_from_lua_closure() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|state: &mut LuaState| -> LuaResult<usize> {
            let x = state.get_arg(1).and_then(|v| v.as_integer()).unwrap_or(0);
            state.push_value(LuaValue::integer(x * x))?;
            Ok(1)
        })
        .unwrap();
    vm.set_global("square", func).unwrap();

    let result = vm.execute_string(
        r#"
        local function apply(f, x)
            return f(x)
        end
        return apply(square, 7)
        "#,
    );
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap()[0].as_integer(), Some(49));
}

#[test]
fn test_rclosure_in_table() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|state: &mut LuaState| -> LuaResult<usize> {
            state.push_value(LuaValue::integer(999))?;
            Ok(1)
        })
        .unwrap();
    vm.set_global("magic", func).unwrap();

    let result = vm.execute_string(
        r#"
        local t = { fn = magic }
        return t.fn()
        "#,
    );
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap()[0].as_integer(), Some(999));
}

#[test]
fn test_rclosure_tail_call() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|state: &mut LuaState| -> LuaResult<usize> {
            let x = state.get_arg(1).and_then(|v| v.as_integer()).unwrap_or(0);
            state.push_value(LuaValue::integer(x + 1))?;
            Ok(1)
        })
        .unwrap();
    vm.set_global("inc", func).unwrap();

    // Lua tail-calls the RClosure
    let result = vm.execute_string(
        r#"
        local function wrapper(x)
            return inc(x)
        end
        return wrapper(41)
        "#,
    );
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap()[0].as_integer(), Some(42));
}

#[test]
fn test_rclosure_error_propagation() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|state: &mut LuaState| -> LuaResult<usize> {
            Err(state.error("custom error from Rust closure".to_string()))
        })
        .unwrap();
    vm.set_global("fail_fn", func).unwrap();

    let result = vm.execute_string(
        r#"
        local ok, msg = pcall(fail_fn)
        assert(not ok)
        assert(type(msg) == "string")
        return msg
        "#,
    );
    assert!(result.is_ok(), "{:?}", result.err());
}

#[test]
fn test_rclosure_gc_survives() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|state: &mut LuaState| -> LuaResult<usize> {
            state.push_value(LuaValue::integer(77))?;
            Ok(1)
        })
        .unwrap();
    vm.set_global("gc_test", func).unwrap();

    // Force GC then call the function
    let result = vm.execute_string(
        r#"
        collectgarbage("collect")
        collectgarbage("collect")
        return gc_test()
        "#,
    );
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap()[0].as_integer(), Some(77));
}

#[test]
fn test_rclosure_type_function() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let func = vm
        .create_closure(|_state: &mut LuaState| -> LuaResult<usize> { Ok(0) })
        .unwrap();
    vm.set_global("myfn", func).unwrap();

    let result = vm.execute_string("return type(myfn)");
    assert!(result.is_ok(), "{:?}", result.err());
    let vals = result.unwrap();
    assert_eq!(vals[0].as_str(), Some("function"));
}
