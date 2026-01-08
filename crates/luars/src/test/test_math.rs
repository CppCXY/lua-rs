// Tests for math library functions
use crate::*;

#[test]
fn test_math_constants() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.pi > 3.14 and math.pi < 3.15)
        assert(math.huge > 0)
        assert(math.huge == math.huge * 2)
        assert(math.maxinteger > 0)
        assert(math.mininteger < 0)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_abs() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.abs(-5) == 5)
        assert(math.abs(5) == 5)
        assert(math.abs(-3.14) == 3.14)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_math_ceil_floor() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.ceil(3.2) == 4)
        assert(math.ceil(3.8) == 4)
        assert(math.floor(3.2) == 3)
        assert(math.floor(3.8) == 3)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_max_min() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.max(1, 2, 3) == 3)
        assert(math.min(1, 2, 3) == 1)
        assert(math.max(-5, -10) == -5)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_math_sqrt() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.sqrt(4) == 2)
        assert(math.sqrt(9) == 3)
        assert(math.sqrt(2) > 1.41 and math.sqrt(2) < 1.42)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_exp_log() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local e = math.exp(1)
        assert(e > 2.71 and e < 2.72)
        assert(math.log(e) > 0.99 and math.log(e) < 1.01)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_sin_cos_tan() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.sin(0) == 0)
        assert(math.cos(0) == 1)
        local sin90 = math.sin(math.pi / 2)
        assert(sin90 > 0.99 and sin90 < 1.01)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_math_deg_rad() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local rad = math.rad(180)
        assert(rad > 3.14 and rad < 3.15)
        assert(math.deg(math.pi) >= 179 and math.deg(math.pi) <= 181)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_random() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        math.randomseed(12345)
        local r1 = math.random()
        assert(r1 >= 0 and r1 < 1)
        
        local r2 = math.random(10)
        assert(r2 >= 1 and r2 <= 10)
        
        local r3 = math.random(5, 15)
        assert(r3 >= 5 and r3 <= 15)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_modf() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        local i, f = math.modf(3.14)
        assert(i == 3)
        assert(f > 0.13 and f < 0.15)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_fmod() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.fmod(10, 3) == 1)
        assert(math.fmod(10.5, 2) == 0.5)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_tointeger() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.tointeger(3.0) == 3)
        assert(math.tointeger(3.5) == nil)
        assert(math.tointeger("10") == 10)
    "#,
    );

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_math_type() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.type(3) == "integer")
        assert(math.type(3.14) == "float")
        assert(math.type("hello") == nil)
    "#,
    );

    assert!(result.is_ok());
}

#[test]
fn test_math_ult() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute_string(
        r#"
        assert(math.ult(2, 3) == true)
        assert(math.ult(3, 2) == false)
    "#,
    );

    assert!(result.is_ok());
}
