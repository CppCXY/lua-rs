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

    // Try to create a valid UTF-8 string first, otherwise return binary
    let result = match String::from_utf8(bytes.clone()) {
        Ok(s) => l.vm_mut().create_string(&s),
        Err(_) => l.vm_mut().create_binary(bytes),
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
    let Some(chunk) = func_obj.chunk() else {
        return Err(l.error("unable to dump given function".to_string()));
    };

    // Serialize the chunk with pool access for string constants
    match chunk_serializer::serialize_chunk_with_pool(&chunk, strip, &vm.object_pool) {
        Ok(bytes) => {
            // Create binary value directly - no encoding needed
            let result = vm.create_binary(bytes);
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
/// OPTIMIZED: ASCII fast path
fn string_lower(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'string.lower' (string expected)".to_string())
    })?;
    let Some(s) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'string.lower' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    let result = {
        // ASCII fast path: if all bytes are ASCII, use make_ascii_lowercase
        if s.is_ascii() {
            s.to_ascii_lowercase()
        } else {
            s.to_lowercase()
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
    let Some(s) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'string.upper' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    let result = {
        // ASCII fast path
        if s.is_ascii() {
            s.to_ascii_uppercase()
        } else {
            s.to_uppercase()
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
    let Some(s_str) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'string.rep' (string expected)".to_string()));
    };
    let s_str = s_str.to_string();

    // Get parameters
    let n_value = l.get_arg(2);
    let sep_value = l.get_arg(3);

    let vm = l.vm_mut();

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
            let Some(sep_str) = v.as_str() else {
                return Err(
                    l.error("bad argument #3 to 'string.rep' (string expected)".to_string())
                );
            };
            let sep_str = sep_str.to_string();

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
    let Some(s) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'string.reverse' (string expected)".to_string()));
    };

    let vm = l.vm_mut();
    let reversed = s.chars().rev().collect::<String>();
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
    let Some(s) = s_value.as_str() else {
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
    let Some(s_str) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'find' (string expected)".to_string()));
    };
    let s_str = s_str.to_string();

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'find' (string expected)".to_string()))?;
    let Some(pattern) = pattern_value.as_str() else {
        return Err(l.error("bad argument #2 to 'find' (string expected)".to_string()));
    };
    let pattern = pattern.to_string();

    let init = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let plain = l.get_arg(4).map(|v| v.is_truthy()).unwrap_or(false);
    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };

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
    let Some(s_str) = s_value.as_str() else {
        return Err(l.error("bad argument #1 to 'gsub' (string expected)".to_string()));
    };
    let s_str = s_str.to_string();

    let pattern_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'gsub' (string expected)".to_string()))?;
    let Some(pattern_str) = pattern_value.as_str() else {
        return Err(l.error("bad argument #2 to 'gsub' (string expected)".to_string()));
    };
    let pattern_str = pattern_str.to_string();

    let repl_value = l
        .get_arg(3)
        .ok_or_else(|| l.error("bad argument #3 to 'gsub' (value expected)".to_string()))?;

    let max = l
        .get_arg(4)
        .and_then(|v| v.as_integer())
        .map(|n| n as usize);

    let pattern = match pattern::parse_pattern(&pattern_str) {
        Ok(p) => p,
        Err(e) => return Err(l.error(format!("invalid pattern: {}", e))),
    };

    // String replacement
    if let Some(repl_str) = repl_value.as_str() {
        let repl_str = repl_str.to_string();
        match pattern::gsub(&s_str, &pattern, &repl_str, max) {
            Ok((result_str, count)) => {
                let result = l.create_string(&result_str);
                l.push_value(result)?;
                l.push_value(LuaValue::integer(count as i64))?;
                Ok(2)
            }
            Err(e) => Err(l.error(e)),
        }
    } else if repl_value.is_function() {
        // Function replacement - currently not fully implemented
        // TODO: Need proper protected call support for Lua functions in gsub
        return Err(l.error("gsub with function replacement not yet fully implemented".to_string()));
    } else if repl_value.is_table() {
        // Table replacement
        let matches = pattern::find_all_matches(&s_str, &pattern, max);
        let mut result = String::new();
        let mut last_end = 0;
        let mut count = 0;

        for m in &matches {
            // Copy text before match
            result.push_str(&s_str[last_end..m.start]);

            // Table lookup
            let key = if m.captures.is_empty() {
                // No captures, use whole match as key
                l.create_string(&s_str[m.start..m.end])
            } else {
                // Use first capture as key
                l.create_string(&m.captures[0])
            };

            let result_val = l.table_get(&repl_value, &key).unwrap_or(LuaValue::nil());

            let replacement = if result_val.is_nil() {
                // nil means no replacement, use original match
                s_str[m.start..m.end].to_string()
            } else if let Some(s) = result_val.as_str() {
                s.to_string()
            } else if let Some(n) = result_val.as_integer() {
                n.to_string()
            } else if let Some(n) = result_val.as_number() {
                n.to_string()
            } else {
                // Use original match for non-string/number results
                s_str[m.start..m.end].to_string()
            };

            result.push_str(&replacement);
            last_end = m.end;
            count += 1;
        }

        // Copy remaining text
        result.push_str(&s_str[last_end..]);

        let result_val = l.create_string(&result);
        l.push_value(result_val)?;
        l.push_value(LuaValue::integer(count as i64))?;
        Ok(2)
    } else {
        Err(l.error("bad argument #3 to 'gsub' (string/function/table expected)".to_string()))
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

    if let Some(state_ref) = state_table.as_table_mut() {
        state_ref.set_int(1, s_value);
        state_ref.set_int(2, pattern_value);
        state_ref.set_int(3, LuaValue::integer(0)); // position
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

    let Some(state_ref) = state_table_value.as_table() else {
        return Err(l.error("gmatch iterator: state is not a table".to_string()));
    };

    // Extract string, pattern, and position from state
    let Some(s_val) = state_ref.get_int(1) else {
        return Err(l.error("gmatch iterator: string not found in state".to_string()));
    };
    let Some(s_str) = s_val.as_str() else {
        return Err(l.error("gmatch iterator: string invalid".to_string()));
    };

    let Some(pattern_val) = state_ref.get_int(2) else {
        return Err(l.error("gmatch iterator: pattern not found in state".to_string()));
    };
    let Some(pattern_str) = pattern_val.as_str() else {
        return Err(l.error("gmatch iterator: pattern invalid".to_string()));
    };

    let position_value = state_ref.get_int(3).unwrap_or(LuaValue::integer(0));
    let position = position_value.as_integer().unwrap_or(0) as usize;

    // Parse pattern
    let pattern = match pattern::parse_pattern(&pattern_str) {
        Ok(p) => p,
        Err(e) => return Err(l.error(format!("invalid pattern: {}", e))),
    };

    // Find next match
    if let Some((start, end, captures)) = pattern::find(&s_str, &pattern, position) {
        // Update position for next iteration
        let next_pos = if end > start { end } else { end + 1 };
        if let Some(state_ref) = state_table_value.as_table_mut() {
            state_ref.set_int(3, LuaValue::integer(next_pos as i64));
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
