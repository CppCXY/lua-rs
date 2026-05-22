// Tests for coroutine library functions
use crate::*;

#[test]
fn test_coroutine_create_resume() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local co = coroutine.create(function()
            return 42
        end)
        
        assert(type(co) == "thread")
        local ok, value = coroutine.resume(co)
        assert(ok == true)
        assert(value == 42)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_yield() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local co = coroutine.create(function()
            coroutine.yield(1)
            coroutine.yield(2)
            return 3
        end)
        
        local ok1, v1 = coroutine.resume(co)
        assert(ok1 == true and v1 == 1)
        
        local ok2, v2 = coroutine.resume(co)
        assert(ok2 == true and v2 == 2)
        
        local ok3, v3 = coroutine.resume(co)
        assert(ok3 == true and v3 == 3)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("test_coroutine_yield Error: {}", e);
        eprintln!("Error message: {}", vm.main_state().get_error_message(*e));
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_status() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local co = coroutine.create(function()
            coroutine.yield()
        end)
        
        assert(coroutine.status(co) == "suspended")
        coroutine.resume(co)
        assert(coroutine.status(co) == "suspended")
        coroutine.resume(co)
        assert(coroutine.status(co) == "dead")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_coroutine_running() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local main_co = coroutine.running()
        assert(main_co ~= nil)
        
        local co = coroutine.create(function()
            local inner_co = coroutine.running()
            assert(inner_co == co)
        end)
        
        coroutine.resume(co)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("test_coroutine_running Error: {}", e);
        eprintln!("Error message: {}", vm.main_state().get_error_message(*e));
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_wrap() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local f = coroutine.wrap(function()
            coroutine.yield(1)
            coroutine.yield(2)
            return 3
        end)
        
        assert(f() == 1)
        assert(f() == 2)
        assert(f() == 3)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("test_coroutine_wrap Error: {}", e);
        eprintln!("Error message: {}", vm.main_state().get_error_message(*e));
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_isyieldable() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local co = coroutine.create(function()
            assert(coroutine.isyieldable() == true)
        end)
        
        coroutine.resume(co)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_coroutine_close() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local co = coroutine.create(function()
            coroutine.yield(1)
            coroutine.yield(2)
        end)
        
        coroutine.resume(co)
        assert(coroutine.status(co) == "suspended")
        
        coroutine.close(co)
        assert(coroutine.status(co) == "dead")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_coroutine_with_loop() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    // Test that coroutine with for loop can be created and resumed
    let result = vm.main_state().execute(
        r#"
        local co = coroutine.create(function()
            local count = 0
            for i = 1, 5 do
                count = count + 1
                coroutine.yield(i * 2)
            end
            return count
        end)
        
        -- First resume
        local ok1, val1 = coroutine.resume(co)
        assert(ok1 == true, "First resume should succeed")
        assert(val1 == 2, "First value should be 2")
        
        -- Second resume
        local ok2, val2 = coroutine.resume(co)
        assert(ok2 == true, "Second resume should succeed")
        assert(val2 == 4, "Second value should be 4")
        
        -- Third resume
        local ok3, val3 = coroutine.resume(co)
        assert(ok3 == true, "Third resume should succeed")
        assert(val3 == 6, "Third value should be 6")
    "#,
    );

    assert!(result.is_ok(), "Test failed: {:?}", result);
}

#[test]
fn test_coroutine_error_handling() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local co = coroutine.create(function()
            error("test error")
        end)
        
        local ok, err = coroutine.resume(co)
        assert(ok == false)
        assert(type(err) == "string")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_coroutine_no_message_error_does_not_reuse_stale_message() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    vm.register_function("raw_runtime_error", |_| Err(lua_vm::LuaError::RuntimeError))
        .unwrap();

    let result = vm.main_state().execute(
        r#"
        local co1 = coroutine.create(function()
            error("first coroutine error")
        end)

        local ok1, err1 = coroutine.resume(co1)
        assert(ok1 == false)
        assert(type(err1) == "string")
        assert(string.find(err1, "first coroutine error", 1, true) ~= nil)

        local co2 = coroutine.create(function()
            raw_runtime_error()
        end)

        local ok2, err2 = coroutine.resume(co2)
        assert(ok2 == false)
        assert(err2 == nil, tostring(err2))
    "#,
    );

    assert!(result.is_ok(), "Test failed: {:?}", result);
}

#[test]
fn test_coroutine_thread_line_hook_fires_on_resume() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local co = coroutine.create(function (x)
            local a = 1
            coroutine.yield(debug.getinfo(1, "l"))
            coroutine.yield(debug.getinfo(1, "l").currentline)
            return a
        end)

        local tr = {}
        local foo = function (e, l) if l then table.insert(tr, l) end end
        debug.sethook(co, foo, "lcr")

        local ok, info = coroutine.resume(co, 10)
        assert(ok)
        assert(type(info) == "table")
        assert(#tr == 2, #tr)
        assert(tr[1] == info.currentline - 1 and tr[2] == info.currentline)
    "#,
    );

    if let Err(e) = &result {
        eprintln!(
            "test_coroutine_thread_line_hook_fires_on_resume Error: {}",
            e
        );
        eprintln!("Error message: {}", vm.main_state().get_error_message(*e));
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_traceback_contains_suspend_and_recursive_frames() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.main_state().execute(
        r#"
        local function f(i)
          if i == 0 then error(i)
          else coroutine.yield(); f(i - 1) end
        end

        local co = coroutine.create(function (x) f(x) end)
        local traces = {}

        local ok, err = coroutine.resume(co, 3)
        traces[#traces + 1] = debug.traceback(co)

        ok, err = coroutine.resume(co)
        traces[#traces + 1] = debug.traceback(co)

        ok, err = coroutine.resume(co)
        traces[#traces + 1] = debug.traceback(co)

        ok, err = coroutine.resume(co)
        traces[#traces + 1] = debug.traceback(co)

        return traces[1], traces[2], traces[3], traces[4]
    "#,
    );

    match result {
        Ok(values) => {
            let traces: Vec<String> = values
                .into_iter()
                .map(|value| value.as_str().unwrap_or_default().to_string())
                .collect();
            for (idx, trace) in traces.iter().enumerate() {
                eprintln!("trace[{idx}] =\n{trace}\n---");
            }
            assert!(
                traces[0].contains("yield"),
                "first trace missing yield: {}",
                traces[0]
            );
            assert!(
                traces[0].contains("function <"),
                "first trace missing function frame: {}",
                traces[0]
            );
            assert!(
                traces[3].contains("error"),
                "final trace missing error: {}",
                traces[3]
            );
        }
        Err(e) => {
            eprintln!(
                "test_coroutine_traceback_contains_suspend_and_recursive_frames Error: {}",
                e
            );
            eprintln!("Error message: {}", vm.main_state().get_error_message(e));
            panic!("traceback capture script failed");
        }
    }
}
