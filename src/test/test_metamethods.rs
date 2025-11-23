// Comprehensive metamethod tests
use crate::*;

#[test]
fn test_add_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}
        local b = {val = 3}
        setmetatable(a, {__add = function(x, y) return {val = x.val + y.val} end})
        setmetatable(b, {__add = function(x, y) return {val = x.val + y.val} end})
        local c = a + b
        assert(c.val == 8)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_add_metamethod_with_number() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}
        setmetatable(a, {__add = function(x, y) 
            if type(y) == "number" then
                return {val = x.val + y}
            else
                return {val = x.val + y.val}
            end
        end})
        local c = a + 10
        assert(c.val == 15)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_sub_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 10}
        local b = {val = 3}
        setmetatable(a, {__sub = function(x, y) return {val = x.val - y.val} end})
        setmetatable(b, {__sub = function(x, y) return {val = x.val - y.val} end})
        local c = a - b
        assert(c.val == 7)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_mul_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}
        setmetatable(a, {__mul = function(x, y) return {val = x.val * y} end})
        local c = a * 3
        assert(c.val == 15)
    "#);
    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_div_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 20}
        setmetatable(a, {__div = function(x, y) return {val = x.val / y} end})
        local c = a / 4
        assert(c.val == 5)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_mod_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 17}
        setmetatable(a, {__mod = function(x, y) return {val = x.val % y} end})
        local c = a % 5
        assert(c.val == 2)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_pow_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 2}
        setmetatable(a, {__pow = function(x, y) return {val = x.val ^ y} end})
        local c = a ^ 4
        assert(c.val == 16)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_unm_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 10}
        setmetatable(a, {__unm = function(x) return {val = -x.val} end})
        local c = -a
        assert(c.val == -10)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_idiv_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 17}
        setmetatable(a, {__idiv = function(x, y) return {val = x.val // y} end})
        local c = a // 5
        assert(c.val == 3)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_band_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}  -- 101
        local b = {val = 3}  -- 011
        setmetatable(a, {__band = function(x, y) return {val = x.val & y.val} end})
        setmetatable(b, {__band = function(x, y) return {val = x.val & y.val} end})
        local c = a & b  -- 001 = 1
        assert(c.val == 1)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_bor_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}  -- 101
        local b = {val = 3}  -- 011
        setmetatable(a, {__bor = function(x, y) return {val = x.val | y.val} end})
        setmetatable(b, {__bor = function(x, y) return {val = x.val | y.val} end})
        local c = a | b  -- 111 = 7
        assert(c.val == 7)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_bxor_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}  -- 101
        local b = {val = 3}  -- 011
        setmetatable(a, {__bxor = function(x, y) return {val = x.val ~ y.val} end})
        setmetatable(b, {__bxor = function(x, y) return {val = x.val ~ y.val} end})
        local c = a ~ b  -- 110 = 6
        assert(c.val == 6)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_bnot_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}
        setmetatable(a, {__bnot = function(x) return {val = ~x.val} end})
        local c = ~a
        assert(c.val == ~5)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_shl_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}
        setmetatable(a, {__shl = function(x, n) return {val = x.val << n} end})
        local c = a << 2
        assert(c.val == 20)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_shr_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 20}
        setmetatable(a, {__shr = function(x, n) return {val = x.val >> n} end})
        local c = a >> 2
        assert(c.val == 5)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_index_metamethod_function() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local t = {}
        setmetatable(t, {
            __index = function(tbl, key)
                return "value_" .. key
            end
        })
        assert(t.foo == "value_foo")
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_index_metamethod_table() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local defaults = {x = 10, y = 20}
        local t = {}
        setmetatable(t, {__index = defaults})
        assert(t.x == 10 and t.y == 20)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_newindex_metamethod_function() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local storage = {}
        local t = {}
        setmetatable(t, {
            __newindex = function(tbl, key, value)
                storage[key] = value * 2
            end,
            __index = storage
        })
        t.val = 5
        assert(storage.val == 10)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_newindex_metamethod_table() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local storage = {}
        local t = {}
        setmetatable(t, {__newindex = storage, __index = storage})
        t.name = "test"
        assert(storage.name == "test")
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_call_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local t = {}
        setmetatable(t, {
            __call = function(tbl, a, b)
                return a + b
            end
        })
        local result = t(3, 5)
        assert(result == 8)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_tostring_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local t = {x = 1, y = 2}
        setmetatable(t, {
            __tostring = function(tbl)
                return "Point(" .. tbl.x .. "," .. tbl.y .. ")"
            end
        })
        assert(tostring(t) == "Point(1,2)")
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_len_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local t = {a = 1, b = 2, c = 3}
        setmetatable(t, {
            __len = function(tbl)
                local count = 0
                for _ in pairs(tbl) do
                    count = count + 1
                end
                return count
            end
        })
        assert(#t == 3)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_eq_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 5}
        local b = {val = 5}
        local mt = {__eq = function(x, y) return x.val == y.val end}
        setmetatable(a, mt)
        setmetatable(b, mt)
        assert(a == b)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_lt_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 3}
        local b = {val = 5}
        setmetatable(a, {__lt = function(x, y) return x.val < y.val end})
        setmetatable(b, {__lt = function(x, y) return x.val < y.val end})
        assert(a < b)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_le_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {val = 3}
        local b = {val = 3}
        setmetatable(a, {__le = function(x, y) return x.val <= y.val end})
        setmetatable(b, {__le = function(x, y) return x.val <= y.val end})
        assert(a <= b)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_concat_metamethod() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local a = {str = "Hello"}
        local b = {str = "World"}
        setmetatable(a, {__concat = function(x, y) return {str = x.str .. " " .. y.str} end})
        setmetatable(b, {__concat = function(x, y) return {str = x.str .. " " .. y.str} end})
        local c = a .. b
        assert(c.str == "Hello World")
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_nested_index_chain() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local level1 = {a = 1}
        local level2 = {}
        local level3 = {}
        setmetatable(level2, {__index = level1})
        setmetatable(level3, {__index = level2})
        assert(level3.a == 1)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_multiple_metamethods_same_object() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local mt = {
            __add = function(a, b) 
                local result = {x = a.x + 5}
                setmetatable(result, mt)
                return result
            end,
            __sub = function(a, b) 
                local result = {x = a.x - 3}
                setmetatable(result, mt)
                return result
            end,
            __tostring = function(o) return "Obj(" .. o.x .. ")" end
        }
        local obj = {x = 10}
        setmetatable(obj, mt)
        local r1 = obj + obj
        local r2 = obj - obj
        local s = tostring(obj)
        assert(r1.x == 15 and r2.x == 7 and s == "Obj(10)")
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_string_metatable_len() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local s = "hello"
        assert(s:len() == 5)
        assert(#s == 5)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_string_metatable_upper() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local s = "hello"
        assert(s:upper() == "HELLO")
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_string_metatable_lower() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local s = "HELLO"
        assert(s:lower() == "hello")
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_string_metatable_sub() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local s = "hello world"
        assert(s:sub(1, 5) == "hello")
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_getmetatable_table() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local t = {}
        local mt = {__index = {x = 1}}
        setmetatable(t, mt)
        assert(getmetatable(t) == mt)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_getmetatable_string() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local s = "hello"
        local mt = getmetatable(s)
        assert(mt ~= nil)
        assert(mt.__index == string)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_arithmetic_metamethod_chain() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local mt = {
            __add = function(a, b) 
                local result = {n = a.n + b.n}
                setmetatable(result, getmetatable(a))
                return result
            end,
            __mul = function(a, b) 
                local result = {n = a.n * b.n}
                setmetatable(result, getmetatable(a))
                return result
            end
        }
        local t = {n = 2}
        setmetatable(t, mt)
        local r = (t + t) * t
        assert(r.n == 8)  -- (2 + 2) * 2
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_index_with_rawget() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local t = {real = 1}
        setmetatable(t, {
            __index = function(tbl, key)
                return "meta_" .. key
            end
        })
        assert(t.real == 1)
        assert(t.fake == "meta_fake")
        assert(rawget(t, "real") == 1)
        assert(rawget(t, "fake") == nil)
    "#);
    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_newindex_with_rawset() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local storage = {}
        local t = {}
        setmetatable(t, {
            __newindex = function(tbl, key, value)
                storage[key] = value
            end
        })
        t.a = 1
        rawset(t, "b", 2)
        assert(storage.a == 1)
        assert(storage.b == nil)
        assert(rawget(t, "b") == 2)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_metatable_protection() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local t = {}
        local mt = {
            __metatable = "protected"
        }
        setmetatable(t, mt)
        assert(getmetatable(t) == "protected")
        local success = pcall(setmetatable, t, {})
        assert(not success)
    "#);
    assert!(result.is_ok());
}

#[test]
fn test_mode_weak_tables() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        local t = {}
        setmetatable(t, {__mode = "k"})
        local key = {}
        t[key] = "value"
        assert(t[key] == "value")
    "#);
    assert!(result.is_ok());
}
