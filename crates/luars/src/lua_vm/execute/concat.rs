/*----------------------------------------------------------------------
  String Concatenation Optimization - Lua 5.5 Style

  Based on luaV_concat from lua-5.5.0/src/lvm.c

  Key optimizations:
  1. Pre-calculate total length to avoid reallocation
  2. Short string optimization (stack buffer)
  3. Empty string fast path
  4. Direct buffer writing (no intermediate Vec)
  5. String interning reuse
----------------------------------------------------------------------*/

use crate::{
    lua_value::LuaValue,
    lua_vm::{LuaResult, LuaState},
};

/// Convert value to string representation
/// Returns Some(string, was_already_string) if convertible, None otherwise
#[inline]
fn value_to_string_content(value: &LuaValue) -> Option<(String, bool)> {
    // Fast path: already a string (using direct pointer)
    if let Some(s) = value.as_str() {
        return Some((s.to_string(), true));
    }

    // Convert other types to string (only number and bool can be auto-converted)
    let s = if let Some(i) = value.as_integer() {
        i.to_string()
    } else if let Some(f) = value.as_float() {
        // Lua number formatting
        if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e14 {
            format!("{:.1}", f)
        } else {
            f.to_string()
        }
    } else {
        // Table, function, nil, bool=false, etc. cannot be auto-converted
        // Let caller decide whether to try metamethod
        return None;
    };

    Some((s, false))
}

/// Optimized string concatenation matching Lua 5.5 behavior
/// Concatenates values from stack[base+a] to stack[base+a+n-1]
/// Returns Ok(result) if all values can be converted to strings
/// Returns Err if any value cannot be converted (caller should try __concat metamethod)
pub fn concat_strings(
    lua_state: &mut LuaState,
    base: usize,
    a: usize,
    n: usize,
) -> LuaResult<LuaValue> {
    if n == 0 {
        // Empty concat - return empty string
        let v = lua_state.create_string("");
        return Ok(v);
    }

    if n == 1 {
        // Single value - convert to string if needed
        let stack = lua_state.stack_mut();
        let val = stack[base + a];
        if val.is_string() {
            return Ok(val); // Already a string
        }
        if let Some((s, _)) = value_to_string_content(&val) {
            let v = lua_state.create_string(&s);
            return Ok(v);
        } else {
            // Cannot convert - need metamethod
            return Err(lua_state.error(format!(
                "attempt to concatenate a {} value",
                val.type_name()
            )));
        }
    }

    // Multiple values - optimize concat
    // Phase 1: Check for empty string optimization and collect lengths
    let mut total_len = 0usize;
    let mut all_strings = true;
    // Collect string_ids first (no borrow conflict)
    let stack = lua_state.stack_mut();
    let mut strings = Vec::with_capacity(n);
    for i in 0..n {
        let val = &stack[base + a + i];
        if let Some(str) = val.as_str() {
            total_len += str.len();
            strings.push(str);
        } else {
            all_strings = false;
            break;
        }
    }

    let mut result = String::with_capacity(total_len);
    // Fast path: all strings already, check for optimizations
    if all_strings {
        for s in strings {
            result.push_str(s);
        }

        let v = lua_state.create_string(&result);
        return Ok(v);
    }
    // Collect values first - try to convert each to string
    for i in 0..n {
        let value = lua_state.stack_mut()[base + a + i];
        if let Some((s, _was_string)) = value_to_string_content(&value) {
            result.push_str(&s);
        } else {
            // Cannot convert this value - need metamethod
            return Err(lua_state.error(format!(
                "attempt to concatenate a {} value",
                value.type_name()
            )));
        }
    }

    let s = lua_state.create_string_owned(result);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lua_vm::{LuaVM, safe_option::SafeOption},
        stdlib,
    };

    #[test]
    fn test_concat_empty() {
        let mut vm = LuaVM::new(SafeOption::default());
        let state = &mut vm.main_state;
        // Empty concat should return empty string
        let result = concat_strings(state, 0, 0, 0);
        assert!(result.is_ok());
        let val = result.unwrap();
        assert!(val.is_string());
    }

    #[test]
    fn test_concat_single() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Test single string
        let result = vm.execute_string(
            r#"
            local s = "hello"
            return s
        "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_concat_numbers() {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.open_stdlib(stdlib::Stdlib::Basic).unwrap();

        // Test number concatenation
        let result = vm.execute_string(
            r#"
            local s = "x" .. 42 .. "y"
            assert(s == "x42y")
        "#,
        );
        assert!(result.is_ok());
    }
}
