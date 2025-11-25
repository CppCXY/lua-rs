// Comprehensive Lua syntax tests
use crate::*;

// === Variable and Scope Tests ===

#[test]
fn test_local_variable_scope() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local x = 10
        do
            local x = 20
            assert(x == 20)
        end
        assert(x == 10)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_global_variable() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        globalVar = 42
        assert(globalVar == 42)
        assert(_G.globalVar == 42)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_multiple_assignment() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local a, b, c = 1, 2, 3
        assert(a == 1 and b == 2 and c == 3)
        local x, y = 10
        assert(x == 10 and y == nil)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_swap_variables() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local a, b = 1, 2
        a, b = b, a
        assert(a == 2 and b == 1)
    "#,
    );
    assert!(result.is_ok());
}

// === Vararg Tests ===

#[test]
fn test_vararg_function() {
    let mut vm = LuaVM::new();
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
        local result = sum(1, 2, 3, 4, 5)
        assert(result == 15)
    "#,
    );
    assert!(result.is_ok());
}

// === Table Constructor Tests ===

#[test]
fn test_table_constructor_array() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {10, 20, 30}
        assert(t[1] == 10 and t[2] == 20 and t[3] == 30)
        assert(#t == 3)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_table_constructor_hash() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {x = 1, y = 2, z = 3}
        assert(t.x == 1 and t.y == 2 and t.z == 3)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_table_constructor_mixed() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {10, 20, x = "a", y = "b", 30}
        assert(t[1] == 10 and t[2] == 20 and t[3] == 30)
        assert(t.x == "a" and t.y == "b")
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_table_constructor_expression_key() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local key = "mykey"
        local t = {[key] = 100, [1+1] = 200}
        assert(t.mykey == 100 and t[2] == 200)
    "#,
    );
    assert!(result.is_ok());
}

// === Function Tests ===

#[test]
fn test_simple_function() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local function add(a, b)
            return a + b
        end
        local result = add(3, 5)
        assert(result == 8)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_function_as_table_method() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local obj = {value = 10}
        function obj:add(n)
            self.value = self.value + n
        end
        obj:add(5)
        assert(obj.value == 15)
    "#,
    );
    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

// === Loop Tests ===

#[test]
fn test_while_loop() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local i = 0
        local sum = 0
        while i < 5 do
            i = i + 1
            sum = sum + i
        end
        assert(sum == 15)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_repeat_until() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local i = 0
        local sum = 0
        repeat
            i = i + 1
            sum = sum + i
        until i >= 5
        assert(sum == 15)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_numeric_for() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local sum = 0
        for i = 1, 5 do
            sum = sum + i
        end
        assert(sum == 15)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_numeric_for_with_step() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local sum = 0
        for i = 0, 10, 2 do
            sum = sum + i
        end
        assert(sum == 30)  -- 0 + 2 + 4 + 6 + 8 + 10
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_break_statement() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local sum = 0
        for i = 1, 10 do
            if i > 5 then
                break
            end
            sum = sum + i
        end
        assert(sum == 15)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_nested_loops() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local sum = 0
        for i = 1, 3 do
            for j = 1, 3 do
                sum = sum + i * j
            end
        end
        assert(sum == 36)  -- (1+2+3)*1 + (1+2+3)*2 + (1+2+3)*3
    "#,
    );
    assert!(result.is_ok());
}

// === Conditional Tests ===

#[test]
fn test_if_elseif_else() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local function classify(n)
            if n > 0 then
                return "positive"
            elseif n < 0 then
                return "negative"
            else
                return "zero"
            end
        end
        assert(classify(5) == "positive")
        assert(classify(-3) == "negative")
        assert(classify(0) == "zero")
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_truthiness() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert(true)
        assert(not false)
        assert(not nil)
        assert(0)  -- 0 is truthy in Lua
        assert("")  -- empty string is truthy
    "#,
    );
    assert!(result.is_ok());
}

// === Logic Operator Tests ===

#[test]
fn test_and_operator() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert((true and true) == true)
        assert((true and false) == false)
        assert((5 and 10) == 10)
        assert((nil and 10) == nil)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_or_operator() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert((false or true) == true)
        assert((nil or 10) == 10)
        assert((5 or 10) == 5)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_not_operator() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert(not false)
        assert(not nil)
        assert(not (not true))
        assert(not 0 == false)  -- 0 is truthy
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_short_circuit_evaluation() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local called = false
        local function f()
            called = true
            return true
        end
        local result = false and f()
        assert(not called)  -- f() should not be called
    "#,
    );
    assert!(result.is_ok());
}

// === Type Checking Tests ===

#[test]
fn test_type_function() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert(type(nil) == "nil")
        assert(type(true) == "boolean")
        assert(type(42) == "number")
        assert(type("hello") == "string")
        assert(type({}) == "table")
        assert(type(print) == "function")
    "#,
    );
    assert!(result.is_ok());
}

// === Error Handling Tests ===

#[test]
fn test_pcall_success() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local function safe_func()
            return 42
        end
        local success, result = pcall(safe_func)
        assert(success == true)
        assert(result == 42)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_pcall_error() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local function error_func()
            error("oops")
        end
        local success, err = pcall(error_func)
        assert(success == false)
        assert(type(err) == "string")
    "#,
    );
    assert!(result.is_ok());
}

// === Iterator Tests ===

#[test]
fn test_ipairs() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {10, 20, 30}
        local sum = 0
        for i, v in ipairs(t) do
            sum = sum + v
        end
        assert(sum == 60)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_pairs() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {a = 1, b = 2, c = 3}
        local count = 0
        for k, v in pairs(t) do
            count = count + 1
        end
        assert(count == 3)
    "#,
    );
    assert!(result.is_ok());
}

// === Conversion Tests ===

#[test]
fn test_tonumber() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert(tonumber("123") == 123)
        assert(tonumber("3.14") == 3.14)
        assert(tonumber("FF", 16) == 255)
        assert(tonumber("invalid") == nil)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_tostring() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert(tostring(123) == "123")
        assert(tostring(true) == "true")
        assert(tostring(nil) == "nil")
    "#,
    );
    assert!(result.is_ok());
}

// === String Operation Tests ===

#[test]
fn test_string_concatenation() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local s = "Hello" .. " " .. "World"
        assert(s == "Hello World")
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_string_length() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local s = "hello"
        assert(#s == 5)
        assert(string.len(s) == 5)
    "#,
    );
    assert!(result.is_ok());
}

// === Table Operation Tests ===

#[test]
fn test_table_insert() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {1, 2, 3}
        table.insert(t, 4)
        assert(#t == 4 and t[4] == 4)
        table.insert(t, 2, 99)
        assert(t[2] == 99)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_table_remove() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {1, 2, 3, 4}
        local v = table.remove(t)
        assert(v == 4 and #t == 3)
        v = table.remove(t, 2)
        assert(v == 2)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_table_concat() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {"a", "b", "c"}
        assert(table.concat(t) == "abc")
        assert(table.concat(t, ",") == "a,b,c")
    "#,
    );
    assert!(result.is_ok());
}

// === Upvalue Tests ===

#[test]
fn test_simple_upvalue() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local x = 10
        local function get_x()
            return x
        end
        assert(get_x() == 10)
        x = 20
        assert(get_x() == 20)
    "#,
    );
    assert!(result.is_ok());
}

// === Nil Handling Tests ===

#[test]
fn test_nil_in_table() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        local t = {1, 2, nil, 4}
        -- In Lua 5.4, length with holes is undefined but typically gives last valid index
        -- assert(#t == 4)  -- Most likely, but not guaranteed
        assert(t[1] == 1)
        assert(t[2] == 2)
        assert(t[3] == nil)
        assert(t[4] == 4)
    "#,
    );
    assert!(result.is_ok());
}

// === Comparison Tests ===

#[test]
fn test_equality_comparison() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert(5 == 5)
        assert(5 ~= 6)
        assert("hello" == "hello")
        local t1 = {}
        local t2 = {}
        assert(t1 == t1)
        assert(t1 ~= t2)
    "#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_relational_comparison() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(
        r#"
        assert(5 < 10)
        assert(10 > 5)
        assert(5 <= 5)
        assert(5 >= 5)
        assert("a" < "b")
    "#,
    );
    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}
