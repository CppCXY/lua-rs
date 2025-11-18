// Tests for coroutine library functions
use crate::*;

#[test]
fn test_coroutine_create_resume() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
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
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
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

    assert!(result.is_ok());
}

#[test]
fn test_coroutine_status() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
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
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
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
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_wrap() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
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
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_coroutine_isyieldable() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
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
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
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
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local co = coroutine.create(function()
            for i = 1, 5 do
                coroutine.yield(i * 2)
            end
        end)
        
        local results = {}
        while coroutine.status(co) ~= "dead" do
            local ok, value = coroutine.resume(co)
            if ok and value then
                table.insert(results, value)
            end
        end
        
        assert(#results == 5)
        assert(results[1] == 2)
        assert(results[5] == 10)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_coroutine_error_handling() {
    let mut vm = LuaVM::new();
    vm.open_libs();

    let result = vm.execute_string(
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
