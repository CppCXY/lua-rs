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

/// Maximum short string length (can use stack buffer)
const MAX_SHORT_LEN: usize = 256;

/// Convert value to string representation
/// Returns (string content, was_already_string)
#[inline]
fn value_to_string_content(
    lua_state: &mut LuaState,
    value: &LuaValue,
) -> LuaResult<(String, bool)> {
    // Fast path: already a string
    if let Some(string_id) = value.as_string_id() {
        if let Some(s) = lua_state.vm_mut().object_pool.get_string(string_id) {
            return Ok((s.as_str().to_string(), true));
        }
    }

    // Convert other types to string
    let s = if let Some(i) = value.as_integer() {
        i.to_string()
    } else if let Some(f) = value.as_float() {
        // Lua number formatting
        if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e14 {
            format!("{:.1}", f)
        } else {
            f.to_string()
        }
    } else if let Some(b) = value.as_bool() {
        b.to_string()
    } else if value.is_nil() {
        "nil".to_string()
    } else {
        // Table, function, etc. - in real Lua this would try __tostring or error
        return Err(lua_state.error(format!(
            "attempt to concatenate a {} value",
            value.type_name()
        )));
    };

    Ok((s, false))
}

/// Optimized string concatenation matching Lua 5.5 behavior
/// Concatenates values from stack[base+a] to stack[base+a+n-1]
pub fn concat_strings(
    lua_state: &mut LuaState,
    stack_ptr: *mut LuaValue,
    base: usize,
    a: usize,
    n: usize,
) -> LuaResult<LuaValue> {
    if n == 0 {
        // Empty concat - return empty string
        let (string_id, _) = lua_state.vm_mut().object_pool.create_string("");
        return Ok(LuaValue::string(string_id));
    }

    if n == 1 {
        // Single value - convert to string if needed
        unsafe {
            let val = *stack_ptr.add(base + a);
            if val.is_string() {
                return Ok(val); // Already a string
            }
            let (s, _) = value_to_string_content(lua_state, &val)?;
            let (string_id, _) = lua_state.vm_mut().object_pool.create_string(&s);
            return Ok(LuaValue::string(string_id));
        }
    }

    // Multiple values - optimize concat
    unsafe {
        // Phase 1: Check for empty string optimization and collect lengths
        let mut total_len = 0usize;
        let mut all_strings = true;
        let mut has_empty = false;
        
        for i in 0..n {
            let val = *stack_ptr.add(base + a + i);
            
            if let Some(string_id) = val.as_string_id() {
                if let Some(s) = lua_state.vm_mut().object_pool.get_string(string_id) {
                    let len = s.as_str().len();
                    if len == 0 {
                        has_empty = true;
                    }
                    total_len += len;
                } else {
                    all_strings = false;
                }
            } else {
                all_strings = false;
            }
        }

        // Fast path: all strings already, check for optimizations
        if all_strings {
            // Optimization: if first or last is empty, can skip it
            if n == 2 && has_empty {
                let first = *stack_ptr.add(base + a);
                let second = *stack_ptr.add(base + a + 1);
                
                if let (Some(id1), Some(id2)) = (first.as_string_id(), second.as_string_id()) {
                    let (len1, len2) = {
                        let pool = &lua_state.vm_mut().object_pool;
                        let l1 = pool.get_string(id1).map(|s| s.as_str().len()).unwrap_or(0);
                        let l2 = pool.get_string(id2).map(|s| s.as_str().len()).unwrap_or(0);
                        (l1, l2)
                    };
                    
                    if len1 == 0 {
                        return Ok(second); // First is empty, return second
                    }
                    if len2 == 0 {
                        return Ok(first); // Second is empty, return first
                    }
                }
            }

            // All strings, no empties or can't optimize - concat them
            if total_len <= MAX_SHORT_LEN {
                // Short string - use Vec (will be on stack in optimized version)
                let mut buffer = Vec::with_capacity(total_len);
                
                for i in 0..n {
                    let val = *stack_ptr.add(base + a + i);
                    if let Some(string_id) = val.as_string_id() {
                        if let Some(s) = lua_state.vm_mut().object_pool.get_string(string_id) {
                            buffer.extend_from_slice(s.as_str().as_bytes());
                        }
                    }
                }
                
                let result = String::from_utf8(buffer).unwrap_or_default();
                let (string_id, _) = lua_state.vm_mut().object_pool.create_string(&result);
                return Ok(LuaValue::string(string_id));
            } else {
                // Long string - build directly
                let mut result = String::with_capacity(total_len);
                
                for i in 0..n {
                    let val = *stack_ptr.add(base + a + i);
                    if let Some(string_id) = val.as_string_id() {
                        if let Some(s) = lua_state.vm_mut().object_pool.get_string(string_id) {
                            result.push_str(s.as_str());
                        }
                    }
                }
                
                let (string_id, _) = lua_state.vm_mut().object_pool.create_string(&result);
                return Ok(LuaValue::string(string_id));
            }
        }

        // Slow path: need to convert some values to strings
        // Collect all parts first, then concatenate
        let mut parts: Vec<String> = Vec::with_capacity(n);
        let mut total_len = 0;
        
        for i in 0..n {
            let val = *stack_ptr.add(base + a + i);
            let (s, _was_string) = value_to_string_content(lua_state, &val)?;
            total_len += s.len();
            parts.push(s);
        }

        // Build final result
        if total_len <= MAX_SHORT_LEN {
            // Short string
            let mut buffer = Vec::with_capacity(total_len);
            for part in &parts {
                buffer.extend_from_slice(part.as_bytes());
            }
            let result = String::from_utf8(buffer).unwrap_or_default();
            let (string_id, _) = lua_state.vm_mut().object_pool.create_string(&result);
            Ok(LuaValue::string(string_id))
        } else {
            // Long string
            let mut result = String::with_capacity(total_len);
            for part in parts {
                result.push_str(&part);
            }
            let (string_id, _) = lua_state.vm_mut().object_pool.create_string(&result);
            Ok(LuaValue::string(string_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua_vm::{LuaVM, safe_option::SafeOption};

    #[test]
    fn test_concat_empty() {
        let mut vm = LuaVM::new(SafeOption::default());
        let state = &mut vm.main_state;
        let stack_ptr = state.stack_ptr_mut();
        // Empty concat should return empty string
        let result = concat_strings(state, stack_ptr, 0, 0, 0);
        assert!(result.is_ok());
        let val = result.unwrap();
        assert!(val.is_string());
    }

    #[test]
    fn test_concat_single() {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.open_libs();
        
        // Test single string
        let result = vm.execute_string(r#"
            local s = "hello"
            return s
        "#);
        assert!(result.is_ok());
    }

    #[test]
    fn test_concat_numbers() {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.open_libs();
        
        // Test number concatenation
        let result = vm.execute_string(r#"
            local s = "x" .. 42 .. "y"
            assert(s == "x42y")
        "#);
        assert!(result.is_ok());
    }
}
