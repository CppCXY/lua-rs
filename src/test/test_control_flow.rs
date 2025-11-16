// Tests for control flow structures
use crate::*;

#[test]
fn test_if_else() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local x = 10
        local result = ""
        
        if x > 5 then
            result = "greater"
        else
            result = "less or equal"
        end
        
        assert(result == "greater")
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_if_elseif_else() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local function classify(x)
            if x > 0 then
                return "positive"
            elseif x < 0 then
                return "negative"
            else
                return "zero"
            end
        end
        
        assert(classify(5) == "positive")
        assert(classify(-5) == "negative")
        assert(classify(0) == "zero")
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_while_loop() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local i = 0
        local sum = 0
        
        while i < 5 do
            i = i + 1
            sum = sum + i
        end
        
        assert(i == 5)
        assert(sum == 15)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_repeat_until() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local i = 0
        
        repeat
            i = i + 1
        until i >= 5
        
        assert(i == 5)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_numeric_for() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local sum = 0
        for i = 1, 10 do
            sum = sum + i
        end
        assert(sum == 55)
        
        local sum2 = 0
        for i = 1, 10, 2 do
            sum2 = sum2 + i
        end
        assert(sum2 == 25)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_generic_for() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local t = {10, 20, 30}
        local sum = 0
        
        for i, v in ipairs(t) do
            sum = sum + v
        end
        
        assert(sum == 60)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_break() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local i = 0
        while true do
            i = i + 1
            if i >= 5 then
                break
            end
        end
        
        assert(i == 5)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_nested_loops() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local count = 0
        
        for i = 1, 3 do
            for j = 1, 3 do
                count = count + 1
            end
        end
        
        assert(count == 9)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_goto() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local x = 0
        ::start::
        x = x + 1
        if x < 5 then
            goto start
        end
        assert(x == 5)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_do_block() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local x = 10
        do
            local x = 20
            assert(x == 20)
        end
        assert(x == 10)
    "#);
    
    assert!(result.is_ok());
}

#[test]
fn test_conditional_expressions() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    
    let result = vm.execute_string(r#"
        local function ternary(cond, a, b)
            if cond then return a else return b end
        end
        
        assert(ternary(true, "yes", "no") == "yes")
        assert(ternary(false, "yes", "no") == "no")
    "#);
    
    assert!(result.is_ok());
}
