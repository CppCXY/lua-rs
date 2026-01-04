// Tests for table library functions
use crate::*;

#[test]
fn test_table_insert() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {1, 2, 3}
        table.insert(t, 4)
        assert(t[4] == 4)
        
        table.insert(t, 2, 99)
        assert(t[2] == 99)
        assert(t[3] == 2)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_remove() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {10, 20, 30, 40}
        local v = table.remove(t, 2)
        assert(v == 20)
        assert(t[2] == 30)
        assert(#t == 3)
        
        local last = table.remove(t)
        assert(last == 40)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_concat() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {"a", "b", "c"}
        assert(table.concat(t) == "abc")
        assert(table.concat(t, ",") == "a,b,c")
        assert(table.concat(t, "-", 2, 3) == "b-c")
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_table_sort() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {3, 1, 4, 1, 5}
        table.sort(t)
        assert(t[1] == 1)
        assert(t[2] == 1)
        assert(t[3] == 3)
        assert(t[4] == 4)
        assert(t[5] == 5)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_sort_with_comparator() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {3, 1, 4, 1, 5}
        table.sort(t, function(a, b) return a > b end)
        assert(t[1] == 5)
        assert(t[2] == 4)
        assert(t[3] == 3)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_pack() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = table.pack(1, 2, 3)
        assert(t[1] == 1)
        assert(t[2] == 2)
        assert(t[3] == 3)
        assert(t.n == 3)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_unpack() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t = {10, 20, 30}
        local a, b, c = table.unpack(t)
        assert(a == 10)
        assert(b == 20)
        assert(c == 30)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_move() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local t1 = {1, 2, 3, 4, 5}
        local t2 = {}
        table.move(t1, 2, 4, 1, t2)
        assert(t2[1] == 2)
        assert(t2[2] == 3)
        assert(t2[3] == 4)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_table_operations() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        -- Test table creation and access
        local t = {}
        t.name = "test"
        t[1] = "first"
        t["key"] = "value"
        
        assert(t.name == "test")
        assert(t[1] == "first")
        assert(t["key"] == "value")
        
        -- Test length operator
        local arr = {1, 2, 3, 4, 5}
        assert(#arr == 5)
    "#,
    );

    assert!(result.is_ok());
}
