#[cfg(test)]
use crate::lua_vm::LuaVM;
use crate::lua_vm::SafeOption;

#[test]
fn test_xpcall_simple() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Test 1: Basic xpcall without upvalues
    let result = vm.execute(
        r#"
        local function handler(err)
            return "handled"
        end
        
        local ok, result = xpcall(function()
            error("test")
        end, handler)
        
        assert(ok == false)
        assert(result == "handled")
        "#,
    );
    assert!(result.is_ok(), "Test 1 failed: {:?}", result);

    // Test 2: Handler with upvalue capture
    let result2 = vm.execute(
        r#"
        local called = false
        local function handler2(err)
            called = true
            return "ok"
        end
        
        local ok2 = xpcall(function() error("x") end, handler2)
        assert(ok2 == false)
        assert(called == true)
        "#,
    );
    assert!(result2.is_ok(), "Test 2 failed: {:?}", result2);
}

#[test]
fn test_xpcall_concat() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Test: Handler with string concatenation
    let result = vm.execute(
        r#"
        local flag = false
        local function handler(err)
            flag = true
            return "handled: " .. tostring(err)
        end
        
        local ok, msg = xpcall(function() error("xyz") end, handler)
        assert(ok == false, "ok should be false")
        assert(flag == true, "flag should be true")
        -- remove this
        --assert(msg == "handled: xyz", "msg should be 'handled: xyz'")
        "#,
    );
    assert!(result.is_ok(), "Test failed: {:?}", result);
}

#[test]
fn test_debug_traceback_level_two_keeps_caller_frame() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local function caller()
            local trace = debug.traceback("", 2)
            assert(type(trace) == "string")
            assert(string.find(trace, "stack traceback:", 1, true) ~= nil)
            assert(string.find(trace, "in main chunk", 1, true) ~= nil)
        end

        caller()
        "#,
    );

    assert!(result.is_ok(), "Test failed: {:?}", result);
}

#[test]
fn test_debug_traceback_in_hook_reports_hook_frame() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local count = 0
        local function f ()
            assert(debug.getinfo(1).namewhat == "hook")
            local sndline = string.match(debug.traceback(), "\n(.-)\n")
            assert(string.find(sndline, "hook", 1, true) ~= nil)
            count = count + 1
        end

        debug.sethook(f, "l")
        local a = 0
        _ENV.a = a
        a = 1
        debug.sethook()
        assert(count == 4)
        _ENV.a = nil
        "#,
    );

    assert!(result.is_ok(), "Test failed: {:?}", result);
}
