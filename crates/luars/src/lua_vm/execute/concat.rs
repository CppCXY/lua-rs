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
        lua_limits::{CONCAT_STACK_BUF_SIZE, LUAI_MAXSHORTLEN},
    },
};

/// Stack buffer size for small concatenations (covers most Lua concat ops)
const STACK_BUF_SIZE: usize = CONCAT_STACK_BUF_SIZE;

/// Short string limit matching StringInterner::SHORT_STRING_LIMIT.
/// Strings ≤ this length are interned (hash table dedup).
const SHORT_STR_LIMIT: usize = LUAI_MAXSHORTLEN;

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
    let start = *base + a;

    // === Ultra-fast path: all values are already strings ===
    // Single scan to verify all-string + compute total length, then copy
    // into a small 40-byte buffer for short results (matching C Lua's
    // LUAI_MAXSHORTLEN). Avoids the 256-byte zero-init and multi-branch
    // type checking of the general path.
    {
        let stack = lua_state.stack();
        let mut total_len = 0usize;
        let mut all_strings = true;

        for i in 0..n {
            if let Some(s) = unsafe { stack.get_unchecked(start + i) }.as_str() {
                total_len += s.len();
            } else {
                all_strings = false;
                break;
            }
        }

        if all_strings {
            let result = if total_len <= SHORT_STR_LIMIT {
                // Short result: must intern for correct string equality semantics.
                // C Lua also interns short concat results (luaS_newlstr).
                let mut buf = [0u8; SHORT_STR_LIMIT];
                let mut pos = 0;
                let stack = lua_state.stack();
                for i in 0..n {
                    let s = unsafe { stack.get_unchecked(start + i).as_str().unwrap_unchecked() };
                    let bytes = s.as_bytes();
                    buf[pos..pos + bytes.len()].copy_from_slice(bytes);
                    pos += bytes.len();
                }
                let result_str = unsafe { std::str::from_utf8_unchecked(&buf[..pos]) };
                lua_state.create_string(result_str)?
            } else {
                // Long result: skip interning (like C Lua's luaS_createlngstrobj).
                // Long strings use content comparison, so equality still works.
                let mut result_buf = String::with_capacity(total_len);
                let stack = lua_state.stack();
                for i in 0..n {
                    let s = unsafe { stack.get_unchecked(start + i).as_str().unwrap_unchecked() };
                    result_buf.push_str(s);
                }
                lua_state.create_string_owned(result_buf)?
            };
            lua_state.stack_mut()[start] = result;
            lua_state.set_frame_pc(frame_idx, pc as u32);
            // Match C Lua: set top to cover concat operands before checkGC,
            // then restore to frame_top. This prevents the GC atomic phase
            // from clearing temp registers below the concat range.
            let concat_top = start + n;
            if concat_top > lua_state.get_top() {
                lua_state.set_top_raw(concat_top);
            }
            lua_state.check_gc()?;
            let frame_top = lua_state.get_call_info(frame_idx).top;
            lua_state.set_top_raw(frame_top);
            return Ok(());
        }
    }

    // General path: handles numbers, binary, mixed types (no metamethods)
    if let Ok(result) = concat_strings(lua_state, *base, a, n) {
        lua_state.stack_mut()[start] = result;
        lua_state.set_frame_pc(frame_idx, pc as u32);
        // Match C Lua: set top to cover concat operands before checkGC
        let concat_top = start + n;
        if concat_top > lua_state.get_top() {
            lua_state.set_top_raw(concat_top);
        }
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
                // Like C Lua's luaG_concaterror: if p1 is string/number, blame p2
                lua_state.set_frame_pc(frame_idx, pc as u32);
                let blame_val = if v_prev.is_string() || v_prev.is_number() || v_prev.is_integer() {
                    v_top
                } else {
                    v_prev
                };
                return Err(crate::stdlib::debug::typeerror(
                    lua_state,
                    &blame_val,
                    "concatenate",
                ));
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
    if value.is_string()
        || value.is_binary()
        || value.as_integer().is_some()
        || value.as_float().is_some()
    {
        return true;
    }
    // Userdata with lua_tostring can be concat-converted
    if value.ttisfulluserdata()
        && let Some(ud) = value.as_userdata_mut()
    {
        return ud.get_trait().lua_tostring().is_some();
    }
    false
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
    if value.is_integer() {
        let i = value.as_integer_strict().unwrap();
        let mut itoa_buf = itoa::Buffer::new();
        buf.extend_from_slice(itoa_buf.format(i).as_bytes());
        Some(false)
    } else if value.is_float() {
        let f = value.as_number().unwrap();
        use crate::stdlib::basic::lua_float_to_string;
        let s = lua_float_to_string(f);
        buf.extend_from_slice(s.as_bytes());
        Some(false)
    } else if value.ttisfulluserdata() {
        // Userdata with lua_tostring can be used in concatenation
        if let Some(ud) = value.as_userdata_mut()
            && let Some(s) = ud.get_trait().lua_tostring()
        {
            buf.extend_from_slice(s.as_bytes());
            return Some(false);
        }
        None
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
                lua_state.create_string_owned(s)
            };
        } else {
            return Err(crate::stdlib::debug::typeerror(
                lua_state,
                &val,
                "concatenate",
            ));
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
            // Longer strings: single heap allocation with exact capacity, skip interning
            let mut result = String::with_capacity(total_len);
            result.push_str(s1);
            result.push_str(s2);
            return lua_state.create_string_owned(result);
        }

        // Fast path: string + integer (very common: "key" .. i)
        if let Some(s1) = v1.as_str()
            && let Some(i2) = v2.as_integer_strict()
        {
            let mut buf = [0u8; STACK_BUF_SIZE];
            let s1_bytes = s1.as_bytes();
            let mut itoa_buf = itoa::Buffer::new();
            let num_str = itoa_buf.format(i2);
            let total_len = s1_bytes.len() + num_str.len();
            if total_len <= STACK_BUF_SIZE {
                buf[..s1_bytes.len()].copy_from_slice(s1_bytes);
                buf[s1_bytes.len()..total_len].copy_from_slice(num_str.as_bytes());
                let s = unsafe { std::str::from_utf8_unchecked(&buf[..total_len]) };
                return lua_state.create_string(s);
            }
            let mut result = String::with_capacity(total_len);
            result.push_str(s1);
            result.push_str(num_str);
            return lua_state.create_string_owned(result);
        }
        // Fast path: integer + string
        if let Some(i1) = v1.as_integer_strict()
            && let Some(s2) = v2.as_str()
        {
            let mut buf = [0u8; STACK_BUF_SIZE];
            let mut itoa_buf = itoa::Buffer::new();
            let num_str = itoa_buf.format(i1);
            let s2_bytes = s2.as_bytes();
            let total_len = num_str.len() + s2_bytes.len();
            if total_len <= STACK_BUF_SIZE {
                buf[..num_str.len()].copy_from_slice(num_str.as_bytes());
                buf[num_str.len()..total_len].copy_from_slice(s2_bytes);
                let s = unsafe { std::str::from_utf8_unchecked(&buf[..total_len]) };
                return lua_state.create_string(s);
            }
            let mut result = String::with_capacity(total_len);
            result.push_str(num_str);
            result.push_str(s2);
            return lua_state.create_string_owned(result);
        }
    }

    // ===== General path: N values (two-pass: estimate then allocate) =====
    let start = base + a;

    // Phase 1: scan all values to estimate total length and detect types.
    // This avoids the old "try stack buffer, restart if overflow" pattern.
    let mut total_len: usize = 0;
    let mut has_binary = false;
    let mut all_strings = true; // true if all values are already strings (no binary, no numbers)

    for i in 0..n {
        let value = lua_state.stack[start + i];
        if let Some(s) = value.as_str() {
            total_len += s.len();
        } else if let Some(b) = value.as_binary() {
            total_len += b.len();
            has_binary = true;
            all_strings = false;
        } else if value.is_integer() {
            total_len += 20; // i64 max display width
            all_strings = false;
        } else if value.is_float() {
            total_len += 24; // float max display width (%.17g + sign + exponent)
            all_strings = false;
        } else if value.ttisfulluserdata() {
            if let Some(ud) = value.as_userdata_mut()
                && let Some(s) = ud.get_trait().lua_tostring()
            {
                total_len += s.len();
                all_strings = false;
                continue;
            }
            return Err(crate::stdlib::debug::typeerror(
                lua_state,
                &value,
                "concatenate",
            ));
        } else {
            return Err(crate::stdlib::debug::typeerror(
                lua_state,
                &value,
                "concatenate",
            ));
        }
    }

    // Phase 2: build result with optimal allocation strategy
    if total_len <= STACK_BUF_SIZE {
        // Small result: stack buffer (no heap allocation)
        let mut buf = [0u8; STACK_BUF_SIZE];
        let mut pos = 0;

        for i in 0..n {
            let value = lua_state.stack[start + i];
            if let Some(s) = value.as_str() {
                let bytes = s.as_bytes();
                buf[pos..pos + bytes.len()].copy_from_slice(bytes);
                pos += bytes.len();
            } else if let Some(b) = value.as_binary() {
                buf[pos..pos + b.len()].copy_from_slice(b);
                pos += b.len();
            } else if value.is_integer() {
                let ival = unsafe { value.as_integer_strict().unwrap_unchecked() };
                let mut itoa_buf = itoa::Buffer::new();
                let num_str = itoa_buf.format(ival);
                buf[pos..pos + num_str.len()].copy_from_slice(num_str.as_bytes());
                pos += num_str.len();
            } else if value.is_float() {
                let f = unsafe { value.as_number().unwrap_unchecked() };
                use crate::stdlib::basic::lua_float_to_string;
                let s = lua_float_to_string(f);
                buf[pos..pos + s.len()].copy_from_slice(s.as_bytes());
                pos += s.len();
            } else if value.ttisfulluserdata()
                && let Some(ud) = value.as_userdata_mut()
                && let Some(s) = ud.get_trait().lua_tostring()
            {
                buf[pos..pos + s.len()].copy_from_slice(s.as_bytes());
                pos += s.len();
            }
        }

        if has_binary {
            match std::str::from_utf8(&buf[..pos]) {
                Ok(s) => lua_state.create_string(s),
                Err(_) => lua_state.create_binary(buf[..pos].to_vec()),
            }
        } else {
            let s = unsafe { std::str::from_utf8_unchecked(&buf[..pos]) };
            lua_state.create_string(s)
        }
    } else if has_binary {
        // Large result with binary data: Vec<u8> buffer → try UTF-8
        let mut buf: Vec<u8> = Vec::with_capacity(total_len);
        for i in 0..n {
            let value = lua_state.stack[start + i];
            value_to_bytes_write(&value, &mut buf);
        }
        match String::from_utf8(buf) {
            Ok(s) => lua_state.create_string_owned(s),
            Err(e) => lua_state.create_binary(e.into_bytes()),
        }
    } else if all_strings {
        // Large result, all strings: single String allocation, zero-copy writes
        // total_len is exact (no number estimates), capacity is perfect
        let mut result = String::with_capacity(total_len);
        for i in 0..n {
            let value = lua_state.stack[start + i];
            result.push_str(unsafe { value.as_str().unwrap_unchecked() });
        }
        lua_state.create_string_owned(result)
    } else {
        // Large result, strings + numbers (no binary): String with estimated capacity
        let mut result = String::with_capacity(total_len);
        for i in 0..n {
            let value = lua_state.stack[start + i];
            if let Some(s) = value.as_str() {
                result.push_str(s);
            } else if value.is_integer() {
                let mut itoa_buf = itoa::Buffer::new();
                let ival = unsafe { value.as_integer_strict().unwrap_unchecked() };
                result.push_str(itoa_buf.format(ival));
            } else if value.is_float() {
                use crate::stdlib::basic::lua_float_to_string;
                let f = unsafe { value.as_number().unwrap_unchecked() };
                result.push_str(&lua_float_to_string(f));
            } else if value.ttisfulluserdata()
                && let Some(ud) = value.as_userdata_mut()
                && let Some(s) = ud.get_trait().lua_tostring()
            {
                result.push_str(&s);
            }
        }
        lua_state.create_string_owned(result)
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
        let result = vm.execute(
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
        let result = vm.execute(
            r#"
            local s = "x" .. 42 .. "y"
            assert(s == "x42y")
        "#,
        );
        assert!(result.is_ok());
    }
}
