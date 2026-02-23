// String library
// Implements: byte, char, dump, find, format, gmatch, gsub, len, lower,
// match, pack, packsize, rep, reverse, sub, unpack, upper
mod pack;
mod pattern;
mod string_format;

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::lua_limits::MAX_STRING_SIZE;
use crate::lua_vm::{LuaResult, LuaState};

/// Mirrors luaL_checkinteger: convert a LuaValue to integer, producing
/// appropriate error messages like C Lua.
fn value_to_integer(v: &LuaValue) -> Result<i64, &'static str> {
    if let Some(i) = v.as_integer() {
        return Ok(i);
    }
    if let Some(f) = v.as_number() {
        if f == f.floor() && f.is_finite() && f >= (i64::MIN as f64) && f < (i64::MAX as f64) {
            return Ok(f as i64);
        }
        return Err("number has no integer representation");
    }
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if let Ok(i) = s.parse::<i64>() {
            return Ok(i);
        }
        if let Ok(f) = s.parse::<f64>() {
            if f == f.floor() && f.is_finite() && f >= (i64::MIN as f64) && f < (i64::MAX as f64) {
                return Ok(f as i64);
            }
            return Err("number has no integer representation");
        }
    }
    Err("number expected")
}

pub fn create_string_lib() -> LibraryModule {
    crate::lib_module!("string", {
        "byte" => string_byte,
        "char" => string_char,
        "dump" => string_dump,
        "find" => string_find,
        "format" => string_format::string_format,
        "gmatch" => string_gmatch,
        "gsub" => string_gsub,
        "len" => string_len,
        "lower" => string_lower,
        "match" => string_match,
        "pack" => pack::string_pack,
        "packsize" => pack::string_packsize,
        "rep" => string_rep,
        "reverse" => string_reverse,
        "sub" => string_sub,
        "unpack" => pack::string_unpack,
        "upper" => string_upper,
    })
}

/// string.byte(s [, i [, j]]) - Return byte values
/// Supports both string and binary types
fn string_byte(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'string.byte' (string expected)".to_string()))?;

    let i = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(1);
    let j = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(i);

    // Get bytes - works for both string and binary
    let bytes = if let Some(s) = s_value.as_str() {
        s.as_bytes()
    } else if let Some(b) = s_value.as_binary() {
        b
    } else {
        return Err(l.error("bad argument #1 to 'string.byte' (string expected)".to_string()));
    };

    let len = bytes.len() as i64;

    // Convert negative indices
    let start = if i < 0 { len + i + 1 } else { i };
    let end = if j < 0 { len + j + 1 } else { j };

    // Clamp start and end to valid range [1, len]
    let start = start.max(1);
    let end = end.min(len);

    // If start > end after clamping, return empty
    if start > end || start > len {
        return Ok(0);
    }

    // FAST PATH: Single byte return (most common case)
    if start == end && start >= 1 && start <= len {
        let byte = bytes[(start - 1) as usize];
        l.push_value(LuaValue::integer(byte as i64))?;
        return Ok(1);
    }

    // FAST PATH: Two byte return
    if end == start + 1 && start >= 1 && end <= len {
        let b1 = bytes[(start - 1) as usize] as i64;
        let b2 = bytes[(end - 1) as usize] as i64;
        l.push_value(LuaValue::integer(b1))?;
        l.push_value(LuaValue::integer(b2))?;
        return Ok(2);
    }

    // Slow path: multiple returns
    let mut count = 0;
    for idx in start..=end {
        if idx >= 1 && idx <= len {
            let byte = bytes[(idx - 1) as usize];
            l.push_value(LuaValue::integer(byte as i64))?;
            count += 1;
        }
    }

    Ok(count)
}

/// string.char(...) - Convert bytes to string
/// OPTIMIZED: Avoid clone by consuming Vec in from_utf8
fn string_char(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();
    let nargs = args.len();

    // Stack buffer for small argument counts (most common: string.char(65,66,67))
    if nargs <= 256 {
        let mut buf = [0u8; 256];
        for (i, arg) in args.iter().enumerate() {
            let Some(byte) = arg.as_integer() else {
                return Err(l.error(format!(
                    "bad argument #{} to 'string.char' (number expected)",
                    i + 1
                )));
            };
            if !(0..=255).contains(&byte) {
                return Err(l.error(format!(
                    "bad argument #{} to 'string.char' (value out of range)",
                    i + 1
                )));
            }
            buf[i] = byte as u8;
        }

        let result = match std::str::from_utf8(&buf[..nargs]) {
            Ok(s) => l.vm_mut().create_string(s)?,
            Err(_) => l.vm_mut().create_binary(buf[..nargs].to_vec())?,
        };
        l.push_value(result)?;
        return Ok(1);
    }

    // Fallback for many arguments
    let mut bytes = Vec::with_capacity(nargs);
    for (i, arg) in args.iter().enumerate() {
        let Some(byte) = arg.as_integer() else {
            return Err(l.error(format!(
                "bad argument #{} to 'string.char' (number expected)",
                i + 1
            )));
        };
        if !(0..=255).contains(&byte) {
            return Err(l.error(format!(
                "bad argument #{} to 'string.char' (value out of range)",
                i + 1
            )));
        }
        bytes.push(byte as u8);
    }

    // Consume Vec — no clone needed
    let result = match String::from_utf8(bytes) {
        Ok(s) => l.vm_mut().create_string_owned(s)?,
        Err(e) => l.vm_mut().create_binary(e.into_bytes())?,
    };
    l.push_value(result)?;
    Ok(1)
}

/// string.dump(function [, strip]) - Serialize a function to binary string
fn string_dump(l: &mut LuaState) -> LuaResult<usize> {
    use crate::lua_value::chunk_serializer;

    let func_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'dump' (function expected)".to_string()))?;
    let strip = l.get_arg(2).map(|v| v.is_truthy()).unwrap_or(false);

    // Get the function ID
    let Some(func_obj) = func_value.as_lua_function() else {
        return Err(l.error("bad argument #1 to 'dump' (function expected)".to_string()));
    };

    let vm = l.vm_mut();

    // Check if it's a Lua function (not a C function)
    let chunk = func_obj.chunk();

    // Serialize the chunk with pool access for string constants
    match chunk_serializer::serialize_chunk_with_pool(chunk, strip, &vm.object_allocator) {
        Ok(bytes) => {
            // Create binary value directly - no encoding needed
            let result = vm.create_binary(bytes)?;
            l.push_value(result)?;
            Ok(1)
        }
        Err(e) => Err(l.error(format!("dump error: {}", e))),
    }
}

/// string.len(s) - Return string length in bytes
/// Supports both string and binary types
fn string_len(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'string.len' (string expected)".to_string()))?;

    let len = if let Some(s) = s_value.as_str() {
        s.len()
    } else if let Some(b) = s_value.as_binary() {
        b.len()
    } else {
        return Err(l.error("bad argument #1 to 'string.len' (string expected)".to_string()));
    };

    l.push_value(LuaValue::integer(len as i64))?;
    Ok(1)
}

/// string.lower(s) - Convert to lowercase
/// OPTIMIZED: ASCII stack-buffer fast path, avoids heap allocation for short strings
fn string_lower(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'string.lower' (string expected)".to_string())
    })?;
    let Some(s) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'string.lower' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    if s.is_ascii() {
        let bytes = s.as_bytes();
        let len = bytes.len();
        // Stack buffer for short strings (covers most Lua strings)
        if len <= 256 {
            let mut buf = [0u8; 256];
            for i in 0..len {
                buf[i] = bytes[i].to_ascii_lowercase();
            }
            // SAFETY: ASCII lowercase of valid ASCII is valid UTF-8
            let result_str = unsafe { std::str::from_utf8_unchecked(&buf[..len]) };
            let result = vm.create_string(result_str)?;
            l.push_value(result)?;
        } else {
            let result = s.to_ascii_lowercase();
            let result = vm.create_string_owned(result)?;
            l.push_value(result)?;
        }
    } else {
        let result = s.to_lowercase();
        let result = vm.create_string_owned(result)?;
        l.push_value(result)?;
    }
    Ok(1)
}

/// string.upper(s) - Convert to uppercase
/// OPTIMIZED: ASCII stack-buffer fast path, avoids heap allocation for short strings
fn string_upper(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'string.upper' (string expected)".to_string())
    })?;
    let Some(s) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'string.upper' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    if s.is_ascii() {
        let bytes = s.as_bytes();
        let len = bytes.len();
        if len <= 256 {
            let mut buf = [0u8; 256];
            for i in 0..len {
                buf[i] = bytes[i].to_ascii_uppercase();
            }
            // SAFETY: ASCII uppercase of valid ASCII is valid UTF-8
            let result_str = unsafe { std::str::from_utf8_unchecked(&buf[..len]) };
            let result = vm.create_string(result_str)?;
            l.push_value(result)?;
        } else {
            let result = s.to_ascii_uppercase();
            let result = vm.create_string_owned(result)?;
            l.push_value(result)?;
        }
    } else {
        let result = s.to_uppercase();
        let result = vm.create_string_owned(result)?;
        l.push_value(result)?;
    }
    Ok(1)
}

/// string.rep(s, n [, sep]) - Repeat string
/// OPTIMIZED: Avoid input cloning, pre-allocate result, use byte slices directly
fn string_rep(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'string.rep' (string expected)".to_string()))?;

    // Get raw byte slice WITHOUT cloning to owned Vec
    let (s_bytes, is_binary) = if let Some(s_str) = s_value.as_str() {
        (s_str.as_bytes(), false)
    } else if let Some(binary) = s_value.as_binary() {
        (binary, true)
    } else {
        return Err(l.error("bad argument #1 to 'string.rep' (string expected)".to_string()));
    };

    // Get parameters
    let n_value = l.get_arg(2);
    let sep_value = l.get_arg(3);

    let vm = l.vm_mut();

    let Some(n_value) = n_value else {
        return Err(l.error("bad argument #2 to 'string.rep' (number expected)".to_string()));
    };
    let n = match value_to_integer(&n_value) {
        Ok(i) => i,
        Err(msg) => return Err(l.error(format!("bad argument #2 to 'string.rep' ({})", msg))),
    };

    if n <= 0 {
        let empty = vm.create_string("")?;
        l.push_value(empty)?;
        return Ok(1);
    }

    let s_len = s_bytes.len() as i64;

    // Check for overflow before multiplying
    if s_len > 0 && n > MAX_STRING_SIZE / s_len {
        return Err(l.error("resulting string too large".to_string()));
    }

    // Get separator bytes WITHOUT cloning
    // Bind sep_value to extend its lifetime past the borrow
    let sep_owned = sep_value;
    let sep_bytes: &[u8] = if let Some(ref v) = sep_owned {
        if let Some(s) = v.as_str() {
            s.as_bytes()
        } else if let Some(b) = v.as_binary() {
            b
        } else {
            &[]
        }
    } else {
        &[]
    };

    let sep_len = sep_bytes.len() as i64;
    let sep_total = sep_len.saturating_mul(n - 1);
    let total_size = s_len.saturating_mul(n).saturating_add(sep_total);

    if total_size > MAX_STRING_SIZE {
        return Err(l.error("resulting string too large".to_string()));
    }

    // Pre-allocate result with exact capacity
    let mut result = Vec::with_capacity(total_size as usize);
    if !sep_bytes.is_empty() {
        for i in 0..n {
            if i > 0 {
                result.extend_from_slice(sep_bytes);
            }
            result.extend_from_slice(s_bytes);
        }
    } else {
        for _ in 0..n {
            result.extend_from_slice(s_bytes);
        }
    }

    // Return binary if input was binary, otherwise string
    let result_val = if is_binary {
        vm.create_binary(result)?
    } else {
        // Input was valid UTF-8, repetition is also valid UTF-8
        // SAFETY: repeating valid UTF-8 produces valid UTF-8
        let s = unsafe { String::from_utf8_unchecked(result) };
        vm.create_string_owned(s)?
    };
    l.push_value(result_val)?;
    Ok(1)
}

/// string.reverse(s) - Reverse string
/// OPTIMIZED: Avoid unnecessary clone by consuming Vec in from_utf8
fn string_reverse(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'string.reverse' (string expected)".to_string())
    })?;

    // Accept both string and binary types
    let (s_bytes, is_binary) = if let Some(s) = s_value.as_str() {
        (s.as_bytes(), false)
    } else if let Some(bytes) = s_value.as_binary() {
        (bytes, true)
    } else {
        return Err(l.error("bad argument #1 to 'string.reverse' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    let len = s_bytes.len();

    // Stack buffer for short strings (most common case)
    let result = if is_binary {
        let mut reversed = s_bytes.to_vec();
        reversed.reverse();
        vm.create_binary(reversed)?
    } else if len <= 256 {
        let mut buf = [0u8; 256];
        for i in 0..len {
            buf[i] = s_bytes[len - 1 - i];
        }
        // Byte-reversed UTF-8 may not be valid UTF-8, must check
        match std::str::from_utf8(&buf[..len]) {
            Ok(s) => vm.create_string(s)?,
            Err(_) => vm.create_binary(buf[..len].to_vec())?,
        }
    } else {
        let mut reversed = s_bytes.to_vec();
        reversed.reverse();
        match String::from_utf8(reversed) {
            Ok(s) => vm.create_string_owned(s)?,
            Err(e) => vm.create_binary(e.into_bytes())?,
        }
    };

    l.push_value(result)?;
    Ok(1)
}

/// string.sub(s, i [, j]) - Extract substring
/// ULTRA-OPTIMIZED: Uses create_substring to avoid allocations when possible
fn string_sub(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| crate::stdlib::debug::argerror(l, 1, "string expected"))?;

    // Get string data - handle both string and binary types
    let s_bytes = if let Some(s) = s_value.as_str() {
        s.as_bytes()
    } else if let Some(bytes) = s_value.as_binary() {
        bytes
    } else {
        return Err(crate::stdlib::debug::arg_typeerror(
            l, 1, "string", &s_value,
        ));
    };

    let i_value = l
        .get_arg(2)
        .ok_or_else(|| crate::stdlib::debug::argerror(l, 2, "number expected"))?;
    let i = match value_to_integer(&i_value) {
        Ok(i) => i,
        Err(msg) => return Err(crate::stdlib::debug::argerror(l, 2, msg)),
    };

    let j = l
        .get_arg(3)
        .map(|v| match value_to_integer(&v) {
            Ok(i) => Ok(i),
            Err(msg) => Err(crate::stdlib::debug::argerror(l, 3, msg)),
        })
        .transpose()?
        .unwrap_or(-1);

    // Get string length and compute byte indices
    let vm = l.vm_mut();
    let (start_byte, end_byte) = {
        let byte_len = s_bytes.len() as i64;

        // Lua string.sub uses byte positions, not character positions!
        let start = if i < 0 { byte_len + i + 1 } else { i };
        let end = if j < 0 { byte_len + j + 1 } else { j };

        // Clamp to valid range
        let start = start.max(1).min(byte_len + 1) as usize;
        let end = end.max(0).min(byte_len) as usize;

        if start > 0 && start <= end + 1 {
            let start_byte = (start - 1).min(byte_len as usize);
            let end_byte = end.min(byte_len as usize);
            (start_byte, end_byte)
        } else {
            // Empty string
            (0, 0)
        }
    };

    // Use optimized create_substring
    let result_value = vm.create_substring(s_value, start_byte, end_byte)?;
    l.push_value(result_value)?;
    Ok(1)
}

/// string.find(s, pattern [, init [, plain]]) - Find pattern
/// ULTRA-OPTIMIZED: Avoid string cloning in hot path
fn string_find(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'find' (string expected)".to_string()))?;

    // Get string data - handle both string and binary types
    let s_str = if let Some(s) = s_value.as_str() {
        s
    } else {
        return Err(l.error("bad argument #1 to 'find' (string expected)".to_string()));
    };

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'find' (string expected)".to_string()))?;

    // Get pattern data - handle both string and binary types
    let pattern = if let Some(p) = pattern_value.as_str() {
        p
    } else {
        return Err(l.error("bad argument #2 to 'find' (string expected)".to_string()));
    };

    let init = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let plain = l.get_arg(4).map(|v| v.is_truthy()).unwrap_or(false);

    // Convert Lua 1-based index (with negative support) to Rust 0-based index
    let start_pos = if init > 0 {
        (init - 1) as usize
    } else if init < 0 {
        // Negative index: count from end
        let abs_init = (-init) as usize;
        if abs_init > s_str.len() {
            0
        } else {
            s_str.len() - abs_init
        }
    } else {
        // init == 0, treat as 1
        0
    };

    // Fast path: check if pattern contains special characters
    let has_special = pattern.bytes().any(|c| {
        matches!(
            c,
            b'%' | b'.' | b'[' | b']' | b'*' | b'+' | b'-' | b'?' | b'^' | b'$' | b'(' | b')'
        )
    });

    if plain || !has_special {
        // Plain string search - NO ALLOCATION!
        if start_pos > s_str.len() {
            l.push_value(LuaValue::nil())?;
            return Ok(1);
        }

        if let Some(pos) = s_str[start_pos..].find(pattern) {
            let actual_pos = start_pos + pos;
            let end_pos = actual_pos + pattern.len();
            l.push_value(LuaValue::integer((actual_pos + 1) as i64))?;
            l.push_value(LuaValue::integer(end_pos as i64))?;
            Ok(2)
        } else {
            l.push_value(LuaValue::nil())?;
            Ok(1)
        }
    } else {
        // Complex pattern matching — use pattern2 engine (no parse phase)
        match pattern::find(s_str, pattern, start_pos) {
            Ok(Some((start, end, captures))) => {
                l.push_value(LuaValue::integer((start + 1) as i64))?;
                l.push_value(LuaValue::integer(end as i64))?;

                // Add captures
                for cap in &captures {
                    match cap {
                        pattern::CaptureValue::Substring(start, end) => {
                            let cap_str = l.create_string(&s_str[*start..*end])?;
                            l.push_value(cap_str)?;
                        }
                        pattern::CaptureValue::Position(p) => {
                            l.push_value(LuaValue::integer(*p as i64))?;
                        }
                    }
                }
                Ok(2 + captures.len())
            }
            Ok(None) => {
                l.push_value(LuaValue::nil())?;
                Ok(1)
            }
            Err(e) => Err(l.error(format!("invalid pattern: {}", e))),
        }
    }
}

/// string.match(s, pattern [, init]) - Match pattern
fn string_match(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'match' (string expected)".to_string()))?;
    let Some(s_str) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'match' (string expected)".to_string()));
    };

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'match' (string expected)".to_string()))?;
    let Some(pattern_str) = pattern_value.as_str() else {
        return Err(l.error("bad argument #2 to 'match' (string expected)".to_string()));
    };

    let init = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let start_pos = if init > 0 {
        (init - 1) as usize
    } else if init < 0 {
        let abs_init = (-init) as usize;
        if abs_init > s_str.len() {
            0
        } else {
            s_str.len() - abs_init
        }
    } else {
        0
    };

    match pattern::find(s_str, pattern_str, start_pos) {
        Ok(Some((start, end, captures))) => {
            if captures.is_empty() {
                // No captures, return the matched portion
                let matched_str = l.create_string(&s_str[start..end])?;
                l.push_value(matched_str)?;
                Ok(1)
            } else {
                let ncaps = captures.len();
                for cap in &captures {
                    match cap {
                        pattern::CaptureValue::Substring(start, end) => {
                            let cap_str = l.create_string(&s_str[*start..*end])?;
                            l.push_value(cap_str)?;
                        }
                        pattern::CaptureValue::Position(p) => {
                            l.push_value(LuaValue::integer(*p as i64))?;
                        }
                    }
                }
                Ok(ncaps)
            }
        }
        Ok(None) => {
            l.push_value(LuaValue::nil())?;
            Ok(1)
        }
        Err(e) => Err(l.error(format!("invalid pattern: {}", e))),
    }
}

/// string.gsub(s, pattern, repl [, n]) - Global substitution
/// NOTE: Only string replacement is currently implemented
/// TODO: Function and table replacement need protected_call support in LuaState API
fn string_gsub(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'gsub' (string expected)".to_string()))?;

    // Get string data - handle both string and binary types
    let s_str: String;
    let s_ref = if let Some(s) = s_value.as_str() {
        s
    } else if let Some(bytes) = s_value.as_binary() {
        // For binary data, map each byte to a Latin-1 char (preserves byte identity)
        s_str = bytes.iter().map(|&b| b as char).collect();
        &s_str
    } else {
        return Err(l.error("bad argument #1 to 'gsub' (string expected)".to_string()));
    };

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'gsub' (string expected)".to_string()))?;
    let Some(pattern_str) = pattern_value.as_str() else {
        return Err(l.error("bad argument #2 to 'gsub' (string expected)".to_string()));
    };

    let repl_value = l
        .get_arg(3)
        .ok_or_else(|| l.error("bad argument #3 to 'gsub' (value expected)".to_string()))?;

    let max = l
        .get_arg(4)
        .and_then(|v| v.as_integer())
        .map(|n| n as usize);

    // String replacement
    if let Some(repl_str) = repl_value.as_str() {
        match pattern::gsub(s_ref, pattern_str, repl_str, max) {
            Ok((result_str, count)) => {
                let result = l.create_string(&result_str)?;
                l.push_value(result)?;
                l.push_value(LuaValue::integer(count as i64))?;
                Ok(2)
            }
            Err(e) => Err(l.error(e)),
        }
    } else if repl_value.is_function() {
        let matches = match pattern::find_all_matches(s_ref, pattern_str, 0, max) {
            Ok(m) => m,
            Err(e) => return Err(l.error(format!("invalid pattern: {}", e))),
        };
        let mut result = String::new();
        let mut last_end = 0;
        let mut count = 0;

        for m in &matches {
            result.push_str(&s_ref[last_end..m.start]);

            let args = if m.captures.is_empty() {
                vec![l.create_string(&s_ref[m.start..m.end])?]
            } else {
                let mut captures = vec![];
                for cap in &m.captures {
                    match cap {
                        pattern::CaptureValue::Substring(start, end) => {
                            captures.push(l.create_string(&s_ref[*start..*end])?)
                        }
                        pattern::CaptureValue::Position(p) => {
                            captures.push(LuaValue::integer(*p as i64))
                        }
                    }
                }
                captures
            };

            match l.pcall(repl_value, args) {
                Ok((success, results)) => {
                    if success {
                        if results.is_empty()
                            || results[0].is_nil()
                            || results[0] == LuaValue::boolean(false)
                        {
                            // No return value, nil, or false: use original match
                            result.push_str(&s_ref[m.start..m.end]);
                        } else if let Some(s) = results[0].as_str() {
                            result.push_str(s);
                        } else if let Some(n) = results[0].as_integer() {
                            result.push_str(&n.to_string());
                        } else if let Some(n) = results[0].as_number() {
                            result.push_str(&n.to_string());
                        } else {
                            return Err(l.error(format!(
                                "invalid replacement value (a {})",
                                results[0].type_name()
                            )));
                        }
                    } else {
                        return Err(l.error(format!(
                            "error calling replacement function: {}",
                            results
                                .first()
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown error")
                        )));
                    }
                }
                Err(e) => return Err(e),
            }

            last_end = m.end;
            count += 1;
        }

        result.push_str(&s_ref[last_end..]);

        let result_val = l.create_string(&result)?;
        l.push_value(result_val)?;
        l.push_value(LuaValue::integer(count as i64))?;
        Ok(2)
    } else if repl_value.is_table() {
        // Table replacement
        let matches = match pattern::find_all_matches(s_ref, pattern_str, 0, max) {
            Ok(m) => m,
            Err(e) => return Err(l.error(format!("invalid pattern: {}", e))),
        };
        let mut result = String::new();
        let mut last_end = 0;
        let mut count = 0;

        for m in &matches {
            // Copy text before match
            result.push_str(&s_ref[last_end..m.start]);

            // Table lookup
            let key = if m.captures.is_empty() {
                // No captures, use whole match as key
                l.create_string(&s_ref[m.start..m.end])?
            } else {
                // Use first capture as key
                match &m.captures[0] {
                    pattern::CaptureValue::Substring(start, end) => {
                        l.create_string(&s_ref[*start..*end])?
                    }
                    pattern::CaptureValue::Position(p) => LuaValue::integer(*p as i64),
                }
            };

            let result_val = l.table_get(&repl_value, &key)?.unwrap_or(LuaValue::nil());

            let replacement = if result_val.is_nil() || result_val == LuaValue::boolean(false) {
                // nil or false means no replacement, use original match
                s_ref[m.start..m.end].to_string()
            } else if let Some(s) = result_val.as_str() {
                s.to_string()
            } else if let Some(n) = result_val.as_integer() {
                n.to_string()
            } else if let Some(n) = result_val.as_number() {
                n.to_string()
            } else {
                return Err(l.error(format!(
                    "invalid replacement value (a {})",
                    result_val.type_name()
                )));
            };

            result.push_str(&replacement);
            last_end = m.end;
            count += 1;
        }

        // Copy remaining text
        result.push_str(&s_ref[last_end..]);

        let result_val = l.create_string(&result)?;
        l.push_value(result_val)?;
        l.push_value(LuaValue::integer(count as i64))?;
        Ok(2)
    } else {
        Err(l.error("bad argument #3 to 'gsub' (string/function/table expected)".to_string()))
    }
}

/// string.gmatch(s, pattern [, init]) - Returns an iterator function
/// Usage: for capture in string.gmatch(s, pattern) do ... end
fn string_gmatch(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'gmatch' (string expected)".to_string()))?;

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'gmatch' (string expected)".to_string()))?;

    // Handle init parameter (3rd arg, default 1, can be negative)
    let s_str = s_value
        .as_str()
        .ok_or_else(|| l.error("bad argument #1 to 'gmatch' (string expected)".to_string()))?;
    let init = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let start_pos = if init > 0 {
        (init - 1) as usize
    } else if init < 0 {
        let abs_init = (-init) as usize;
        if abs_init > s_str.len() {
            0
        } else {
            s_str.len() - abs_init
        }
    } else {
        0
    };

    let pattern_str = pattern_value
        .as_str()
        .ok_or_else(|| l.error("bad argument #2 to 'gmatch' (string expected)".to_string()))?;

    // Pre-compute all matches in one shot using find_all_matches
    // (avoids repeated Vec<char>/c2b allocation that find() would do per call)
    let all_matches = match pattern::find_all_matches(s_str, pattern_str, start_pos, None) {
        Ok(m) => m,
        Err(e) => return Err(l.error(format!("invalid pattern: {}", e))),
    };

    // Store pre-computed results as a table of tables
    let results_table = l.create_table(all_matches.len(), 0)?;
    for (i, m) in all_matches.iter().enumerate() {
        let entry = l.create_table(m.captures.len().max(1), 0)?;
        if let Some(entry_ref) = entry.as_table_mut() {
            if m.captures.is_empty() {
                let matched = l.create_string(&s_str[m.start..m.end])?;
                entry_ref.raw_seti(1, matched);
            } else {
                for (j, cap) in m.captures.iter().enumerate() {
                    match cap {
                        pattern::CaptureValue::Substring(start, end) => {
                            let val = l.create_string(&s_str[*start..*end])?;
                            entry_ref.raw_seti((j + 1) as i64, val);
                        }
                        pattern::CaptureValue::Position(p) => {
                            entry_ref.raw_seti((j + 1) as i64, LuaValue::integer(*p as i64));
                        }
                    }
                }
            }
        }
        if let Some(results_ref) = results_table.as_table_mut() {
            results_ref.raw_seti((i + 1) as i64, entry);
        }
        // GC barrier for entry
        if let Some(gc_ptr) = entry.as_gc_ptr() {
            l.gc_barrier_back(gc_ptr);
        }
    }

    // GC barrier for results table
    if let Some(gc_ptr) = results_table.as_gc_ptr() {
        l.gc_barrier_back(gc_ptr);
    }

    // State: {results_table, current_index, total_count, num_captures_per_entry}
    let num_caps_per_entry = if let Some(m) = all_matches.first() {
        if m.captures.is_empty() {
            1
        } else {
            m.captures.len()
        }
    } else {
        1
    };

    let state_table = l.create_table(4, 0)?;
    if let Some(state_ref) = state_table.as_table_mut() {
        state_ref.raw_seti(1, results_table);
        state_ref.raw_seti(2, LuaValue::integer(1)); // current index (1-based)
        state_ref.raw_seti(3, LuaValue::integer(all_matches.len() as i64));
        state_ref.raw_seti(4, LuaValue::integer(num_caps_per_entry as i64));
    }

    if let Some(gc_ptr) = state_table.as_gc_ptr() {
        l.gc_barrier_back(gc_ptr);
    }

    let vm = l.vm_mut();
    let closure = vm.create_c_closure(gmatch_iterator, vec![state_table])?;
    l.push_value(closure)?;
    Ok(1)
}

/// Iterator function for string.gmatch — returns pre-computed results
fn gmatch_iterator(l: &mut LuaState) -> LuaResult<usize> {
    let func_val = l
        .current_frame()
        .map(|frame| frame.func)
        .ok_or_else(|| l.error("gmatch iterator: no active call frame".to_string()))?;

    let state_table_value = if let Some(cclosure) = func_val.as_cclosure() {
        if cclosure.upvalues().is_empty() {
            return Err(l.error("gmatch iterator: no upvalues".to_string()));
        }
        cclosure.upvalues()[0]
    } else {
        return Err(l.error("gmatch iterator: not a function".to_string()));
    };

    let Some(state_ref) = state_table_value.as_table() else {
        return Err(l.error("gmatch iterator: state is not a table".to_string()));
    };

    let current_idx = state_ref
        .raw_geti(2)
        .and_then(|v| v.as_integer())
        .unwrap_or(1);

    let total = state_ref
        .raw_geti(3)
        .and_then(|v| v.as_integer())
        .unwrap_or(0);

    let num_caps = state_ref
        .raw_geti(4)
        .and_then(|v| v.as_integer())
        .unwrap_or(1) as usize;

    if current_idx > total {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    }

    // Get results table
    let Some(results_val) = state_ref.raw_geti(1) else {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    };

    let Some(results_ref) = results_val.as_table() else {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    };

    // Get entry at current index
    let Some(entry_val) = results_ref.raw_geti(current_idx) else {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    };

    let Some(entry_ref) = entry_val.as_table() else {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    };

    // Increment index for next call
    l.raw_seti(&state_table_value, 2, LuaValue::integer(current_idx + 1));

    // Push all captures from the entry
    for j in 1..=num_caps {
        if let Some(val) = entry_ref.raw_geti(j as i64) {
            l.push_value(val)?;
        } else {
            l.push_value(LuaValue::nil())?;
        }
    }

    Ok(num_caps)
}
