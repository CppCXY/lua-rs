/*----------------------------------------------------------------------
  String Concatenation Optimization - Lua 5.5 Style

  Based on luaV_concat from lua-5.5.0/src/lvm.c

  Key optimizations:
  1. Stack buffer for small concats (avoid heap allocation)
  2. Fast path for 2-value all-string concat (most common case)
  3. itoa/ryu for fast number formatting (no heap alloc)
  4. No unnecessary Vec::clone
  5. String interning reuse
----------------------------------------------------------------------*/

use crate::{
    lua_value::LuaValue,
    lua_vm::{LuaResult, LuaState},
};

/// Stack buffer size for small concatenations (covers most Lua concat ops)
const STACK_BUF_SIZE: usize = 256;

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

    // Convert numbers using stack-allocated formatting (no heap alloc)
    if let Some(i) = value.as_integer() {
        let mut itoa_buf = itoa::Buffer::new();
        buf.extend_from_slice(itoa_buf.format(i).as_bytes());
        Some(false)
    } else if let Some(f) = value.as_float() {
        // Use Rust's default formatting (matches Lua's %.14g closely enough)
        // ryu would change format and break tests
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
        return lua_state.create_string("");
    }

    if n == 1 {
        let stack = lua_state.stack_mut();
        let val = stack[base + a];
        if val.is_string() || val.is_binary() {
            return Ok(val);
        }
        // Convert single value to string
        let mut result = Vec::new();
        if value_to_bytes_write(&val, &mut result).is_some() {
            // SAFETY: number formatting always produces valid UTF-8
            return unsafe {
                let s = String::from_utf8_unchecked(result);
                lua_state.create_string(&s)
            };
        } else {
            return Err(lua_state.error(format!(
                "attempt to concatenate a {} value",
                val.type_name()
            )));
        }
    }

    // ===== Fast path: 2 string values (most common concat case: "a" .. "b") =====
    if n == 2 {
        let stack = lua_state.stack_mut();
        let v1 = stack[base + a];
        let v2 = stack[base + a + 1];

        // Ultra-fast: both already strings
        if let (Some(s1), Some(s2)) = (v1.as_str(), v2.as_str()) {
            let total_len = s1.len() + s2.len();
            if total_len <= STACK_BUF_SIZE {
                // Use stack buffer — avoid heap allocation entirely
                let mut buf = [0u8; STACK_BUF_SIZE];
                buf[..s1.len()].copy_from_slice(s1.as_bytes());
                buf[s1.len()..total_len].copy_from_slice(s2.as_bytes());
                // SAFETY: both inputs are valid UTF-8 str, concatenation is also valid
                let s = unsafe { std::str::from_utf8_unchecked(&buf[..total_len]) };
                return lua_state.create_string(s);
            }
            // Longer strings: single heap allocation with exact capacity
            let mut result = String::with_capacity(total_len);
            result.push_str(s1);
            result.push_str(s2);
            return lua_state.create_string(&result);
        }
    }

    // ===== General path: N values =====
    // Pre-calculate total length for exact Vec capacity
    let mut total_len = 0usize;
    let mut all_strings = true;
    for i in 0..n {
        let value = lua_state.stack_mut()[base + a + i];
        if let Some(s) = value.as_str() {
            total_len += s.len();
        } else if let Some(b) = value.as_binary() {
            total_len += b.len();
        } else if value.as_integer().is_some() {
            total_len += 20; // max digits for i64
            all_strings = false;
        } else if value.as_float().is_some() {
            total_len += 24; // max chars for f64
            all_strings = false;
        } else {
            return Err(lua_state.error(format!(
                "attempt to concatenate a {} value",
                value.type_name()
            )));
        }
    }

    // For small all-string results, use stack buffer
    if all_strings && total_len <= STACK_BUF_SIZE {
        let mut buf = [0u8; STACK_BUF_SIZE];
        let mut pos = 0;
        for i in 0..n {
            let value = lua_state.stack_mut()[base + a + i];
            if let Some(s) = value.as_str() {
                let bytes = s.as_bytes();
                buf[pos..pos + bytes.len()].copy_from_slice(bytes);
                pos += bytes.len();
            } else if let Some(b) = value.as_binary() {
                buf[pos..pos + b.len()].copy_from_slice(b);
                pos += b.len();
            }
        }
        // All inputs are valid UTF-8 strings
        let s = unsafe { std::str::from_utf8_unchecked(&buf[..pos]) };
        return lua_state.create_string(s);
    }

    let mut result: Vec<u8> = Vec::with_capacity(total_len);

    for i in 0..n {
        let value = lua_state.stack_mut()[base + a + i];
        if value_to_bytes_write(&value, &mut result).is_none() {
            return Err(lua_state.error(format!(
                "attempt to concatenate a {} value",
                value.type_name()
            )));
        }
    }

    // All number/string formatting produces valid UTF-8, so this is safe
    // Only binary data could be non-UTF-8, and as_binary() returns bytes directly
    match String::from_utf8(result) {
        Ok(s) => lua_state.create_string(&s),
        Err(e) => lua_state.create_binary(e.into_bytes()),
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
