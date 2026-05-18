use crate::*;

#[test]
fn test_miri_gc_fixed_objects_smoke() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
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
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(crate::stdlib::Stdlib::All).unwrap();

    let result = vm.execute(
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
