use crate::*;

#[test]
fn test_miri_gc_fixed_objects_smoke() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local MAX_NESTING <const> = 32

        local function render()
            local result = {}

            local function dump()
                if MAX_NESTING > 0 then
                    result[#result + 1] = "x,\n"
                end
            end

            dump()

            local last = result[#result]
            result[#result] = string.sub(last, 1, #last - 2) .. "\n"

            return table.concat(result)
        end

        assert(render() == "x\n")
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_miri_debug_traceback_smoke() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local function outer()
            local trace = debug.traceback("", 2)
            assert(type(trace) == "string")
            assert(string.find(trace, "stack traceback:", 1, true) ~= nil)
        end

        outer()
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_miri_pow_basic_regression() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let results = vm.main_state().execute("return 2 ^ 3").unwrap();

    assert_eq!(results[0].as_integer(), Some(8), "value: {:?}", results[0]);
}

#[test]
fn test_miri_pow_precedence_regression() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let results = vm
        .main_state()
        .execute("return 2 ^ 3 ^ 2, (2 ^ 3) ^ 2")
        .unwrap();

    assert_eq!(
        results[0].as_integer(),
        Some(512),
        "value: {:?}",
        results[0]
    );
    assert_eq!(results[1].as_integer(), Some(64), "value: {:?}", results[1]);
}

#[test]
fn test_miri_pow_metamethod_regression() {
    let mut vm = GlobalState::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.main_state().execute(
        r#"
        local a = {val = 2}
        setmetatable(a, {__pow = function(x, y) return {val = x.val ^ y} end})
        local c = a ^ 4
        assert(c.val == 16)
        "#,
    );

    assert!(result.is_ok(), "Error: {:?}", result.err());
}
