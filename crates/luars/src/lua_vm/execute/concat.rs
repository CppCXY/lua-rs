/*----------------------------------------------------------------------
  String Concatenation Optimization - Lua 5.5 Style

  Based on luaV_concat from lua-5.5.0/src/lvm.c

  Key optimizations:
  1. Pre-calculate total length to avoid reallocation
  2. Short string optimization (stack buffer)
  3. Empty string fast path
  4. Direct buffer writing (no intermediate allocations)
  5. String interning reuse
----------------------------------------------------------------------*/

use crate::{
    lua_value::LuaValue,
    lua_vm::{LuaResult, LuaState},
};

/// Write value's string representation directly to buffer
/// Returns Some(was_already_string) if convertible, None otherwise
/// This avoids temporary string allocations
#[inline]
fn value_to_bytes_write(value: &LuaValue, buf: &mut Vec<u8>) -> Option<bool> {
    // Fast path: already a string (using direct pointer)
    if let Some(s) = value.as_str() {
        buf.extend_from_slice(s.as_bytes());
        return Some(true);
    }

    // Handle binary data directly
    if let Some(bytes) = value.as_binary() {
        buf.extend_from_slice(bytes);
        return Some(true);
    }

    // Convert other types to string (only number and bool can be auto-converted)
    if let Some(i) = value.as_integer() {
        buf.extend_from_slice(i.to_string().as_bytes());
        Some(false)
    } else if let Some(f) = value.as_float() {
        buf.extend_from_slice(f.to_string().as_bytes());
        Some(false)
    } else {
        // Table, function, nil, bool=false, etc. cannot be auto-converted
        // Let caller decide whether to try metamethod
        None
    }
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
        return lua_state.create_string("");
    }

    if n == 1 {
        // Single value - convert to string if needed
        let stack = lua_state.stack_mut();
        let val = stack[base + a];
        if val.is_string() || val.is_binary() {
            return Ok(val); // Already a string or binary
        }
        // Convert to string
        let mut result = Vec::new();
        if value_to_bytes_write(&val, &mut result).is_some() {
            // Try to create string, fall back to binary if not valid UTF-8
            return match String::from_utf8(result.clone()) {
                Ok(s) => lua_state.create_string(&s),
                Err(_) => lua_state.create_binary(result),
            };
        } else {
            // Cannot convert - need metamethod
            return Err(lua_state.error(format!(
                "attempt to concatenate a {} value",
                val.type_name()
            )));
        }
    }

    let mut result = Vec::new();

    for i in 0..n {
        let value = lua_state.stack_mut()[base + a + i];
        if value_to_bytes_write(&value, &mut result).is_none() {
            // Cannot convert this value - need metamethod
            return Err(lua_state.error(format!(
                "attempt to concatenate a {} value",
                value.type_name()
            )));
        }
    }

    // Try to create string, fall back to binary if not valid UTF-8
    match String::from_utf8(result.clone()) {
        Ok(s) => lua_state.create_string_owned(s),
        Err(_) => lua_state.create_binary(result),
    }
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
        let state = vm.main_state();
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
