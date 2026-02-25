// Tests for table library functions
use crate::*;

#[test]
fn test_table_insert() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
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

    let result = vm.execute(
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

    let result = vm.execute(
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

    let result = vm.execute(
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

    let result = vm.execute(
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

    let result = vm.execute(
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

    let result = vm.execute(
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

    let result = vm.execute(
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

    let result = vm.execute(
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

// ===== Table library metamethod tests =====

#[test]
fn test_table_insert_with_metamethods() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        -- Proxy table: all reads/writes go to backing table via __index/__newindex
        local backing = {}
        local proxy = setmetatable({}, {
            __len = function() return #backing end,
            __index = backing,
            __newindex = backing,
        })

        table.insert(proxy, 10)
        table.insert(proxy, 20)
        table.insert(proxy, 1, 5)

        assert(backing[1] == 5, "insert metamethod: backing[1]")
        assert(backing[2] == 10, "insert metamethod: backing[2]")
        assert(backing[3] == 20, "insert metamethod: backing[3]")
        assert(#backing == 3, "insert metamethod: length")
    "#,
    );

    assert!(result.is_ok(), "table.insert with metamethods failed: {:?}", result);
}

#[test]
fn test_table_remove_with_metamethods() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local backing = {10, 20, 30, 40}
        local proxy = setmetatable({}, {
            __len = function() return #backing end,
            __index = backing,
            __newindex = backing,
        })

        local removed = table.remove(proxy, 2)
        assert(removed == 20, "remove metamethod: removed value")
        assert(backing[1] == 10, "remove metamethod: backing[1]")
        assert(backing[2] == 30, "remove metamethod: backing[2]")
        assert(backing[3] == 40, "remove metamethod: backing[3]")
    "#,
    );

    assert!(result.is_ok(), "table.remove with metamethods failed: {:?}", result);
}

#[test]
fn test_table_sort_with_metamethods() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local backing = {5, 3, 1, 4, 2}
        local proxy = setmetatable({}, {
            __len = function() return #backing end,
            __index = backing,
            __newindex = backing,
        })

        table.sort(proxy)
        for i = 1, 5 do
            assert(backing[i] == i, "sort metamethod: backing[" .. i .. "]")
        end
    "#,
    );

    assert!(result.is_ok(), "table.sort with metamethods failed: {:?}", result);
}

#[test]
fn test_table_concat_with_metamethods() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        -- __index function that returns computed values
        local t = setmetatable({}, {
            __index = function(_, k) return k + 1 end,
            __len = function() return 5 end,
        })

        local s = table.concat(t, ";")
        assert(s == "2;3;4;5;6", "concat metamethod: got " .. s)
    "#,
    );

    assert!(result.is_ok(), "table.concat with metamethods failed: {:?}", result);
}

#[test]
fn test_table_unpack_with_metamethods() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local backing = {100, 200, 300}
        local proxy = setmetatable({}, {
            __len = function() return #backing end,
            __index = backing,
        })

        local a, b, c = table.unpack(proxy)
        assert(a == 100 and b == 200 and c == 300,
               "unpack metamethod: " .. tostring(a) .. "," .. tostring(b) .. "," .. tostring(c))
    "#,
    );

    assert!(result.is_ok(), "table.unpack with metamethods failed: {:?}", result);
}

#[test]
fn test_table_move_with_metamethods() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local src = setmetatable({}, {
            __index = {10, 20, 30, 40, 50},
        })
        local dst = {}

        table.move(src, 1, 5, 1, dst)
        assert(dst[1] == 10 and dst[3] == 30 and dst[5] == 50,
               "move metamethod failed")
    "#,
    );

    assert!(result.is_ok(), "table.move with metamethods failed: {:?}", result);
}

#[test]
fn test_table_insert_overflow_with_len_metamethod() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
        r#"
        local t = setmetatable({},
                    {__len = function() return math.maxinteger end})
        table.insert(t, 42)
        local k, v = next(t)
        assert(k == math.mininteger and v == 42,
               "insert overflow: k=" .. tostring(k) .. " v=" .. tostring(v))
    "#,
    );

    assert!(result.is_ok(), "table.insert overflow with __len failed: {:?}", result);
}

#[test]
fn test_table_full_proxy_workflow() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Full workflow matching the C Lua test: insert, sort, concat, remove, unpack
    let result = vm.execute(
        r#"
        local t = {}
        local proxy = setmetatable({}, {
            __len = function() return #t end,
            __index = t,
            __newindex = t,
        })

        for i = 1, 10 do
            table.insert(proxy, 1, i)
        end
        assert(#proxy == 10 and #t == 10)

        table.sort(proxy)
        for i = 1, 10 do
            assert(t[i] == i and proxy[i] == i)
        end

        assert(table.concat(proxy, ",") == "1,2,3,4,5,6,7,8,9,10")

        for i = 1, 8 do
            assert(table.remove(proxy, 1) == i)
        end
        assert(#proxy == 2 and #t == 2)

        local a, b, c = table.unpack(proxy)
        assert(a == 9 and b == 10 and c == nil)
    "#,
    );

    assert!(result.is_ok(), "full proxy workflow failed: {:?}", result);
}

#[test]
fn test_table_newindex_counting() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    // Test that __newindex is actually triggered (counting calls)
    let result = vm.execute(
        r#"
        local count = 0
        local t = setmetatable({}, {
            __newindex = function(tbl, k, v)
                count = count + 1
                rawset(tbl, k, v)
            end
        })

        for i = 1, 10 do
            table.insert(t, 1, i)
        end

        -- Only the first 10 inserts trigger __newindex (new keys)
        assert(count == 10, "expected 10 __newindex calls, got " .. count)

        table.sort(t)
        for i = 1, 10 do
            assert(t[i] == i)
        end
    "#,
    );

    assert!(result.is_ok(), "newindex counting failed: {:?}", result);
}
