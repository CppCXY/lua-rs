// Tests for operators and expressions
use crate::*;

#[test]
fn test_arithmetic_operators() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        assert(5 + 3 == 8)
        assert(5 - 3 == 2)
        assert(5 * 3 == 15)
        assert(15 / 3 == 5)
        assert(15 // 4 == 3)
        assert(15 % 4 == 3)
        assert(2 ^ 3 == 8)
        assert(-5 == 0 - 5)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_comparison_operators() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        assert((5 == 5) == true)
        assert((5 ~= 3) == true)
        assert((5 > 3) == true)
        assert((5 < 3) == false)
        assert((5 >= 5) == true)
        assert((5 <= 5) == true)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_logical_operators() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        assert((true and true) == true)
        assert((true and false) == false)
        assert((true or false) == true)
        assert((false or false) == false)
        assert((not true) == false)
        assert((not false) == true)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_logical_short_circuit() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local x = false or 10
        assert(x == 10)
        
        local y = true and 20
        assert(y == 20)
        
        local z = nil or "default"
        assert(z == "default")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_concat_operator() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        assert("hello" .. " " .. "world" == "hello world")
        assert("num: " .. 42 == "num: 42")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_length_operator() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        assert(#"hello" == 5)
        assert(#{1, 2, 3, 4, 5} == 5)
        assert(#{} == 0)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_bitwise_operators() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        assert((5 & 3) == 1)
        assert((5 | 3) == 7)
        assert((5 ~ 3) == 6)
        assert((5 << 1) == 10)
        assert((5 >> 1) == 2)
        assert((~0) == -1)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_operator_precedence() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        assert(2 + 3 * 4 == 14)
        assert((2 + 3) * 4 == 20)
        assert(2 ^ 3 ^ 2 == 512)
        assert((2 ^ 3) ^ 2 == 64)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_constructor() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local t1 = {1, 2, 3}
        assert(t1[1] == 1)
        
        local t2 = {x = 10, y = 20}
        assert(t2.x == 10)
        
        local t3 = {[1] = "a", [2] = "b"}
        assert(t3[1] == "a")
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_function_expressions() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local f = function(x) return x * 2 end
        assert(f(5) == 10)
        
        local t = {
            method = function(self, x)
                return x + 1
            end
        }
        assert(t:method(5) == 6)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_vararg() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local function sum(...)
            local total = 0
            for _, v in ipairs({...}) do
                total = total + v
            end
            return total
        end
        
        assert(sum(1, 2, 3, 4, 5) == 15)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_multiple_assignment() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local a, b, c = 1, 2, 3
        assert(a == 1 and b == 2 and c == 3)
        
        local x, y = 10
        assert(x == 10 and y == nil)
        
        local function multi()
            return 1, 2, 3
        end
        
        local p, q, r = multi()
        assert(p == 1 and q == 2 and r == 3)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_access() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();

    let result = vm.execute_string(
        r#"
        local t = {x = 10}
        assert(t.x == 10)
        assert(t["x"] == 10)
        
        t.y = 20
        assert(t["y"] == 20)
    "#,
    );

    assert!(result.is_ok());
}
