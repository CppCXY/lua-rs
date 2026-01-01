#[cfg(test)]
use crate::lua_vm::LuaVM;
use crate::lua_vm::SafeOption;

#[test]
fn test_xpcall_simple() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    // Test 1: Basic xpcall without upvalues
    let result = vm.execute_string(
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
    let result2 = vm.execute_string(
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
    vm.open_libs();

    // Test: Handler with string concatenation
    let result = vm.execute_string(
        r#"
        local flag = false
        local function handler(err)
            flag = true
            return "handled: " .. tostring(err)
        end
        
        local ok, msg = xpcall(function() error("xyz") end, handler)
        assert(ok == false, "ok should be false")
        assert(flag == true, "flag should be true")
        assert(msg == "handled: xyz", "msg should be 'handled: xyz'")
        "#,
    );
    assert!(result.is_ok(), "Test failed: {:?}", result);
}
