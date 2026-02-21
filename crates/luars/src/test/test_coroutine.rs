// Tests for coroutine library functions
use crate::*;

#[test]
fn test_coroutine_create_resume() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute(
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
        eprintln!("Error message: {}", vm.get_error_message(*e));
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_status() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute(
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
        eprintln!("Error message: {}", vm.get_error_message(*e));
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_wrap() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute(
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
        eprintln!("Error message: {}", vm.get_error_message(*e));
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_isyieldable() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    // Test that coroutine with for loop can be created and resumed
    let result = vm.execute(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();
    let result = vm.execute(
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
