// Tests for the trait-based userdata system
use crate::lua_value::LuaUserdata;
use crate::lua_value::userdata_trait::{UdValue, UserDataTrait};
use crate::*;
use std::fmt;

// ==================== Test structs ====================

/// A simple 2D point — demonstrates field access and metamethods
#[derive(LuaUserData, PartialEq, PartialOrd)]
#[lua_impl(Display, PartialEq, PartialOrd)]
struct Point {
    pub x: f64,
    pub y: f64,
    /// Internal — not exposed to Lua because it's private
    _id: u32,
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Point({}, {})", self.x, self.y)
    }
}

/// Methods exposed to Lua via `#[lua_methods]`
#[lua_methods]
impl Point {
    /// Constructor — creates a new Point
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y, _id: 0 }
    }

    /// Euclidean distance from origin
    pub fn distance(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Translate the point by (dx, dy) — mutating method
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
    }

    /// Scale both coordinates — returns self for testing
    pub fn scale(&mut self, factor: f64) {
        self.x *= factor;
        self.y *= factor;
    }

    /// Method with optional parameter
    pub fn greet(&self, name: Option<String>) -> String {
        match name {
            Some(n) => format!("Hello {} from Point({}, {})", n, self.x, self.y),
            None => format!("Hello from Point({}, {})", self.x, self.y),
        }
    }

    /// Method returning Result
    pub fn checked_div(&self, divisor: f64) -> Result<f64, String> {
        if divisor == 0.0 {
            Err("division by zero".to_string())
        } else {
            Ok(self.x / divisor)
        }
    }
}

#[allow(unused)]
/// Demonstrates readonly and skip attributes
#[derive(LuaUserData)]
struct Config {
    pub name: String,
    #[lua(readonly)]
    pub version: i64,
    #[lua(skip)]
    pub secret: String,
    #[lua(name = "count")]
    pub item_count: u32,
}

// ==================== Trait implementation tests ====================

#[test]
fn test_userdata_trait_type_name() {
    let p = Point {
        x: 1.0,
        y: 2.0,
        _id: 0,
    };
    assert_eq!(p.type_name(), "Point");
}

#[test]
fn test_userdata_trait_get_field() {
    let p = Point {
        x: 3.0,
        y: 4.0,
        _id: 42,
    };

    // Public fields should be accessible
    assert!(matches!(p.get_field("x"), Some(UdValue::Number(n)) if n == 3.0));
    assert!(matches!(p.get_field("y"), Some(UdValue::Number(n)) if n == 4.0));

    // Private fields should not be accessible
    assert!(p.get_field("_id").is_none());

    // Unknown fields should return None
    assert!(p.get_field("z").is_none());
}

#[test]
fn test_userdata_trait_set_field() {
    let mut p = Point {
        x: 1.0,
        y: 2.0,
        _id: 0,
    };

    // Set x to 10.0
    let result = p.set_field("x", UdValue::Number(10.0));
    assert!(matches!(result, Some(Ok(()))));
    assert_eq!(p.x, 10.0);

    // Set y via integer (coerced to float)
    let result = p.set_field("y", UdValue::Integer(20));
    assert!(matches!(result, Some(Ok(()))));
    assert_eq!(p.y, 20.0);

    // Setting with wrong type should error
    let result = p.set_field("x", UdValue::Str("bad".into()));
    assert!(matches!(result, Some(Err(_))));

    // Setting unknown field should return None
    assert!(p.set_field("z", UdValue::Number(0.0)).is_none());
}

#[test]
fn test_userdata_trait_field_names() {
    let p = Point {
        x: 0.0,
        y: 0.0,
        _id: 0,
    };
    let names = p.field_names();
    assert!(names.contains(&"x"));
    assert!(names.contains(&"y"));
    assert!(!names.contains(&"_id")); // private
}

#[test]
fn test_userdata_trait_display_metamethod() {
    let p = Point {
        x: 1.5,
        y: 2.5,
        _id: 0,
    };
    assert_eq!(p.lua_tostring(), Some("Point(1.5, 2.5)".to_string()));
}

#[test]
fn test_userdata_trait_eq_metamethod() {
    let p1 = Point {
        x: 1.0,
        y: 2.0,
        _id: 0,
    };
    let p2 = Point {
        x: 1.0,
        y: 2.0,
        _id: 99,
    }; // different _id but same x,y
    let p3 = Point {
        x: 3.0,
        y: 4.0,
        _id: 0,
    };

    // Since PartialEq is derived, it compares ALL fields including _id
    // p1 != p2 because _id differs
    assert_eq!(p1.lua_eq(&p2), Some(false));

    // p1 != p3
    assert_eq!(p1.lua_eq(&p3), Some(false));

    // p1 == p1
    assert_eq!(p1.lua_eq(&p1), Some(true));
}

#[test]
fn test_userdata_trait_ord_metamethod() {
    let p1 = Point {
        x: 1.0,
        y: 2.0,
        _id: 0,
    };
    let p2 = Point {
        x: 3.0,
        y: 4.0,
        _id: 0,
    };

    assert_eq!(p1.lua_lt(&p2), Some(true));
    assert_eq!(p2.lua_lt(&p1), Some(false));
    assert_eq!(p1.lua_le(&p2), Some(true));
    assert_eq!(p1.lua_le(&p1), Some(true));
}

#[test]
fn test_userdata_trait_readonly_field() {
    let mut cfg = Config {
        name: "test".to_string(),
        version: 1,
        secret: "sshh".to_string(),
        item_count: 5,
    };

    // Regular field can be set
    let result = cfg.set_field("name", UdValue::Str("new_name".into()));
    assert!(matches!(result, Some(Ok(()))));
    assert_eq!(cfg.name, "new_name");

    // Readonly field returns error
    let result = cfg.set_field("version", UdValue::Integer(2));
    assert!(matches!(result, Some(Err(_))));
    assert_eq!(cfg.version, 1); // unchanged

    // Skipped field is not accessible
    assert!(cfg.get_field("secret").is_none());
    assert!(
        cfg.set_field("secret", UdValue::Str("new".into()))
            .is_none()
    );
}

#[test]
fn test_userdata_trait_renamed_field() {
    let cfg = Config {
        name: "test".to_string(),
        version: 1,
        secret: "sshh".to_string(),
        item_count: 42,
    };

    // Access by Lua name, not Rust name
    assert!(matches!(cfg.get_field("count"), Some(UdValue::Integer(42))));
    assert!(cfg.get_field("item_count").is_none()); // Rust name not accessible
}

#[test]
fn test_userdata_trait_downcast() {
    let p = Point {
        x: 1.0,
        y: 2.0,
        _id: 0,
    };
    let trait_obj: &dyn UserDataTrait = &p;

    // Downcast via as_any
    let p_ref = trait_obj.as_any().downcast_ref::<Point>();
    assert!(p_ref.is_some());
    assert_eq!(p_ref.unwrap().x, 1.0);
}

#[test]
fn test_lua_userdata_wrapper() {
    let p = Point {
        x: 5.0,
        y: 10.0,
        _id: 0,
    };
    let mut ud = LuaUserdata::new(p);

    // Type name
    assert_eq!(ud.type_name(), "Point");

    // Trait-based field access
    assert!(matches!(ud.get_trait().get_field("x"), Some(UdValue::Number(n)) if n == 5.0));

    // Downcast access (backward compat)
    let p = ud.downcast_mut::<Point>().unwrap();
    p.x = 99.0;
    assert_eq!(ud.downcast_ref::<Point>().unwrap().x, 99.0);
}

#[test]
fn test_udvalue_conversions() {
    // From impls
    assert!(matches!(UdValue::from(42i64), UdValue::Integer(42)));
    assert!(matches!(UdValue::from(3.15f64), UdValue::Number(n) if n == 3.15));
    assert!(matches!(UdValue::from(true), UdValue::Boolean(true)));
    assert!(matches!(UdValue::from("hello"), UdValue::Str(s) if s == "hello"));

    // Option → UdValue
    let some: Option<i64> = Some(10);
    assert!(matches!(UdValue::from(some), UdValue::Integer(10)));
    let none: Option<i64> = None;
    assert!(matches!(UdValue::from(none), UdValue::Nil));

    // UdValue → Rust
    assert_eq!(UdValue::Integer(5).to_integer(), Some(5));
    assert_eq!(UdValue::Number(3.0).to_integer(), Some(3)); // exact float→int
    assert_eq!(UdValue::Number(3.5).to_integer(), None); // non-exact
    assert_eq!(UdValue::Integer(5).to_number(), Some(5.0));
    assert_eq!(UdValue::Str("hi".into()).to_str(), Some("hi"));
    assert!(!UdValue::Nil.to_bool());
    assert!(UdValue::Integer(0).to_bool()); // Lua truthiness
}

// ==================== Simple userdata trait (macro) ====================

struct SimpleHandle {
    id: u32,
}

crate::impl_simple_userdata!(SimpleHandle, "SimpleHandle");

#[test]
fn test_simple_userdata_macro() {
    let h = SimpleHandle { id: 42 };
    assert_eq!(h.type_name(), "SimpleHandle");

    // Simple userdata has no fields exposed
    assert!(h.get_field("id").is_none());

    // But downcast still works
    let ud = LuaUserdata::new(h);
    assert!(ud.downcast_ref::<SimpleHandle>().is_some());
    assert_eq!(ud.downcast_ref::<SimpleHandle>().unwrap().id, 42);
}

// ==================== VM Integration Tests ====================
// These tests verify that userdata is properly wired to the VM,
// so Lua scripts can access fields, set fields, and trigger metamethods.

use crate::lua_vm::{LuaVM, SafeOption};
use crate::stdlib;

/// Helper: create a VM with basic stdlib and register a Point userdata as global "p"
fn setup_point_vm() -> Box<LuaVM> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(stdlib::Stdlib::Basic).unwrap();
    vm.open_stdlib(stdlib::Stdlib::String).unwrap();

    let p = Point {
        x: 3.0,
        y: 4.0,
        _id: 0,
    };
    let ud = LuaUserdata::new(p);
    let state = vm.main_state();
    let ud_val = state.create_userdata(ud).unwrap();
    state.set_global("p", ud_val).unwrap();
    vm
}

#[test]
fn test_vm_get_field() {
    let mut vm = setup_point_vm();
    let results = vm.execute("return p.x, p.y").unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_number(), Some(3.0));
    assert_eq!(results[1].as_number(), Some(4.0));
}

#[test]
fn test_vm_set_field() {
    let mut vm = setup_point_vm();
    let results = vm
        .execute(
            r#"
        p.x = 10.0
        p.y = 20.0
        return p.x, p.y
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_number(), Some(10.0));
    assert_eq!(results[1].as_number(), Some(20.0));
}

#[test]
fn test_vm_tostring() {
    let mut vm = setup_point_vm();
    let results = vm.execute("return tostring(p)").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("Point(3, 4)"));
}

#[test]
fn test_vm_eq() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(stdlib::Stdlib::Basic).unwrap();

    let p1 = Point {
        x: 1.0,
        y: 2.0,
        _id: 0,
    };
    let p2 = Point {
        x: 1.0,
        y: 2.0,
        _id: 0,
    };
    let p3 = Point {
        x: 3.0,
        y: 4.0,
        _id: 0,
    };

    let state = vm.main_state();
    let v1 = state.create_userdata(LuaUserdata::new(p1)).unwrap();
    let v2 = state.create_userdata(LuaUserdata::new(p2)).unwrap();
    let v3 = state.create_userdata(LuaUserdata::new(p3)).unwrap();
    state.set_global("p1", v1).unwrap();
    state.set_global("p2", v2).unwrap();
    state.set_global("p3", v3).unwrap();

    let results = vm.execute("return p1 == p2, p1 == p3").unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_boolean(), Some(true));
    assert_eq!(results[1].as_boolean(), Some(false));
}

#[test]
fn test_vm_lt_le() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(stdlib::Stdlib::Basic).unwrap();

    let p1 = Point {
        x: 1.0,
        y: 2.0,
        _id: 0,
    };
    let p2 = Point {
        x: 3.0,
        y: 4.0,
        _id: 0,
    };

    let state = vm.main_state();
    let v1 = state.create_userdata(LuaUserdata::new(p1)).unwrap();
    let v2 = state.create_userdata(LuaUserdata::new(p2)).unwrap();
    state.set_global("p1", v1).unwrap();
    state.set_global("p2", v2).unwrap();

    let results = vm.execute("return p1 < p2, p1 <= p2, p2 < p1").unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_boolean(), Some(true));
    assert_eq!(results[1].as_boolean(), Some(true));
    assert_eq!(results[2].as_boolean(), Some(false));
}

#[test]
fn test_vm_concat() {
    let mut vm = setup_point_vm();
    let results = vm.execute(r#"return "pos=" .. tostring(p)"#).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("pos=Point(3, 4)"));
}

#[test]
fn test_vm_pass_userdata_to_function() {
    let mut vm = setup_point_vm();
    let results = vm
        .execute(
            r#"
        local function get_x(obj)
            return obj.x
        end
        return get_x(p)
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_number(), Some(3.0));
}

#[test]
fn test_vm_config_readonly() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(stdlib::Stdlib::Basic).unwrap();

    let cfg = Config {
        name: "test".to_string(),
        version: 42,
        secret: "hidden".to_string(),
        item_count: 10,
    };
    let state = vm.main_state();
    let ud_val = state.create_userdata(LuaUserdata::new(cfg)).unwrap();
    state.set_global("cfg", ud_val).unwrap();

    // Can read name and version
    let results = vm
        .execute("return cfg.name, cfg.version, cfg.count")
        .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_str(), Some("test"));
    assert_eq!(results[1].as_integer(), Some(42));
    assert_eq!(results[2].as_integer(), Some(10));

    // Can set name (writable)
    let results = vm.execute(r#"cfg.name = "new"; return cfg.name"#).unwrap();
    assert_eq!(results[0].as_str(), Some("new"));

    // Cannot set version (readonly) — should error
    let result = vm.execute("cfg.version = 99");
    assert!(result.is_err());
}

#[test]
fn test_vm_unknown_field_is_nil() {
    let mut vm = setup_point_vm();
    // Accessing a field that doesn't exist should fall through to metatable,
    // and since there's no metatable, should error (attempt to index userdata)
    // Actually, looking at the code: if get_field returns None AND there's no __index,
    // it produces an error. Let's verify the error case:
    let result = vm.execute("return p.nonexistent");
    // With no metatable set, this should error since no __index metamethod exists
    assert!(result.is_err());
}

#[test]
fn test_vm_type_of_userdata() {
    let mut vm = setup_point_vm();
    let results = vm.execute("return type(p)").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("userdata"));
}

// ==================== #[lua_methods] Tests ====================

#[test]
fn test_vm_method_distance() {
    let mut vm = setup_point_vm();
    let results = vm.execute("return p:distance()").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_float(), Some(5.0)); // 3-4-5 triangle
}

#[test]
fn test_vm_method_translate() {
    let mut vm = setup_point_vm();
    let results = vm
        .execute(
            r#"
        p:translate(10, 20)
        return p.x, p.y
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_number(), Some(13.0));
    assert_eq!(results[1].as_number(), Some(24.0));
}

#[test]
fn test_vm_method_scale() {
    let mut vm = setup_point_vm();
    let results = vm
        .execute(
            r#"
        p:scale(2)
        return p.x, p.y
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_number(), Some(6.0));
    assert_eq!(results[1].as_number(), Some(8.0));
}

#[test]
fn test_vm_method_optional_param() {
    let mut vm = setup_point_vm();

    // With parameter
    let results = vm.execute(r#"return p:greet("Alice")"#).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("Hello Alice from Point(3, 4)"));

    // Without parameter (nil/missing → None)
    let results = vm.execute("return p:greet()").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("Hello from Point(3, 4)"));
}

#[test]
fn test_vm_method_result_ok() {
    let mut vm = setup_point_vm();
    let results = vm.execute("return p:checked_div(2)").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_number(), Some(1.5)); // 3.0 / 2
}

#[test]
fn test_vm_method_result_err() {
    let mut vm = setup_point_vm();
    let result = vm.execute("return p:checked_div(0)");
    assert!(result.is_err()); // Should raise Lua error
}

#[test]
fn test_vm_method_as_field_access() {
    let mut vm = setup_point_vm();
    // Methods are accessed as fields that return CFunction values
    let results = vm.execute("return type(p.distance)").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("function"));
}

// ==================== register_type / Constructor Tests ====================

/// Helper: create a VM with Point registered as a class table
fn setup_point_class_vm() -> Box<LuaVM> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(stdlib::Stdlib::Basic).unwrap();
    vm.open_stdlib(stdlib::Stdlib::String).unwrap();

    let state = vm.main_state();
    state
        .register_type("Point", Point::__lua_static_methods())
        .unwrap();
    vm
}

#[test]
fn test_register_type_creates_global_table() {
    let mut vm = setup_point_class_vm();
    let results = vm.execute("return type(Point)").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("table"));
}

#[test]
fn test_register_type_new_is_function() {
    let mut vm = setup_point_class_vm();
    let results = vm.execute("return type(Point.new)").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("function"));
}

#[test]
fn test_register_type_constructor() {
    let mut vm = setup_point_class_vm();
    let results = vm
        .execute(
            r#"
        local p = Point.new(3, 4)
        return p.x, p.y
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_number(), Some(3.0));
    assert_eq!(results[1].as_number(), Some(4.0));
}

#[test]
fn test_register_type_constructor_with_methods() {
    let mut vm = setup_point_class_vm();
    let results = vm
        .execute(
            r#"
        local p = Point.new(3, 4)
        return p:distance()
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_float(), Some(5.0)); // 3-4-5 triangle
}

#[test]
fn test_register_type_constructor_with_mutation() {
    let mut vm = setup_point_class_vm();
    let results = vm
        .execute(
            r#"
        local p = Point.new(1, 2)
        p:translate(10, 20)
        return p.x, p.y
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_number(), Some(11.0));
    assert_eq!(results[1].as_number(), Some(22.0));
}

#[test]
fn test_register_type_constructor_tostring() {
    let mut vm = setup_point_class_vm();
    let results = vm
        .execute(
            r#"
        local p = Point.new(5, 10)
        return tostring(p)
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_str(), Some("Point(5, 10)"));
}

#[test]
fn test_register_type_multiple_instances() {
    let mut vm = setup_point_class_vm();
    let results = vm
        .execute(
            r#"
        local a = Point.new(1, 0)
        local b = Point.new(0, 1)
        return a.x, a.y, b.x, b.y
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 4);
    assert_eq!(results[0].as_number(), Some(1.0));
    assert_eq!(results[1].as_number(), Some(0.0));
    assert_eq!(results[2].as_number(), Some(0.0));
    assert_eq!(results[3].as_number(), Some(1.0));
}

#[test]
fn test_register_type_equality() {
    let mut vm = setup_point_class_vm();
    let results = vm
        .execute(
            r#"
        local a = Point.new(3, 4)
        local b = Point.new(3, 4)
        local c = Point.new(1, 2)
        return a == b, a == c
    "#,
        )
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_boolean(), Some(true));
    assert_eq!(results[1].as_boolean(), Some(false));
}

// ==================== Enum export tests ====================

#[allow(unused)]
#[derive(LuaUserData)]
enum Color {
    Red,
    Green,
    Blue,
}

#[allow(unused)]
#[derive(LuaUserData)]
enum HttpStatus {
    Ok = 200,
    NotFound = 404,
    ServerError = 500,
}

#[allow(unused)]
#[derive(LuaUserData)]
enum MixedDisc {
    A,      // 0
    B = 10, // 10
    C,      // 11
    D = 20, // 20
    E,      // 21
}

#[test]
fn test_enum_basic() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_enum::<Color>("Color").unwrap();

    let results = vm
        .execute("return Color.Red, Color.Green, Color.Blue")
        .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_integer(), Some(0));
    assert_eq!(results[1].as_integer(), Some(1));
    assert_eq!(results[2].as_integer(), Some(2));
}

#[test]
fn test_enum_explicit_discriminants() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_enum::<HttpStatus>("HttpStatus").unwrap();

    let results = vm
        .execute("return HttpStatus.Ok, HttpStatus.NotFound, HttpStatus.ServerError")
        .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_integer(), Some(200));
    assert_eq!(results[1].as_integer(), Some(404));
    assert_eq!(results[2].as_integer(), Some(500));
}

#[test]
fn test_enum_mixed_discriminants() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_enum::<MixedDisc>("MD").unwrap();

    let results = vm.execute("return MD.A, MD.B, MD.C, MD.D, MD.E").unwrap();
    assert_eq!(results.len(), 5);
    assert_eq!(results[0].as_integer(), Some(0));
    assert_eq!(results[1].as_integer(), Some(10));
    assert_eq!(results[2].as_integer(), Some(11));
    assert_eq!(results[3].as_integer(), Some(20));
    assert_eq!(results[4].as_integer(), Some(21));
}

#[test]
fn test_enum_in_lua_comparison() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_enum::<HttpStatus>("Status").unwrap();

    let results = vm
        .execute(
            r#"
        local code = 404
        if code == Status.NotFound then
            return "not found"
        elseif code == Status.Ok then
            return "ok"
        end
        return "unknown"
    "#,
        )
        .unwrap();
    assert_eq!(results[0].as_str(), Some("not found"));
}

#[test]
fn test_enum_iteration_in_lua() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_enum::<Color>("Color").unwrap();

    let results = vm
        .execute(
            r#"
        local count = 0
        for k, v in pairs(Color) do
            count = count + 1
        end
        return count
    "#,
        )
        .unwrap();
    assert_eq!(results[0].as_integer(), Some(3));
}

// ==================== Arithmetic operator tests ====================

/// Test type for arithmetic operators
#[derive(LuaUserData, Clone, PartialEq)]
#[lua_impl(Display, PartialEq, Add, Sub, Mul, Neg)]
struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl fmt::Display for Vec2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vec2({}, {})", self.x, self.y)
    }
}

impl std::ops::Add for Vec2 {
    type Output = Vec2;
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2 {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2 {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl std::ops::Mul for Vec2 {
    type Output = Vec2;
    fn mul(self, rhs: Vec2) -> Vec2 {
        Vec2 {
            x: self.x * rhs.x,
            y: self.y * rhs.y,
        }
    }
}

impl std::ops::Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2 {
            x: -self.x,
            y: -self.y,
        }
    }
}

#[lua_methods]
impl Vec2 {
    pub fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }
}

#[test]
fn test_userdata_add() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_type_of::<Vec2>("Vec2").unwrap();

    let results = vm
        .execute(
            r#"
        local a = Vec2.new(1, 2)
        local b = Vec2.new(3, 4)
        local c = a + b
        return c.x, c.y
    "#,
        )
        .unwrap();
    assert_eq!(results[0].as_number(), Some(4.0));
    assert_eq!(results[1].as_number(), Some(6.0));
}

#[test]
fn test_userdata_sub() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_type_of::<Vec2>("Vec2").unwrap();

    let results = vm
        .execute(
            r#"
        local a = Vec2.new(5, 7)
        local b = Vec2.new(2, 3)
        local c = a - b
        return c.x, c.y
    "#,
        )
        .unwrap();
    assert_eq!(results[0].as_number(), Some(3.0));
    assert_eq!(results[1].as_number(), Some(4.0));
}

#[test]
fn test_userdata_mul() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_type_of::<Vec2>("Vec2").unwrap();

    let results = vm
        .execute(
            r#"
        local a = Vec2.new(2, 3)
        local b = Vec2.new(4, 5)
        local c = a * b
        return c.x, c.y
    "#,
        )
        .unwrap();
    assert_eq!(results[0].as_number(), Some(8.0));
    assert_eq!(results[1].as_number(), Some(15.0));
}

#[test]
fn test_userdata_neg() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_type_of::<Vec2>("Vec2").unwrap();

    let results = vm
        .execute(
            r#"
        local a = Vec2.new(3, -4)
        local b = -a
        return b.x, b.y
    "#,
        )
        .unwrap();
    assert_eq!(results[0].as_number(), Some(-3.0));
    assert_eq!(results[1].as_number(), Some(4.0));
}

#[test]
fn test_userdata_chained_arithmetic() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_type_of::<Vec2>("Vec2").unwrap();

    let results = vm
        .execute(
            r#"
        local a = Vec2.new(1, 2)
        local b = Vec2.new(3, 4)
        local c = Vec2.new(10, 10)
        local d = (a + b) - c
        return d.x, d.y
    "#,
        )
        .unwrap();
    assert_eq!(results[0].as_number(), Some(-6.0));
    assert_eq!(results[1].as_number(), Some(-4.0));
}

#[test]
fn test_userdata_arithmetic_preserves_type() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();
    vm.register_type_of::<Vec2>("Vec2").unwrap();

    let results = vm
        .execute(
            r#"
        local a = Vec2.new(1, 2)
        local b = Vec2.new(3, 4)
        local c = a + b
        -- Result should be a full userdata with field access and tostring
        return tostring(c), c.x, c.y
    "#,
        )
        .unwrap();
    assert_eq!(results[0].as_str(), Some("Vec2(4, 6)"));
    assert_eq!(results[1].as_number(), Some(4.0));
    assert_eq!(results[2].as_number(), Some(6.0));
}
