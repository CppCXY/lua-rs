// Tests for async function support
//
// These tests verify that Rust async functions can be registered and called
// from Lua code via the AsyncThread mechanism.

use crate::lua_value::LuaUserdata;
use crate::lua_vm::async_thread::AsyncReturnValue;
use crate::*;
use std::fmt;

/// Helper: create a VM with stdlib loaded
fn new_vm() -> Box<LuaVM> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm
}

// ============ Basic async function tests ============

#[tokio::test]
async fn test_async_basic_return() {
    let mut vm = new_vm();

    // Register an async function that returns a single value
    vm.register_async("async_add", |args| async move {
        let a = args[0].as_integer().unwrap_or(0);
        let b = args[1].as_integer().unwrap_or(0);
        Ok(vec![AsyncReturnValue::integer(a + b)])
    })
    .unwrap();

    let results = vm.execute_async("return async_add(10, 20)").await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_integer(), Some(30));
}

#[tokio::test]
async fn test_async_no_args() {
    let mut vm = new_vm();

    vm.register_async("async_hello", |_args| async move {
        Ok(vec![AsyncReturnValue::string("hello")])
    })
    .unwrap();

    let results = vm.execute_async("return async_hello()").await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("hello"));
}

#[tokio::test]
async fn test_async_multiple_returns() {
    let mut vm = new_vm();

    vm.register_async("async_multi", |_args| async move {
        Ok(vec![
            AsyncReturnValue::integer(1),
            AsyncReturnValue::integer(2),
            AsyncReturnValue::integer(3),
        ])
    })
    .unwrap();

    let results = vm.execute_async("return async_multi()").await.unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_integer(), Some(1));
    assert_eq!(results[1].as_integer(), Some(2));
    assert_eq!(results[2].as_integer(), Some(3));
}

#[tokio::test]
async fn test_async_nil_return() {
    let mut vm = new_vm();

    // Async function that returns exactly one nil value
    vm.register_async("async_nil", |_args| async move {
        Ok(vec![AsyncReturnValue::nil()])
    })
    .unwrap();

    let results = vm
        .execute_async(
            r#"
        local x = async_nil()
        assert(x == nil)
        return true
    "#,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
}

// ============ Multiple async calls in sequence ============

#[tokio::test]
async fn test_async_sequential_calls() {
    let mut vm = new_vm();

    vm.register_async("async_double", |args| async move {
        let n = args[0].as_integer().unwrap_or(0);
        Ok(vec![AsyncReturnValue::integer(n * 2)])
    })
    .unwrap();

    let results = vm
        .execute_async(
            r#"
        local a = async_double(5)
        local b = async_double(a)
        local c = async_double(b)
        return c
    "#,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_integer(), Some(40)); // 5*2*2*2
}

// ============ Async with actual .await (tokio::time::sleep) ============

#[tokio::test]
async fn test_async_with_sleep() {
    let mut vm = new_vm();

    vm.register_async("async_sleep_and_return", |args| async move {
        let val = args[0].as_integer().unwrap_or(0);
        // Actually await something
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        Ok(vec![AsyncReturnValue::integer(val + 100)])
    })
    .unwrap();

    let results = vm
        .execute_async("return async_sleep_and_return(42)")
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_integer(), Some(142));
}

// ============ Error handling ============

#[tokio::test]
async fn test_async_error_propagation() {
    let mut vm = new_vm();

    vm.register_async("async_fail", |_args| async move {
        Err(lua_vm::LuaError::RuntimeError)
    })
    .unwrap();

    let result = vm.execute_async("return async_fail()").await;

    assert!(result.is_err());
}

// ============ Interaction with normal Lua code ============

#[tokio::test]
async fn test_async_mixed_with_sync() {
    let mut vm = new_vm();

    vm.register_async("async_get", |args| async move {
        let key = args[0].as_str().unwrap_or("?").to_string();
        Ok(vec![AsyncReturnValue::string(format!("value_{}", key))])
    })
    .unwrap();

    let results = vm
        .execute_async(
            r#"
        local function sync_process(s)
            return string.upper(s)
        end
        local val = async_get("test")
        return sync_process(val)
    "#,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("VALUE_TEST"));
}

#[tokio::test]
async fn test_async_in_loop() {
    let mut vm = new_vm();

    vm.register_async("async_inc", |args| async move {
        let n = args[0].as_integer().unwrap_or(0);
        Ok(vec![AsyncReturnValue::integer(n + 1)])
    })
    .unwrap();

    let results = vm
        .execute_async(
            r#"
        local sum = 0
        for i = 1, 5 do
            sum = sum + async_inc(i)
        end
        return sum
    "#,
        )
        .await
        .unwrap();

    // sum = (1+1) + (2+1) + (3+1) + (4+1) + (5+1) = 2+3+4+5+6 = 20
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_integer(), Some(20));
}

// ============ create_async_thread API ============

#[tokio::test]
async fn test_create_async_thread_directly() {
    let mut vm = new_vm();

    vm.register_async("async_square", |args| async move {
        let n = args[0].as_integer().unwrap_or(0);
        Ok(vec![AsyncReturnValue::integer(n * n)])
    })
    .unwrap();

    let chunk = vm.compile("return async_square(7)").unwrap();
    let thread = vm.create_async_thread(chunk, vec![]).unwrap();
    let results = thread.await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_integer(), Some(49));
}

// ============ Async UserData return tests ============

/// Test struct returned from async functions
#[derive(LuaUserData)]
#[lua_impl(Display)]
struct AsyncPoint {
    pub x: f64,
    pub y: f64,
}

impl fmt::Display for AsyncPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AsyncPoint({}, {})", self.x, self.y)
    }
}

#[lua_methods]
impl AsyncPoint {
    pub fn sum(&self) -> f64 {
        self.x + self.y
    }
}

#[tokio::test]
async fn test_async_return_userdata() {
    let mut vm = new_vm();

    // Register an async function that returns a UserData
    vm.register_async("make_point", |args| async move {
        let x = args[0].as_number().unwrap_or(0.0);
        let y = args[1].as_number().unwrap_or(0.0);
        Ok(vec![AsyncReturnValue::userdata(AsyncPoint { x, y })])
    })
    .unwrap();

    // Test field access on the returned userdata
    let results = vm
        .execute_async("local p = make_point(3.0, 4.0); return p.x, p.y")
        .await
        .unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_number(), Some(3.0));
    assert_eq!(results[1].as_number(), Some(4.0));
}

#[tokio::test]
async fn test_async_return_userdata_method() {
    let mut vm = new_vm();

    vm.register_async("make_point", |args| async move {
        let x = args[0].as_number().unwrap_or(0.0);
        let y = args[1].as_number().unwrap_or(0.0);
        Ok(vec![AsyncReturnValue::userdata(AsyncPoint { x, y })])
    })
    .unwrap();

    // Test method call on the returned userdata
    let results = vm
        .execute_async("local p = make_point(10, 20); return p:sum()")
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_number(), Some(30.0));
}

#[tokio::test]
async fn test_async_return_userdata_tostring() {
    let mut vm = new_vm();

    vm.register_async("make_point", |args| async move {
        let x = args[0].as_number().unwrap_or(0.0);
        let y = args[1].as_number().unwrap_or(0.0);
        Ok(vec![AsyncReturnValue::userdata(AsyncPoint { x, y })])
    })
    .unwrap();

    // Test __tostring metamethod
    let results = vm
        .execute_async("local p = make_point(1, 2); return tostring(p)")
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("AsyncPoint(1, 2)"));
}

#[tokio::test]
async fn test_async_return_userdata_via_from() {
    let mut vm = new_vm();

    // Test using From<LuaUserdata> conversion
    vm.register_async("make_point", |_args| async move {
        let ud = LuaUserdata::new(AsyncPoint { x: 5.0, y: 6.0 });
        Ok(vec![AsyncReturnValue::from(ud)])
    })
    .unwrap();

    let results = vm
        .execute_async("local p = make_point(); return p.x + p.y")
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_number(), Some(11.0));
}

#[tokio::test]
async fn test_async_return_mixed_userdata_and_values() {
    let mut vm = new_vm();

    // Return userdata alongside other types
    vm.register_async("make_result", |_args| async move {
        Ok(vec![
            AsyncReturnValue::userdata(AsyncPoint { x: 1.0, y: 2.0 }),
            AsyncReturnValue::string("ok"),
            AsyncReturnValue::integer(42),
        ])
    })
    .unwrap();

    let results = vm
        .execute_async("local p, s, n = make_result(); return p.x, s, n")
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_number(), Some(1.0));
    assert_eq!(results[1].as_str(), Some("ok"));
    assert_eq!(results[2].as_integer(), Some(42));
}
