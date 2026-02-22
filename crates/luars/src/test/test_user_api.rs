// Tests for the user-facing Ref API and related features
use crate::lua_vm::SafeOption;
use crate::{LuaVM, LuaValue, Stdlib};

// ============================================================================
// LuaTableRef tests
// ============================================================================

#[test]
fn test_table_ref_basic() {
    let mut vm = LuaVM::new(SafeOption::default());
    let tbl = vm.create_table_ref(0, 4).unwrap();

    // Set and get string keys
    let hello = vm.create_string("hello").unwrap();
    tbl.set("greeting", hello).unwrap();
    let val = tbl.get("greeting").unwrap();
    assert_eq!(val.as_str(), Some("hello"));

    // Set and get integer keys
    tbl.seti(1, LuaValue::integer(42)).unwrap();
    let val = tbl.geti(1).unwrap();
    assert_eq!(val.as_integer(), Some(42));

    // Set and get via LuaValue key
    let key = vm.create_string("key2").unwrap();
    tbl.set_value(key.clone(), LuaValue::boolean(true)).unwrap();
    let val = tbl.get_value(&key).unwrap();
    assert_eq!(val.as_boolean(), Some(true));
}

#[test]
fn test_table_ref_get_as() {
    let mut vm = LuaVM::new(SafeOption::default());
    let tbl = vm.create_table_ref(0, 4).unwrap();

    tbl.set("count", LuaValue::integer(99)).unwrap();
    let count: i64 = tbl.get_as("count").unwrap();
    assert_eq!(count, 99);

    tbl.set("pi", LuaValue::float(3.14)).unwrap();
    let pi: f64 = tbl.get_as("pi").unwrap();
    assert!((pi - 3.14).abs() < 1e-10);
}

#[test]
fn test_table_ref_pairs_and_len() {
    let mut vm = LuaVM::new(SafeOption::default());
    let tbl = vm.create_table_ref(4, 0).unwrap();

    tbl.seti(1, LuaValue::integer(10)).unwrap();
    tbl.seti(2, LuaValue::integer(20)).unwrap();
    tbl.seti(3, LuaValue::integer(30)).unwrap();

    assert_eq!(tbl.len().unwrap(), 3);

    let pairs = tbl.pairs().unwrap();
    assert_eq!(pairs.len(), 3);
}

#[test]
fn test_table_ref_push() {
    let mut vm = LuaVM::new(SafeOption::default());
    let tbl = vm.create_table_ref(4, 0).unwrap();

    tbl.push(LuaValue::integer(100)).unwrap();
    tbl.push(LuaValue::integer(200)).unwrap();

    assert_eq!(tbl.len().unwrap(), 2);
    assert_eq!(tbl.geti(1).unwrap().as_integer(), Some(100));
    assert_eq!(tbl.geti(2).unwrap().as_integer(), Some(200));
}

#[test]
fn test_table_ref_from_global() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute(
        r#"
        config = { host = "localhost", port = 8080 }
    "#,
    )
    .unwrap();

    let config = vm.get_global_table("config").unwrap().unwrap();
    let host: String = config.get_as("host").unwrap();
    assert_eq!(host, "localhost");
    let port: i64 = config.get_as("port").unwrap();
    assert_eq!(port, 8080);

    // Modify through ref and verify from Lua
    config.set("port", LuaValue::integer(9090)).unwrap();
    let results = vm.execute("return config.port").unwrap();
    assert_eq!(results[0].as_integer(), Some(9090));
}

#[test]
fn test_table_ref_auto_drop() {
    let mut vm = LuaVM::new(SafeOption::default());

    // Create and drop a table ref — should not leak registry entries
    let ref_id;
    {
        let tbl = vm.create_table_ref(0, 0).unwrap();
        ref_id = tbl.ref_id();
        tbl.set("x", LuaValue::integer(1)).unwrap();
    }
    // After drop, the registry entry should be cleared
    let val = vm.get_ref_value_by_id(ref_id);
    assert!(val.is_nil(), "Registry entry should be released on drop");
}

// ============================================================================
// LuaFunctionRef tests
// ============================================================================

#[test]
fn test_function_ref_call() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute(
        r#"
        function add(a, b)
            return a + b
        end
    "#,
    )
    .unwrap();

    let add = vm.get_global_function("add").unwrap().unwrap();
    let results = add
        .call(vec![LuaValue::integer(3), LuaValue::integer(4)])
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(7));
}

#[test]
fn test_function_ref_call1() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute(
        r#"
        function greet(name)
            return "Hello, " .. name
        end
    "#,
    )
    .unwrap();

    let greet = vm.get_global_function("greet").unwrap().unwrap();
    let name = vm.create_string("World").unwrap();
    let result = greet.call1(vec![name]).unwrap();
    assert_eq!(result.as_str(), Some("Hello, World"));
}

#[test]
fn test_function_ref_multiple_calls() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute(
        r#"
        counter = 0
        function inc()
            counter = counter + 1
            return counter
        end
    "#,
    )
    .unwrap();

    let inc = vm.get_global_function("inc").unwrap().unwrap();
    assert_eq!(inc.call1(vec![]).unwrap().as_integer(), Some(1));
    assert_eq!(inc.call1(vec![]).unwrap().as_integer(), Some(2));
    assert_eq!(inc.call1(vec![]).unwrap().as_integer(), Some(3));
}

#[test]
fn test_function_ref_auto_drop() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute("function noop() end").unwrap();

    let ref_id;
    {
        let f = vm.get_global_function("noop").unwrap().unwrap();
        ref_id = f.ref_id();
    }
    let val = vm.get_ref_value_by_id(ref_id);
    assert!(val.is_nil(), "Function ref should be released on drop");
}

// ============================================================================
// LuaStringRef tests
// ============================================================================

#[test]
fn test_string_ref_basic() {
    let mut vm = LuaVM::new(SafeOption::default());
    let s = vm.create_string("hello world").unwrap();
    let sref = vm.to_string_ref(s).unwrap();

    assert_eq!(sref.as_str(), Some("hello world"));
    assert_eq!(sref.byte_len(), 11);
    assert_eq!(sref.to_string_lossy(), "hello world");
    assert_eq!(format!("{}", sref), "hello world");
}

#[test]
fn test_string_ref_auto_drop() {
    let mut vm = LuaVM::new(SafeOption::default());
    let s = vm.create_string("test_drop").unwrap();
    let ref_id;
    {
        let sref = vm.to_string_ref(s).unwrap();
        ref_id = sref.ref_id();
    }
    let val = vm.get_ref_value_by_id(ref_id);
    assert!(val.is_nil());
}

// ============================================================================
// LuaAnyRef tests
// ============================================================================

#[test]
fn test_any_ref_table() {
    let mut vm = LuaVM::new(SafeOption::default());
    let table = vm.create_table(0, 2).unwrap();
    let any = vm.to_ref(table);

    assert!(matches!(any.kind(), crate::lua_value::LuaValueKind::Table));
    let tbl = any.as_table().unwrap();
    tbl.set("x", LuaValue::integer(42)).unwrap();
    assert_eq!(tbl.get("x").unwrap().as_integer(), Some(42));
}

#[test]
fn test_any_ref_function() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.execute("function f() return 99 end").unwrap();

    let val = vm.get_global("f").unwrap().unwrap();
    let any = vm.to_ref(val);
    let func = any.as_function().unwrap();
    let result = func.call1(vec![]).unwrap();
    assert_eq!(result.as_integer(), Some(99));
}

#[test]
fn test_any_ref_string() {
    let mut vm = LuaVM::new(SafeOption::default());
    let s = vm.create_string("any_string").unwrap();
    let any = vm.to_ref(s);
    let sref = any.as_string().unwrap();
    assert_eq!(sref.as_str(), Some("any_string"));
}

#[test]
fn test_any_ref_wrong_type() {
    let mut vm = LuaVM::new(SafeOption::default());
    let table = vm.create_table(0, 0).unwrap();
    let any = vm.to_ref(table);

    assert!(any.as_function().is_none());
    assert!(any.as_string().is_none());
    assert!(any.as_table().is_some());
}

// ============================================================================
// OpaqueUserData tests
// ============================================================================

#[test]
fn test_push_any_basic() {
    let mut vm = LuaVM::new(SafeOption::default());
    #[allow(dead_code)]
    // Push an arbitrary Rust struct as opaque userdata
    struct MyConfig {
        name: String,
        value: i32,
    }

    let config = MyConfig {
        name: "test".to_string(),
        value: 42,
    };

    let ud = vm.push_any(config).unwrap();
    assert!(ud.is_userdata());
    vm.set_global("my_config", ud).unwrap();

    // Verify type() in Lua shows the Rust type name
    vm.open_stdlib(Stdlib::Basic).unwrap();
    let results = vm.execute("return type(my_config)").unwrap();
    assert_eq!(results[0].as_str(), Some("userdata"));
}

#[test]
fn test_push_any_downcast() {
    let mut vm = LuaVM::new(SafeOption::default());

    #[derive(Debug, PartialEq)]
    struct Point {
        x: f64,
        y: f64,
    }

    let point = Point { x: 3.0, y: 4.0 };
    let ud = vm.push_any(point).unwrap();
    vm.set_global("pt", ud).unwrap();

    // Retrieve via Rust and downcast
    let val = vm.get_global("pt").unwrap().unwrap();
    let ud_data = val.as_userdata_mut().unwrap();
    let recovered = ud_data.downcast_ref::<Point>().unwrap();
    assert_eq!(recovered.x, 3.0);
    assert_eq!(recovered.y, 4.0);
}

#[test]
fn test_push_any_in_callback() {
    let mut vm = LuaVM::new(SafeOption::default());

    struct Counter {
        count: i32,
    }

    let counter = Counter { count: 0 };
    let ud = vm.push_any(counter).unwrap();
    vm.set_global("counter", ud).unwrap();

    // Register a function that mutates the opaque userdata
    vm.register_function("increment", |state| {
        let ud_val = state.get_arg(1).unwrap();
        let ud = ud_val.as_userdata_mut().unwrap();
        let c = ud.downcast_mut::<Counter>().unwrap();
        c.count += 1;
        state.push_value(LuaValue::integer(c.count as i64))?;
        Ok(1)
    })
    .unwrap();

    let results = vm.execute("return increment(counter)").unwrap();
    assert_eq!(results[0].as_integer(), Some(1));

    let results = vm.execute("return increment(counter)").unwrap();
    assert_eq!(results[0].as_integer(), Some(2));
}

// ============================================================================
// UserDataBuilder tests
// ============================================================================

#[test]
fn test_userdata_builder_fields() {
    use crate::lua_value::UserDataBuilder;

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::Basic).unwrap();

    struct Address {
        ip: String,
        port: u16,
    }

    let addr = Address {
        ip: "127.0.0.1".to_owned(),
        port: 8080,
    };

    let ud = UserDataBuilder::new(addr)
        .set_type_name("Address")
        .add_field_getter("ip", |a| crate::UdValue::Str(a.ip.clone()))
        .add_field_getter("port", |a| crate::UdValue::Integer(a.port as i64))
        .build(&mut vm)
        .unwrap();

    vm.set_global("addr", ud).unwrap();

    let results = vm.execute("return addr.ip").unwrap();
    assert_eq!(results[0].as_str(), Some("127.0.0.1"));

    let results = vm.execute("return addr.port").unwrap();
    assert_eq!(results[0].as_integer(), Some(8080));
}

#[test]
fn test_userdata_builder_tostring() {
    use crate::lua_value::UserDataBuilder;

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::Basic).unwrap();

    struct Version {
        major: u32,
        minor: u32,
        patch: u32,
    }

    let ver = Version {
        major: 1,
        minor: 2,
        patch: 3,
    };

    let ud = UserDataBuilder::new(ver)
        .set_type_name("Version")
        .set_tostring(|v| format!("{}.{}.{}", v.major, v.minor, v.patch))
        .build(&mut vm)
        .unwrap();

    vm.set_global("ver", ud).unwrap();

    let results = vm.execute("return tostring(ver)").unwrap();
    assert_eq!(results[0].as_str(), Some("1.2.3"));
}

#[test]
fn test_userdata_builder_setter() {
    use crate::lua_value::UserDataBuilder;

    let mut vm = LuaVM::new(SafeOption::default());

    struct Config {
        debug: bool,
    }

    let cfg = Config { debug: false };

    let ud = UserDataBuilder::new(cfg)
        .add_field_getter("debug", |c| crate::UdValue::Boolean(c.debug))
        .add_field_setter("debug", |c, v| {
            c.debug = v.to_bool();
            Ok(())
        })
        .build(&mut vm)
        .unwrap();

    vm.set_global("cfg", ud).unwrap();

    // Read default
    let results = vm.execute("return cfg.debug").unwrap();
    assert_eq!(results[0].as_boolean(), Some(false));

    // Set and read back
    vm.execute("cfg.debug = true").unwrap();
    let results = vm.execute("return cfg.debug").unwrap();
    assert_eq!(results[0].as_boolean(), Some(true));
}

// ============================================================================
// build_table_ref / TableBuilder integration
// ============================================================================

#[test]
fn test_build_table_ref() {
    use crate::TableBuilder;

    let mut vm = LuaVM::new(SafeOption::default());

    let name_str = vm.create_string("Alice").unwrap();
    let tbl = vm
        .build_table_ref(
            TableBuilder::new()
                .set("name", name_str)
                .set("age", LuaValue::integer(30)),
        )
        .unwrap();

    let name: String = tbl.get_as("name").unwrap();
    assert_eq!(name, "Alice");
    let age: i64 = tbl.get_as("age").unwrap();
    assert_eq!(age, 30);
}

// ============================================================================
// to_ref / to_table_ref / to_function_ref type checks
// ============================================================================

#[test]
fn test_to_ref_type_mismatch() {
    let mut vm = LuaVM::new(SafeOption::default());
    let table = vm.create_table(0, 0).unwrap();

    // Table → to_function_ref should return None
    assert!(vm.to_function_ref(table).is_none());

    // Integer → to_table_ref should return None
    assert!(vm.to_table_ref(LuaValue::integer(42)).is_none());
}

#[test]
fn test_get_global_nonexistent() {
    let mut vm = LuaVM::new(SafeOption::default());
    assert!(vm.get_global_table("nonexistent").unwrap().is_none());
    assert!(vm.get_global_function("nonexistent").unwrap().is_none());
}
