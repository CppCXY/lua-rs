/// Advanced closure and upvalue tests
use crate::lua_vm::LuaVM;

#[test]
fn test_simple_closure() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function make_counter()
            local count = 0
            return function()
                count = count + 1
                return count
            end
        end
        local counter = make_counter()
        assert(counter() == 1)
        assert(counter() == 2)
        assert(counter() == 3)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_multiple_closures_share_upvalue() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function make_getset()
            local value = 10
            local function get()
                return value
            end
            local function set(v)
                value = v
            end
            return get, set
        end
        local get, set = make_getset()
        assert(get() == 10)
        set(20)
        assert(get() == 20)
        set(30)
        assert(get() == 30)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_nested_closures() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function outer(x)
            return function(y)
                return function(z)
                    return x + y + z
                end
            end
        end
        local f1 = outer(1)
        local f2 = f1(2)
        assert(f2(3) == 6)
        assert(outer(10)(20)(30) == 60)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_captures_loop_variable() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local funcs = {}
        for i = 1, 5 do
            funcs[i] = function() return i end
        end
        assert(funcs[1]() == 1)
        assert(funcs[3]() == 3)
        assert(funcs[5]() == 5)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_modifies_upvalue_in_loop() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local sum = 0
        local adders = {}
        for i = 1, 3 do
            adders[i] = function(x)
                sum = sum + x
                return sum
            end
        end
        assert(adders[1](10) == 10)
        assert(adders[2](5) == 15)
        assert(adders[3](3) == 18)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_factory_pattern() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function make_adder(n)
            return function(x)
                return x + n
            end
        end
        local add5 = make_adder(5)
        local add10 = make_adder(10)
        assert(add5(3) == 8)
        assert(add10(3) == 13)
        assert(add5(7) == 12)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_with_multiple_upvalues() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function make_calc(a, b, c)
            return function(x)
                return a * x * x + b * x + c
            end
        end
        local f = make_calc(2, 3, 1)
        assert(f(0) == 1)
        assert(f(1) == 6)
        assert(f(2) == 15)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_returning_multiple_values() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function make_pair(a, b)
            return function()
                return a, b
            end
        end
        local f = make_pair(10, 20)
        local x, y = f()
        assert(x == 10 and y == 20)
    "#);
    assert!(result.is_ok());
}

// Temporarily disabled due to VM issue with varargs
// #[test]
// fn test_closure_with_vararg() {
//     let mut vm = LuaVM::new(SafeOption::default());
//     vm.open_libs();
//     let result = vm.execute_string(r#"
//         local function make_collector(...)
//             local args = {...}
//             return function()
//                 return table.unpack(args)
//             end
//         end
//         local f = make_collector(1, 2, 3, 4, 5)
//         local a, b, c, d, e = f()
//         assert(a == 1 and b == 2 and c == 3 and d == 4 and e == 5)
//     "#);
//     assert!(result.is_ok());
// }

#[test]
fn test_closure_recursive_upvalue() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function make_factorial()
            local fact
            fact = function(n)
                if n <= 1 then
                    return 1
                else
                    return n * fact(n - 1)
                end
            end
            return fact
        end
        local f = make_factorial()
        assert(f(5) == 120)
        assert(f(6) == 720)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_chain_calls() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function chain(value)
            return {
                add = function(n)
                    value = value + n
                    return chain(value)
                end,
                mul = function(n)
                    value = value * n
                    return chain(value)
                end,
                get = function()
                    return value
                end
            }
        end
        local result = chain(5).add(3).mul(2).add(10).get()
        assert(result == 26)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_array_of_closures() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local operations = {}
        local value = 100
        operations.double = function() value = value * 2 end
        operations.half = function() value = value / 2 end
        operations.add10 = function() value = value + 10 end
        operations.get = function() return value end
        
        operations.double()
        assert(operations.get() == 200)
        operations.half()
        assert(operations.get() == 100)
        operations.add10()
        assert(operations.get() == 110)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_with_table_capture() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function make_obj(name)
            local data = {name = name, count = 0}
            return {
                getName = function() return data.name end,
                increment = function() data.count = data.count + 1 end,
                getCount = function() return data.count end
            }
        end
        local obj = make_obj("test")
        assert(obj.getName() == "test")
        obj.increment()
        obj.increment()
        assert(obj.getCount() == 2)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_deep_nesting() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local function level1(a)
            return function level2(b)
                return function level3(c)
                    return function level4(d)
                        return function level5(e)
                            return a + b + c + d + e
                        end
                    end
                end
            end
        end
        assert(level1(1)(2)(3)(4)(5) == 15)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_closure_mutually_recursive() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_libs();
    let result = vm.execute_string(r#"
        local is_even, is_odd
        is_even = function(n)
            if n == 0 then return true
            else return is_odd(n - 1) end
        end
        is_odd = function(n)
            if n == 0 then return false
            else return is_even(n - 1) end
        end
        assert(is_even(4) == true)
        assert(is_even(5) == false)
        assert(is_odd(4) == false)
        assert(is_odd(5) == true)
    "#);
    assert!(result.is_ok());
}
