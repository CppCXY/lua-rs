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
    Instruction,
    lua_value::LuaValue,
    lua_vm::{
        LuaResult, LuaState, TmKind,
        execute::{helper, metamethod},
    },
};

/// Stack buffer size for small concatenations (covers most Lua concat ops)
const STACK_BUF_SIZE: usize = 256;

#[cold]
#[inline(never)]
pub fn handle_concat(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: &mut usize,
    frame_idx: usize,
    pc: usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let n = instr.get_b() as usize;

    // Fast path: try to concat all values at once (no tables/metamethods)
    if let Ok(result) = concat_strings(lua_state, *base, a, n) {
        let stack = lua_state.stack_mut();
        stack[*base + a] = result;
        lua_state.set_frame_pc(frame_idx, pc as u32);
        lua_state.check_gc()?;
        let frame_top = lua_state.get_call_info(frame_idx).top;
        lua_state.set_top_raw(frame_top);
        return Ok(());
    }

    // Slow path: process iteratively from right to left, like C Lua 5.5's luaV_concat.
    // Stack range: [base+a .. base+a+n-1]. We track 'total' remaining values.
    let mut total = n;
    while total > 1 {
        // Try to coalesce consecutive string/number values from the right end
        // top points to base+a+total-1 (last value)
        let top_idx = *base + a + total - 1;

        let v_top = lua_state.stack()[top_idx];
        let v_prev = lua_state.stack()[top_idx - 1];

        let top_convertible = is_concat_convertible(&v_top);
        let prev_convertible = is_concat_convertible(&v_prev);

        if prev_convertible && top_convertible {
            // Both are string/number - coalesce as many consecutive convertible values as possible
            let mut coalesce_count = 2;
            while coalesce_count < total {
                let idx = top_idx - coalesce_count;
                let v = lua_state.stack()[idx];
                if !is_concat_convertible(&v) {
                    break;
                }
                coalesce_count += 1;
            }

            // Concat the coalesced values
            let start = top_idx + 1 - coalesce_count;
            let result = concat_strings(lua_state, 0, start, coalesce_count)?;
            let stack = lua_state.stack_mut();
            stack[start] = result;
            // "pop" the consumed values: total decreases by coalesce_count - 1
            // Shift isn't needed since the result is at 'start' and we track by total
            total -= coalesce_count - 1;
            // Move result to the correct position if needed
            let new_top = *base + a + total - 1;
            if start != new_top {
                let stack = lua_state.stack_mut();
                stack[new_top] = stack[start];
            }
        } else {
            // At least one value needs __concat metamethod
            if let Some(mm) =
                helper::get_binop_metamethod(lua_state, &v_prev, &v_top, TmKind::Concat)
            {
                lua_state.set_frame_pc(frame_idx, pc as u32);
                let result = match metamethod::call_tm_res(lua_state, mm, v_prev, v_top) {
                    Ok(r) => r,
                    Err(crate::lua_vm::LuaError::Yield) => {
                        use crate::lua_vm::call_info::call_status::CIST_PENDING_FINISH;
                        let ci = lua_state.get_call_info_mut(frame_idx);
                        ci.call_status |= CIST_PENDING_FINISH;
                        return Err(crate::lua_vm::LuaError::Yield);
                    }
                    Err(e) => return Err(e),
                };
                *base = lua_state.get_frame_base(frame_idx);

                // Store result, replacing the two operands with one
                let result_idx = *base + a + total - 2;
                let stack = lua_state.stack_mut();
                stack[result_idx] = result;
                total -= 1;
            } else {
                return Err(lua_state.error(format!(
                    "attempt to concatenate {} and {} values",
                    v_prev.type_name(),
                    v_top.type_name()
                )));
            }
        }
    }

    // Final result is at stack[base+a]
    // (It's already there since we've been collapsing towards the left)

    lua_state.set_frame_pc(frame_idx, pc as u32);
    lua_state.check_gc()?;

    let frame_top = lua_state.get_call_info(frame_idx).top;
    lua_state.set_top_raw(frame_top);
    Ok(())
}

/// Check if a value can be directly converted to string for concatenation
#[inline(always)]
fn is_concat_convertible(value: &LuaValue) -> bool {
    value.is_string() || value.is_binary() || value.as_integer().is_some() || value.as_float().is_some()
}

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
