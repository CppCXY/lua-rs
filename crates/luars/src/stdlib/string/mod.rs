// String library
// Implements: byte, char, dump, find, format, gmatch, gsub, len, lower,
// match, pack, packsize, rep, reverse, sub, unpack, upper
mod pack;
mod pattern;
mod string_format;

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState};

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
/// ULTRA-OPTIMIZED: Direct pointer access, no allocation!
fn string_byte(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'string.byte' (string expected)".to_string()))?;
    
    // 直接获取字符串引用 - 无需 to_vec()！
    let Some(s) = s_value.as_string_str() else {
        return Err(l.error("bad argument #1 to 'string.byte' (string expected)".to_string()));
    };

    let i = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(1);
    let j = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(i);

    let bytes = s.as_bytes(); // 直接获取字节切片，零拷贝！
    let len = bytes.len() as i64;

    // Convert negative indices
    let start = if i < 0 { len + i + 1 } else { i };
    let end = if j < 0 { len + j + 1 } else { j };

    if start < 1 || start > len {
        return Ok(0);
    }

    let end = end.min(len);

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
fn string_char(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();

    let mut bytes = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let Some(byte) = arg.as_integer() else {
            return Err(l.error(format!(
                "bad argument #{} to 'string.char' (number expected)",
                i + 1
            )));
        };
        if byte < 0 || byte > 255 {
            return Err(l.error(format!(
                "bad argument #{} to 'string.char' (value out of range)",
                i + 1
            )));
        }

        bytes.push(byte as u8);
    }

    let result_str = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => {
            return Err(l.error("invalid byte sequence in 'string.char'".to_string()));
        }
    };
    let result = l.vm_mut().create_string_owned(result_str);
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
    let Some(func_id) = func_value.as_function_id() else {
        return Err(l.error("bad argument #1 to 'dump' (function expected)".to_string()));
    };

    let vm = l.vm_mut();

    // Get the function from object pool
    let Some(func) = vm.object_pool.get_function(func_id) else {
        return Err(l.error("bad argument #1 to 'dump' (function expected)".to_string()));
    };

    // Check if it's a Lua function (not a C function)
    let Some(chunk) = func.data.chunk() else {
        return Err(l.error("unable to dump given function".to_string()));
    };

    // Clone the chunk to avoid borrow issues
    let chunk = chunk.clone();

    // Serialize the chunk with pool access for string constants
    match chunk_serializer::serialize_chunk_with_pool(&chunk, strip, &vm.object_pool) {
        Ok(bytes) => {
            // Convert bytes to a string using Latin-1 encoding (each byte -> char)
            // This is how Lua handles binary strings
            let result_str: String = bytes.iter().map(|&b| b as char).collect();
            let result = vm.create_string_owned(result_str);
            l.push_value(result)?;
            Ok(1)
        }
        Err(e) => Err(l.error(format!("dump error: {}", e))),
    }
}

/// string.len(s) - Return string length
/// OPTIMIZED: Use byte length directly, Lua string.len returns byte length not char count
fn string_len(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'string.len' (string expected)".to_string()))?;

    let Some(string_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'string.len' (string expected)".to_string()));
    };
    let Some(s) = l.vm_mut().object_pool.get_string(string_id) else {
        return Err(l.error("bad argument #1 to 'string.len' (string expected)".to_string()));
    };

    // Lua string.len returns byte length, not UTF-8 character count
    // This is correct and much faster than chars().count()
    let len = s.len() as i64;
    l.push_value(LuaValue::integer(len))?;
    Ok(1)
}

/// string.lower(s) - Convert to lowercase
/// OPTIMIZED: ASCII fast path
fn string_lower(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'string.lower' (string expected)".to_string())
    })?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'string.lower' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    let result = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(l.error("bad argument #1 to 'string.lower' (string expected)".to_string()));
        };
        let str_ref = s;
        // ASCII fast path: if all bytes are ASCII, use make_ascii_lowercase
        if str_ref.is_ascii() {
            str_ref.to_ascii_lowercase()
        } else {
            str_ref.to_lowercase()
        }
    };
    let result = vm.create_string_owned(result);
    l.push_value(result)?;
    Ok(1)
}

/// string.upper(s) - Convert to uppercase
/// OPTIMIZED: ASCII fast path
fn string_upper(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'string.upper' (string expected)".to_string())
    })?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'string.upper' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    let result = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(l.error("bad argument #1 to 'string.upper' (string expected)".to_string()));
        };
        let str_ref = s;
        // ASCII fast path
        if str_ref.is_ascii() {
            str_ref.to_ascii_uppercase()
        } else {
            str_ref.to_uppercase()
        }
    };
    let result = vm.create_string_owned(result);
    l.push_value(result)?;
    Ok(1)
}

/// string.rep(s, n [, sep]) - Repeat string
fn string_rep(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'string.rep' (string expected)".to_string()))?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'string.rep' (string expected)".to_string()));
    };

    // Get parameters before borrowing vm_mut
    let n_value = l.get_arg(2);
    let sep_value = l.get_arg(3);

    let vm = l.vm_mut();
    let s_str = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(l.error("bad argument #1 to 'string.rep' (string expected)".to_string()));
        };
        s.to_string()
    };

    let Some(n_value) = n_value else {
        return Err(l.error("bad argument #2 to 'string.rep' (number expected)".to_string()));
    };
    let Some(n) = n_value.as_integer() else {
        return Err(l.error("bad argument #2 to 'string.rep' (number expected)".to_string()));
    };

    if n <= 0 {
        let empty = vm.create_string("");
        l.push_value(empty)?;
        return Ok(1);
    }

    let mut result = String::new();
    match sep_value {
        Some(v) => {
            let sep_str = if let Some(sep_id) = v.as_string_id() {
                if let Some(sep) = vm.object_pool.get_string(sep_id) {
                    sep.to_string()
                } else {
                    return Err(
                        l.error("bad argument #3 to 'string.rep' (string expected)".to_string())
                    );
                }
            } else {
                return Err(
                    l.error("bad argument #3 to 'string.rep' (string expected)".to_string())
                );
            };

            for i in 0..n {
                if i > 0 && !sep_str.is_empty() {
                    result.push_str(&sep_str);
                }
                result.push_str(&s_str);
            }
        }
        None => {
            for _ in 0..n {
                result.push_str(&s_str);
            }
        }
    };

    let result = vm.create_string_owned(result);
    l.push_value(result)?;
    Ok(1)
}

/// string.reverse(s) - Reverse string
fn string_reverse(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'string.reverse' (string expected)".to_string())
    })?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'string.reverse' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    let reversed = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(
                l.error("bad argument #1 to 'string.reverse' (string expected)".to_string())
            );
        };
        s.chars().rev().collect::<String>()
    };
    let result = vm.create_string_owned(reversed);
    l.push_value(result)?;
    Ok(1)
}

/// string.sub(s, i [, j]) - Extract substring
/// ULTRA-OPTIMIZED: Uses create_substring to avoid allocations when possible
fn string_sub(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'string.sub' (string expected)".to_string()))?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'string.sub' (string expected)".to_string()));
    };

    let i_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'string.sub' (number expected)".to_string()))?;
    let Some(i) = i_value.as_integer() else {
        return Err(l.error("bad argument #2 to 'string.sub' (number expected)".to_string()));
    };

    let j = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(-1);

    // Get string length and compute byte indices
    let vm = l.vm_mut();
    let (start_byte, end_byte) = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(l.error("bad argument #1 to 'string.sub' (string expected)".to_string()));
        };
        let byte_len = s.len() as i64;

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
    let result_value = vm
        .object_pool
        .create_substring(s_value, start_byte, end_byte);
    l.push_value(result_value)?;
    Ok(1)
}

/// string.find(s, pattern [, init [, plain]]) - Find pattern
/// ULTRA-OPTIMIZED: Avoid string cloning in hot path
fn string_find(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'find' (string expected)".to_string()))?;
    let s_id = s_value
        .as_string_id()
        .ok_or_else(|| l.error("bad argument #1 to 'find' (string expected)".to_string()))?;

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'find' (string expected)".to_string()))?;
    let pattern_id = pattern_value
        .as_string_id()
        .ok_or_else(|| l.error("bad argument #2 to 'find' (string expected)".to_string()))?;

    let init = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let plain = l.get_arg(4).map(|v| v.is_truthy()).unwrap_or(false);
    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };

    // OPTIMIZATION: Get string references without cloning first
    let (s_str, pattern) = {
        let vm = l.vm_mut();
        let s_lua = vm.object_pool.get_string(s_id);
        let pattern_lua = vm.object_pool.get_string(pattern_id);
        match (s_lua, pattern_lua) {
            (Some(s), Some(p)) => Ok((s.to_string(), p.to_string())),
            _ => Err("invalid string".to_string()),
        }
    }
    .map_err(|e| l.error(e))?;

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

        if let Some(pos) = s_str[start_pos..].find(&pattern) {
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
        // Complex pattern matching
        match pattern::parse_pattern(&pattern) {
            Ok(parsed_pattern) => {
                // Fast path: if pattern is just a literal string, use plain search
                if let Some(literal) = parsed_pattern.as_literal_string() {
                    if start_pos > s_str.len() {
                        l.push_value(LuaValue::nil())?;
                        return Ok(1);
                    }

                    if let Some(pos) = s_str[start_pos..].find(&literal) {
                        let actual_pos = start_pos + pos;
                        let end_pos = actual_pos + literal.len();
                        l.push_value(LuaValue::integer((actual_pos + 1) as i64))?;
                        l.push_value(LuaValue::integer(end_pos as i64))?;
                        Ok(2)
                    } else {
                        l.push_value(LuaValue::nil())?;
                        Ok(1)
                    }
                } else {
                    // Complex pattern - use full pattern matcher
                    if let Some((start, end, captures)) =
                        pattern::find(&s_str, &parsed_pattern, start_pos)
                    {
                        l.push_value(LuaValue::integer((start + 1) as i64))?;
                        l.push_value(LuaValue::integer(end as i64))?;

                        // Add captures
                        for cap in captures {
                            let cap_str = l.create_string(&cap);
                            l.push_value(cap_str)?;
                        }
                        Ok(2 + pattern::find(&s_str, &parsed_pattern, start_pos)
                            .map(|(_, _, caps)| caps.len())
                            .unwrap_or(0))
                    } else {
                        l.push_value(LuaValue::nil())?;
                        Ok(1)
                    }
                }
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
    let string_id = s_value
        .as_string_id()
        .ok_or_else(|| l.error("bad argument #1 to 'match' (string expected)".to_string()))?;

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'match' (string expected)".to_string()))?;
    let pattern_id = pattern_value
        .as_string_id()
        .ok_or_else(|| l.error("bad argument #2 to 'match' (string expected)".to_string()))?;

    let (s_str, pattern_str) = {
        let vm = l.vm_mut();
        let s = vm.object_pool.get_string(string_id);
        let p = vm.object_pool.get_string(pattern_id);
        match (s, p) {
            (Some(s_obj), Some(p_obj)) => Ok((s_obj.to_string(), p_obj.to_string())),
            _ => Err("invalid string".to_string()),
        }
    }
    .map_err(|e| l.error(e))?;

    let init = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };
    let text = s_str[start_pos..].to_string();

    match pattern::parse_pattern(&pattern_str) {
        Ok(pattern) => {
            if let Some((start, end, captures)) = pattern::find(&text, &pattern, 0) {
                if captures.is_empty() {
                    // No captures, return the matched portion
                    let matched = text[start..end].to_string();
                    let matched_str = l.create_string(&matched);
                    l.push_value(matched_str)?;
                    Ok(1)
                } else {
                    // Return captures
                    for cap in captures {
                        let cap_str = l.create_string(&cap);
                        l.push_value(cap_str)?;
                    }
                    Ok(pattern::find(&text, &pattern, 0)
                        .map(|(_, _, caps)| caps.len())
                        .unwrap_or(0))
                }
            } else {
                l.push_value(LuaValue::nil())?;
                Ok(1)
            }
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
    let s_id = s_value
        .as_string_id()
        .ok_or_else(|| l.error("bad argument #1 to 'gsub' (string expected)".to_string()))?;

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'gsub' (string expected)".to_string()))?;
    let pattern_id = pattern_value
        .as_string_id()
        .ok_or_else(|| l.error("bad argument #2 to 'gsub' (string expected)".to_string()))?;

    let repl_value = l
        .get_arg(3)
        .ok_or_else(|| l.error("bad argument #3 to 'gsub' (value expected)".to_string()))?;

    let repl_id = repl_value
        .as_string_id()
        .ok_or_else(|| l.error("bad argument #3 to 'gsub' (string expected)".to_string()))?;

    // Get all strings first before any operations
    let (s_str, pattern_str, repl_str) = {
        let vm = l.vm_mut();
        let s = vm.object_pool.get_string(s_id);
        let p = vm.object_pool.get_string(pattern_id);
        let r = vm.object_pool.get_string(repl_id);

        match (s, p, r) {
            (Some(s_obj), Some(p_obj), Some(r_obj)) => {
                Ok((s_obj.to_string(), p_obj.to_string(), r_obj.to_string()))
            }
            _ => Err("invalid string".to_string()),
        }
    }
    .map_err(|e| l.error(e))?;

    let max = l
        .get_arg(4)
        .and_then(|v| v.as_integer())
        .map(|n| n as usize);

    let pattern = match pattern::parse_pattern(&pattern_str) {
        Ok(p) => p,
        Err(e) => return Err(l.error(format!("invalid pattern: {}", e))),
    };

    // Currently only support string replacement
    if repl_value.is_string() {
        match pattern::gsub(&s_str, &pattern, &repl_str, max) {
            Ok((result_str, count)) => {
                let result = l.create_string(&result_str);
                l.push_value(result)?;
                l.push_value(LuaValue::integer(count as i64))?;
                Ok(2)
            }
            Err(e) => Err(l.error(e)),
        }
    } else {
        // TODO: Implement function and table replacement when LuaState supports pcall
        Err(l.error("gsub with function/table replacement not yet implemented".to_string()))
    }
}

/// string.gmatch(s, pattern) - Returns an iterator function
/// Usage: for capture in string.gmatch(s, pattern) do ... end
fn string_gmatch(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'gmatch' (string expected)".to_string()))?;

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'gmatch' (string expected)".to_string()))?;

    // Create state table: {string, pattern, position}
    let state_table = l.create_table(3, 0);
    let Some(table_id) = state_table.as_table_id() else {
        return Err(l.error("failed to create state table for gmatch".to_string()));
    };

    {
        let vm = l.vm_mut();
        if let Some(state_ref) = vm.object_pool.get_table_mut(table_id) {
            state_ref.set_int(1, s_value);
            state_ref.set_int(2, pattern_value);
            state_ref.set_int(3, LuaValue::integer(0)); // position
        }
    }

    // Return: iterator function, state table, nil (initial control variable)
    l.push_value(LuaValue::cfunction(gmatch_iterator))?;
    l.push_value(state_table)?;
    l.push_value(LuaValue::nil())?;
    Ok(3)
}

/// Iterator function for string.gmatch
/// Called as: f(state, control_var)
fn gmatch_iterator(l: &mut LuaState) -> LuaResult<usize> {
    // Arg 1: state table
    // Arg 2: control variable (unused, we use state.position)
    let state_table_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("gmatch iterator: state expected".to_string()))?;

    let Some(table_id) = state_table_value.as_table_id() else {
        return Err(l.error("gmatch iterator: state is not a table".to_string()));
    };

    // Extract string, pattern, and position from state
    let (s_str, pattern_str, position) = {
        let vm = l.vm_mut();
        let Some(state_ref) = vm.object_pool.get_table(table_id) else {
            return Err(l.error("gmatch iterator: state is not a table".to_string()));
        };

        let Some(s_val) = state_ref.get_int(1) else {
            return Err(l.error("gmatch iterator: string not found in state".to_string()));
        };
        let Some(s_id) = s_val.as_string_id() else {
            return Err(l.error("gmatch iterator: string invalid".to_string()));
        };

        let Some(pattern_val) = state_ref.get_int(2) else {
            return Err(l.error("gmatch iterator: pattern not found in state".to_string()));
        };
        let Some(pattern_id) = pattern_val.as_string_id() else {
            return Err(l.error("gmatch iterator: pattern invalid".to_string()));
        };

        let position_value = state_ref.get_int(3).unwrap_or(LuaValue::integer(0));
        let position = position_value.as_integer().unwrap_or(0) as usize;

        // Get string contents
        let Some(s_obj) = vm.object_pool.get_string(s_id) else {
            return Err(l.error("gmatch iterator: string invalid".to_string()));
        };
        let Some(pattern_obj) = vm.object_pool.get_string(pattern_id) else {
            return Err(l.error("gmatch iterator: pattern invalid".to_string()));
        };

        (s_obj.to_string(), pattern_obj.to_string(), position)
    };

    // Parse pattern
    let pattern = match pattern::parse_pattern(&pattern_str) {
        Ok(p) => p,
        Err(e) => return Err(l.error(format!("invalid pattern: {}", e))),
    };

    // Find next match
    if let Some((start, end, captures)) = pattern::find(&s_str, &pattern, position) {
        // Update position for next iteration
        let next_pos = if end > start { end } else { end + 1 };
        {
            let vm = l.vm_mut();
            if let Some(state_ref) = vm.object_pool.get_table_mut(table_id) {
                state_ref.set_int(3, LuaValue::integer(next_pos as i64));
            }
        }

        // Return captures if any, otherwise return the matched string
        if captures.is_empty() {
            let matched = &s_str[start..end];
            let result = l.create_string(matched);
            l.push_value(result)?;
            Ok(1)
        } else {
            for cap in &captures {
                let result = l.create_string(cap);
                l.push_value(result)?;
            }
            Ok(captures.len())
        }
    } else {
        // No more matches - return nil to end iteration
        l.push_value(LuaValue::nil())?;
        Ok(1)
    }
}

// /// string.gmatch(s, pattern) - Returns an iterator function
// /// Usage: for capture in string.gmatch(s, pattern) do ... end
// fn string_gmatch(vm: &mut LuaVM) -> LuaResult<MultiValue> {
//     let s_value = require_arg(vm, 1, "string.gmatch")?;
//     if !s_value.is_string() {
//         return Err(vm.error("bad argument #1 to 'string.gmatch' (string expected)".to_string()));
//     };

//     let pattern_value = require_arg(vm, 2, "string.gmatch")?;
//     if !pattern_value.is_string() {
//         return Err(vm.error("bad argument #2 to 'string.gmatch' (string expected)".to_string()));
//     };

//     // Create state table: {string = s, pattern = p, position = 0}
//     let state_table = vm.create_table(3, 0);
//     let Some(table_id) = state_table.as_table_id() else {
//         return Err(vm.error("failed to create state table for gmatch".to_string()));
//     };
//     if let Some(state_ref) = vm.object_pool.get_table_mut(table_id) {
//         state_ref.set_int(1, s_value);
//         state_ref.set_int(2, pattern_value);
//         state_ref.set_int(3, LuaValue::integer(0));
//     }
//     // Return: iterator function, state table, nil (initial control variable)
//     // TODO: Convert gmatch_iterator to new signature
//     Ok(MultiValue::multiple(vec![
//         // LuaValue::cfunction(gmatch_iterator),
//         LuaValue::nil(), // placeholder
//         state_table,
//         LuaValue::nil(),
//     ]))
// }

// /// Iterator function for string.gmatch
// /// Called as: f(state, control_var)
// fn gmatch_iterator(vm: &mut LuaVM) -> LuaResult<MultiValue> {
//     // Arg 0: state table
//     // Arg 1: control variable (unused, we use state.position)
//     let state_table_value = require_arg(vm, 1, "gmatch iterator")?;

//     // Extract string, pattern, and position from state
//     let Some(table_id) = state_table_value.as_table_id() else {
//         return Err(vm.error("gmatch iterator: state is not a table".to_string()));
//     };

//     // Extract all values from table first
//     let (s_str, pattern_str_owned, position) = {
//         let Some(state_ref) = vm.object_pool.get_table(table_id) else {
//             return Err(vm.error("gmatch iterator: state is not a table".to_string()));
//         };

//         let Some(s_val) = state_ref.get_int(1) else {
//             return Err(vm.error("gmatch iterator: string not found in state".to_string()));
//         };
//         let Some(s_id) = s_val.as_string_id() else {
//             return Err(vm.error("gmatch iterator: string invalid".to_string()));
//         };

//         let Some(pattern_val) = state_ref.get_int(2) else {
//             return Err(vm.error("gmatch iterator: pattern not found in state".to_string()));
//         };
//         let Some(pattern_id) = pattern_val.as_string_id() else {
//             return Err(vm.error("gmatch iterator: pattern invalid".to_string()));
//         };

//         let position_value = state_ref.get_int(3).unwrap_or(LuaValue::integer(0));
//         let position = position_value.as_integer().unwrap_or(0) as usize;

//         // Get string contents
//         let Some(s_obj) = vm.object_pool.get_string(s_id) else {
//             return Err(vm.error("gmatch iterator: string invalid".to_string()));
//         };
//         let Some(pattern_obj) = vm.object_pool.get_string(pattern_id) else {
//             return Err(vm.error("gmatch iterator: pattern invalid".to_string()));
//         };

//         (
//             s_obj.to_string(),
//             pattern_obj.to_string(),
//             position,
//         )
//     };

//     // Parse pattern
//     let pattern = match pattern::parse_pattern(&pattern_str_owned) {
//         Ok(p) => p,
//         Err(e) => return Err(vm.error(format!("invalid pattern: {}", e))),
//     };

//     // Find next match
//     if let Some((start, end, captures)) = pattern::find(&s_str, &pattern, position) {
//         // Update position for next iteration
//         let next_pos = if end > start { end } else { end + 1 };
//         if let Some(state_ref) = vm.object_pool.get_table_mut(table_id) {
//             state_ref.set_int(3, LuaValue::integer(next_pos as i64));
//         }

//         // Return captures if any, otherwise return the matched string
//         if captures.is_empty() {
//             let matched = &s_str[start..end];
//             Ok(MultiValue::single(vm.create_string(matched)))
//         } else {
//             let mut results = Vec::new();
//             for cap in captures {
//                 results.push(vm.create_string(&cap));
//             }
//             Ok(MultiValue::multiple(results))
//         }
//     } else {
//         // No more matches
//         Ok(MultiValue::single(LuaValue::nil()))
//     }
// }

// /// string.pack(fmt, v1, v2, ...) - Pack values into binary string
// /// Simplified implementation supporting basic format codes
// fn string_pack(vm: &mut LuaVM) -> LuaResult<MultiValue> {
//     let fmt_value = require_arg(vm, 1, "string.pack")?;
//     let Some(fmt_id) = fmt_value.as_string_id() else {
//         return Err(vm.error("bad argument #1 to 'string.pack' (string expected)".to_string()));
//     };
//     let fmt_str = {
//         let Some(fmt) = vm.object_pool.get_string(fmt_id) else {
//             return Err(vm.error("bad argument #1 to 'string.pack' (string expected)".to_string()));
//         };
//         fmt.as_str().to_string()
//     };

//     let args = get_args(vm);
//     let values = &args[1..]; // Skip format string

//     let mut result = Vec::new();
//     let mut value_idx = 0;
//     let mut chars = fmt_str.chars();

//     while let Some(ch) = chars.next() {
//         match ch {
//             ' ' | '\t' | '\n' | '\r' => continue, // Skip whitespace
//             'b' => {
//                 // signed byte
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let n = values[value_idx].as_integer().ok_or_else(|| {
//                     vm.error("bad argument to 'string.pack' (number expected)".to_string())
//                 })?;
//                 result.push((n & 0xFF) as u8);
//                 value_idx += 1;
//             }
//             'B' => {
//                 // unsigned byte
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let n = values[value_idx].as_integer().ok_or_else(|| {
//                     vm.error("bad argument to 'string.pack' (number expected)".to_string())
//                 })?;
//                 result.push((n & 0xFF) as u8);
//                 value_idx += 1;
//             }
//             'h' => {
//                 // signed short (2 bytes, little-endian)
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let n = values[value_idx].as_integer().ok_or_else(|| {
//                     vm.error("bad argument to 'string.pack' (number expected)".to_string())
//                 })? as i16;
//                 result.extend_from_slice(&n.to_le_bytes());
//                 value_idx += 1;
//             }
//             'H' => {
//                 // unsigned short (2 bytes, little-endian)
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let n = values[value_idx].as_integer().ok_or_else(|| {
//                     vm.error("bad argument to 'string.pack' (number expected)".to_string())
//                 })? as u16;
//                 result.extend_from_slice(&n.to_le_bytes());
//                 value_idx += 1;
//             }
//             'i' | 'l' => {
//                 // signed int (4 bytes, little-endian)
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let n = values[value_idx].as_integer().ok_or_else(|| {
//                     vm.error("bad argument to 'string.pack' (number expected)".to_string())
//                 })? as i32;
//                 result.extend_from_slice(&n.to_le_bytes());
//                 value_idx += 1;
//             }
//             'I' | 'L' => {
//                 // unsigned int (4 bytes, little-endian)
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let n = values[value_idx].as_integer().ok_or_else(|| {
//                     vm.error("bad argument to 'string.pack' (number expected)".to_string())
//                 })? as u32;
//                 result.extend_from_slice(&n.to_le_bytes());
//                 value_idx += 1;
//             }
//             'f' => {
//                 // float (4 bytes, little-endian)
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let n = values[value_idx].as_number().ok_or_else(|| {
//                     vm.error("bad argument to 'string.pack' (number expected)".to_string())
//                 })? as f32;
//                 result.extend_from_slice(&n.to_le_bytes());
//                 value_idx += 1;
//             }
//             'd' => {
//                 // double (8 bytes, little-endian)
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let n = values[value_idx].as_number().ok_or_else(|| {
//                     vm.error("bad argument to 'string.pack' (number expected)".to_string())
//                 })?;
//                 result.extend_from_slice(&n.to_le_bytes());
//                 value_idx += 1;
//             }
//             'z' => {
//                 // zero-terminated string
//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let s_str = {
//                     let Some(s_id) = values[value_idx].as_string_id() else {
//                         return Err(
//                             vm.error("bad argument to 'string.pack' (string expected)".to_string())
//                         );
//                     };
//                     let Some(s) = vm.object_pool.get_string(s_id) else {
//                         return Err(
//                             vm.error("bad argument to 'string.pack' (string expected)".to_string())
//                         );
//                     };
//                     s.to_string()
//                 };
//                 result.extend_from_slice(s_str.as_bytes());
//                 result.push(0); // null terminator
//                 value_idx += 1;
//             }
//             'c' => {
//                 // fixed-length string - need to read size
//                 let mut size_str = String::new();
//                 loop {
//                     match chars.next() {
//                         Some(digit) if digit.is_ascii_digit() => size_str.push(digit),
//                         _ => break,
//                     }
//                 }
//                 let size: usize = size_str.parse().map_err(|_| {
//                     vm.error("bad argument to 'string.pack' (invalid size)".to_string())
//                 })?;

//                 if value_idx >= values.len() {
//                     return Err(
//                         vm.error("bad argument to 'string.pack' (not enough values)".to_string())
//                     );
//                 }
//                 let s_str = {
//                     let Some(s_id) = values[value_idx].as_string_id() else {
//                         return Err(
//                             vm.error("bad argument to 'string.pack' (string expected)".to_string())
//                         );
//                     };
//                     let Some(s) = vm.object_pool.get_string(s_id) else {
//                         return Err(
//                             vm.error("bad argument to 'string.pack' (string expected)".to_string())
//                         );
//                     };
//                     s.to_string()
//                 };
//                 let bytes = s_str.as_bytes();
//                 result.extend_from_slice(&bytes[..size.min(bytes.len())]);
//                 // Pad with zeros if needed
//                 for _ in bytes.len()..size {
//                     result.push(0);
//                 }
//                 value_idx += 1;
//             }
//             _ => {
//                 return Err(vm.error(format!(
//                     "bad argument to 'string.pack' (invalid format option '{}')",
//                     ch
//                 )));
//             }
//         }
//     }

//     // Create a string directly from bytes without UTF-8 validation
//     // Lua strings can contain arbitrary binary data
//     let packed = unsafe { String::from_utf8_unchecked(result) };
//     Ok(MultiValue::single(vm.create_string(&packed)))
// }

// /// string.packsize(fmt) - Return size of packed data
// fn string_packsize(vm: &mut LuaVM) -> LuaResult<MultiValue> {
//     let fmt_value = require_arg(vm, 1, "string.packsize")?;
//     let Some(fmt_id) = fmt_value.as_string_id() else {
//         return Err(vm.error("bad argument #1 to 'string.packsize' (string expected)".to_string()));
//     };
//     let fmt_str = {
//         let Some(fmt) = vm.object_pool.get_string(fmt_id) else {
//             return Err(
//                 vm.error("bad argument #1 to 'string.packsize' (string expected)".to_string())
//             );
//         };
//         fmt.as_str().to_string()
//     };

//     let mut size = 0usize;
//     let mut chars = fmt_str.chars().peekable();

//     while let Some(ch) = chars.next() {
//         match ch {
//             ' ' | '\t' | '\n' | '\r' => continue,
//             'b' | 'B' => size += 1,
//             'h' | 'H' => size += 2,
//             'l' | 'L' | 'f' => size += 4,
//             // 'i' and 'I' can have optional size specifier
//             'i' | 'I' => {
//                 // Check for size specifier
//                 let mut size_str = String::new();
//                 while let Some(&digit) = chars.peek() {
//                     if digit.is_ascii_digit() {
//                         size_str.push(chars.next().unwrap());
//                     } else {
//                         break;
//                     }
//                 }
//                 let n: usize = if size_str.is_empty() {
//                     4 // default size
//                 } else {
//                     size_str.parse().unwrap_or(4)
//                 };
//                 size += n;
//             }
//             'd' | 'n' => size += 8, // 'n' is lua_Number (double)
//             'j' | 'J' | 'T' => size += std::mem::size_of::<i64>(), // lua_Integer / size_t
//             's' => {
//                 // Check for size specifier
//                 let mut size_str = String::new();
//                 while let Some(&digit) = chars.peek() {
//                     if digit.is_ascii_digit() {
//                         size_str.push(chars.next().unwrap());
//                     } else {
//                         break;
//                     }
//                 }
//                 let _n: usize = if size_str.is_empty() {
//                     std::mem::size_of::<usize>() // default size_t
//                 } else {
//                     size_str.parse().unwrap_or(std::mem::size_of::<usize>())
//                 };
//                 return Err(vm.error("variable-length format 's' in 'string.packsize'".to_string()));
//             }
//             'z' => {
//                 return Err(vm.error("variable-length format in 'string.packsize'".to_string()));
//             }
//             'c' => {
//                 let mut size_str = String::new();
//                 while let Some(&digit) = chars.peek() {
//                     if digit.is_ascii_digit() {
//                         size_str.push(chars.next().unwrap());
//                     } else {
//                         break;
//                     }
//                 }
//                 let n: usize = size_str.parse().map_err(|_| {
//                     vm.error("bad argument to 'string.packsize' (invalid size)".to_string())
//                 })?;
//                 size += n;
//             }
//             'x' => size += 1,           // padding byte
//             'X' => {}                   // empty alignment
//             '<' | '>' | '=' | '!' => {} // endianness/alignment modifiers
//             _ => {
//                 return Err(vm.error(format!(
//                     "bad argument to 'string.packsize' (invalid format option '{}')",
//                     ch
//                 )));
//             }
//         }
//     }

//     Ok(MultiValue::single(LuaValue::integer(size as i64)))
// }

// /// string.unpack(fmt, s [, pos]) - Unpack binary string
// fn string_unpack(vm: &mut LuaVM) -> LuaResult<MultiValue> {
//     let fmt_value = require_arg(vm, 1, "string.unpack")?;
//     let Some(fmt_id) = fmt_value.as_string_id() else {
//         return Err(vm.error("bad argument #1 to 'string.unpack' (string expected)".to_string()));
//     };
//     let fmt_str = {
//         let Some(fmt) = vm.object_pool.get_string(fmt_id) else {
//             return Err(
//                 vm.error("bad argument #1 to 'string.unpack' (string expected)".to_string())
//             );
//         };
//         fmt.as_str().to_string()
//     };

//     let s_value = require_arg(vm, 2, "string.unpack")?;
//     let Some(s_id) = s_value.as_string_id() else {
//         return Err(vm.error("bad argument #2 to 'string.unpack' (string expected)".to_string()));
//     };
//     let s_str = {
//         let Some(s) = vm.object_pool.get_string(s_id) else {
//             return Err(
//                 vm.error("bad argument #2 to 'string.unpack' (string expected)".to_string())
//             );
//         };
//         s.to_string()
//     };
//     let bytes = s_str.as_bytes();

//     let pos = get_arg(vm, 3).and_then(|v| v.as_integer()).unwrap_or(1) as usize - 1; // Convert to 0-based

//     let mut results = Vec::new();
//     let mut idx = pos;
//     let mut chars = fmt_str.chars();

//     while let Some(ch) = chars.next() {
//         match ch {
//             ' ' | '\t' | '\n' | '\r' => continue,
//             'b' => {
//                 if idx >= bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 results.push(LuaValue::integer(bytes[idx] as i8 as i64));
//                 idx += 1;
//             }
//             'B' => {
//                 if idx >= bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 results.push(LuaValue::integer(bytes[idx] as i64));
//                 idx += 1;
//             }
//             'h' => {
//                 if idx + 2 > bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 let val = i16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
//                 results.push(LuaValue::integer(val as i64));
//                 idx += 2;
//             }
//             'H' => {
//                 if idx + 2 > bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 let val = u16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
//                 results.push(LuaValue::integer(val as i64));
//                 idx += 2;
//             }
//             'i' | 'l' => {
//                 if idx + 4 > bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 let val = i32::from_le_bytes([
//                     bytes[idx],
//                     bytes[idx + 1],
//                     bytes[idx + 2],
//                     bytes[idx + 3],
//                 ]);
//                 results.push(LuaValue::integer(val as i64));
//                 idx += 4;
//             }
//             'I' | 'L' => {
//                 if idx + 4 > bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 let val = u32::from_le_bytes([
//                     bytes[idx],
//                     bytes[idx + 1],
//                     bytes[idx + 2],
//                     bytes[idx + 3],
//                 ]);
//                 results.push(LuaValue::integer(val as i64));
//                 idx += 4;
//             }
//             'f' => {
//                 if idx + 4 > bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 let val = f32::from_le_bytes([
//                     bytes[idx],
//                     bytes[idx + 1],
//                     bytes[idx + 2],
//                     bytes[idx + 3],
//                 ]);
//                 results.push(LuaValue::float(val as f64));
//                 idx += 4;
//             }
//             'd' => {
//                 if idx + 8 > bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 let mut arr = [0u8; 8];
//                 arr.copy_from_slice(&bytes[idx..idx + 8]);
//                 let val = f64::from_le_bytes(arr);
//                 results.push(LuaValue::float(val));
//                 idx += 8;
//             }
//             'z' => {
//                 // Read null-terminated string
//                 let start = idx;
//                 while idx < bytes.len() && bytes[idx] != 0 {
//                     idx += 1;
//                 }
//                 let s = String::from_utf8_lossy(&bytes[start..idx]);
//                 results.push(vm.create_string(&s));
//                 idx += 1; // Skip null terminator
//             }
//             'c' => {
//                 let mut size_str = String::new();
//                 loop {
//                     match chars.next() {
//                         Some(digit) if digit.is_ascii_digit() => size_str.push(digit),
//                         _ => break,
//                     }
//                 }
//                 let size: usize = size_str.parse().map_err(|_| {
//                     vm.error("bad argument to 'string.unpack' (invalid size)".to_string())
//                 })?;

//                 if idx + size > bytes.len() {
//                     return Err(vm.error("data string too short".to_string()));
//                 }
//                 let s = String::from_utf8_lossy(&bytes[idx..idx + size]);
//                 results.push(vm.create_string(&s));
//                 idx += size;
//             }
//             _ => {
//                 return Err(vm.error(format!(
//                     "bad argument to 'string.unpack' (invalid format option '{}')",
//                     ch
//                 )));
//             }
//         }
//     }

//     // Return unpacked values plus next position
//     results.push(LuaValue::integer((idx + 1) as i64)); // Convert back to 1-based
//     Ok(MultiValue::multiple(results))
// }
