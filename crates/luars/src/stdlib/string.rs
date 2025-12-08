// String library
// Implements: byte, char, dump, find, format, gmatch, gsub, len, lower,
// match, pack, packsize, rep, reverse, sub, unpack, upper

use crate::lib_registry::{LibraryModule, get_arg, get_args, require_arg};
use crate::lua_pattern;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

pub fn create_string_lib() -> LibraryModule {
    crate::lib_module!("string", {
        "byte" => string_byte,
        "char" => string_char,
        "len" => string_len,
        "lower" => string_lower,
        "upper" => string_upper,
        "rep" => string_rep,
        "reverse" => string_reverse,
        "sub" => string_sub,
        "format" => string_format,
        "find" => string_find,
        "match" => string_match,
        "gsub" => string_gsub,
        "gmatch" => string_gmatch,
        "pack" => string_pack,
        "packsize" => string_packsize,
        "unpack" => string_unpack,
    })
}

/// string.byte(s [, i [, j]]) - Return byte values
/// OPTIMIZED: Fast path for single byte return (common case)
fn string_byte(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.byte")?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.byte' (string expected)".to_string()));
    };
    let Some(s) = vm.object_pool.get_string(string_id) else {
        return Err(vm.error("bad argument #1 to 'string.byte' (string expected)".to_string()));
    };

    let str_bytes = s.as_str().as_bytes();
    let len = str_bytes.len() as i64;

    let i = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(1);
    let j = get_arg(vm, 3).and_then(|v| v.as_integer()).unwrap_or(i);

    // Convert negative indices
    let start = if i < 0 { len + i + 1 } else { i };
    let end = if j < 0 { len + j + 1 } else { j };

    if start < 1 || start > len {
        return Ok(MultiValue::empty());
    }

    let end = end.min(len);

    // FAST PATH: Single byte return (most common case)
    if start == end && start >= 1 && start <= len {
        let byte = str_bytes[(start - 1) as usize];
        return Ok(MultiValue::single(LuaValue::integer(byte as i64)));
    }

    // FAST PATH: Two byte return
    if end == start + 1 && start >= 1 && end <= len {
        let b1 = str_bytes[(start - 1) as usize] as i64;
        let b2 = str_bytes[(end - 1) as usize] as i64;
        return Ok(MultiValue::two(
            LuaValue::integer(b1),
            LuaValue::integer(b2),
        ));
    }

    // Slow path: multiple returns
    let mut result = Vec::with_capacity((end - start + 1) as usize);
    for idx in start..=end {
        if idx >= 1 && idx <= len {
            let byte = str_bytes[(idx - 1) as usize];
            result.push(LuaValue::integer(byte as i64));
        }
    }

    Ok(MultiValue::multiple(result))
}

/// string.char(...) - Convert bytes to string
fn string_char(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    let mut bytes = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let Some(byte) = arg.as_integer() else {
            return Err(vm.error(format!(
                "bad argument #{} to 'string.char' (number expected)",
                i + 1
            )));
        };
        if byte < 0 || byte > 255 {
            return Err(vm.error(format!(
                "bad argument #{} to 'string.char' (value out of range)",
                i + 1
            )));
        }

        bytes.push(byte as u8);
    }

    let result_str = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => {
            return Err(vm.error("invalid byte sequence in 'string.char'".to_string()));
        }
    };
    let result = vm.create_string_owned(result_str);
    Ok(MultiValue::single(result))
}

/// string.len(s) - Return string length
/// OPTIMIZED: Use byte length directly, Lua string.len returns byte length not char count
fn string_len(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.len")?;

    let Some(string_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.len' (string expected)".to_string()));
    };
    let Some(s) = vm.object_pool.get_string(string_id) else {
        return Err(vm.error("bad argument #1 to 'string.len' (string expected)".to_string()));
    };

    // Lua string.len returns byte length, not UTF-8 character count
    // This is correct and much faster than chars().count()
    let len = s.as_str().len() as i64;
    Ok(MultiValue::single(LuaValue::integer(len)))
}

/// string.lower(s) - Convert to lowercase
/// OPTIMIZED: ASCII fast path
fn string_lower(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.lower")?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.lower' (string expected)".to_string()));
    };
    let result = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(vm.error("bad argument #1 to 'string.lower' (string expected)".to_string()));
        };
        let str_ref = s.as_str();
        // ASCII fast path: if all bytes are ASCII, use make_ascii_lowercase
        if str_ref.is_ascii() {
            str_ref.to_ascii_lowercase()
        } else {
            str_ref.to_lowercase()
        }
    };
    let result = vm.create_string_owned(result);
    Ok(MultiValue::single(result))
}

/// string.upper(s) - Convert to uppercase
/// OPTIMIZED: ASCII fast path
fn string_upper(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.upper")?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.upper' (string expected)".to_string()));
    };
    let result = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(vm.error("bad argument #1 to 'string.upper' (string expected)".to_string()));
        };
        let str_ref = s.as_str();
        // ASCII fast path
        if str_ref.is_ascii() {
            str_ref.to_ascii_uppercase()
        } else {
            str_ref.to_uppercase()
        }
    };
    let result = vm.create_string_owned(result);
    Ok(MultiValue::single(result))
}

/// string.rep(s, n [, sep]) - Repeat string
fn string_rep(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.rep")?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.rep' (string expected)".to_string()));
    };
    let s_str = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(vm.error("bad argument #1 to 'string.rep' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    let n_value = require_arg(vm, 2, "string.rep")?;
    let Some(n) = n_value.as_integer() else {
        return Err(vm.error("bad argument #2 to 'string.rep' (number expected)".to_string()));
    };

    if n <= 0 {
        let empty = vm.create_string("");
        return Ok(MultiValue::single(empty));
    }

    let sep_value = get_arg(vm, 3);

    let mut result = String::new();
    match sep_value {
        Some(v) => {
            let sep_str = if let Some(sep_id) = v.as_string_id() {
                if let Some(sep) = vm.object_pool.get_string(sep_id) {
                    sep.as_str().to_string()
                } else {
                    return Err(
                        vm.error("bad argument #3 to 'string.rep' (string expected)".to_string())
                    );
                }
            } else {
                return Err(
                    vm.error("bad argument #3 to 'string.rep' (string expected)".to_string())
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
    Ok(MultiValue::single(result))
}

/// string.reverse(s) - Reverse string
fn string_reverse(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.reverse")?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.reverse' (string expected)".to_string()));
    };
    let reversed = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(
                vm.error("bad argument #1 to 'string.reverse' (string expected)".to_string())
            );
        };
        s.as_str().chars().rev().collect::<String>()
    };
    let result = vm.create_string_owned(reversed);
    Ok(MultiValue::single(result))
}

/// string.sub(s, i [, j]) - Extract substring
/// ULTRA-OPTIMIZED: Uses create_substring to avoid allocations when possible
fn string_sub(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.sub")?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.sub' (string expected)".to_string()));
    };

    let i_value = require_arg(vm, 2, "string.sub")?;
    let Some(i) = i_value.as_integer() else {
        return Err(vm.error("bad argument #2 to 'string.sub' (number expected)".to_string()));
    };

    let j = get_arg(vm, 3).and_then(|v| v.as_integer()).unwrap_or(-1);

    // Get string length and compute byte indices
    let (start_byte, end_byte) = {
        let Some(s) = vm.object_pool.get_string(string_id) else {
            return Err(vm.error("bad argument #1 to 'string.sub' (string expected)".to_string()));
        };
        let byte_len = s.as_str().len() as i64;

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
    let result_id = vm
        .object_pool
        .create_substring(string_id, start_byte, end_byte);
    Ok(MultiValue::single(LuaValue::string(result_id)))
}

/// string.format(formatstring, ...) - Format string (simplified)
fn string_format(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let format_str_value = require_arg(vm, 1, "string.format")?;
    let Some(string_id) = format_str_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.format' (string expected)".to_string()));
    };

    // Copy the format string to avoid holding a borrow on vm throughout the loop
    let format = {
        let Some(format_str) = vm.object_pool.get_string(string_id) else {
            return Err(
                vm.error("bad argument #1 to 'string.format' (string expected)".to_string())
            );
        };
        format_str.as_str().to_string()
    };
    let mut result = String::new();
    let mut arg_index = 2; // Start from 2 since arg 1 is the format string
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            if chars.peek().is_some() {
                // Parse format flags and width
                let mut flags = String::new();
                let mut has_format_char = false;

                // Collect format specifier (flags, width, precision)
                while let Some(&c) = chars.peek() {
                    if c == '-'
                        || c == '+'
                        || c == ' '
                        || c == '#'
                        || c == '0'
                        || c.is_numeric()
                        || c == '.'
                    {
                        flags.push(c);
                        chars.next();
                    } else {
                        has_format_char = true;
                        break;
                    }
                }

                if !has_format_char {
                    return Err(vm.error("incomplete format string".to_string()));
                }

                let format_char = chars.next().unwrap();

                match format_char {
                    '%' => {
                        result.push('%');
                    }
                    'c' => {
                        // Character
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val.as_integer().ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (number expected)",
                                arg_index + 1
                            ))
                        })?;
                        if num >= 0 && num <= 255 {
                            result.push(num as u8 as char);
                        } else {
                            return Err(vm.error(format!(
                                "bad argument #{} to 'format' (invalid value for '%%c')",
                                arg_index + 1
                            )));
                        }
                        arg_index += 1;
                    }
                    'd' | 'i' => {
                        // Integer
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{}", num));
                        arg_index += 1;
                    }
                    'o' => {
                        // Octal
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:o}", num));
                        arg_index += 1;
                    }
                    'u' => {
                        // Unsigned integer
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{}", num as u64));
                        arg_index += 1;
                    }
                    'x' => {
                        // Lowercase hexadecimal
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:x}", num));
                        arg_index += 1;
                    }
                    'X' => {
                        // Uppercase hexadecimal
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:X}", num));
                        arg_index += 1;
                    }
                    'e' => {
                        // Scientific notation (lowercase)
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:e}", num));
                        arg_index += 1;
                    }
                    'E' => {
                        // Scientific notation (uppercase)
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:E}", num));
                        arg_index += 1;
                    }
                    'f' => {
                        // Floating point
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;

                        // Parse precision from flags (e.g., ".2")
                        if let Some(dot_pos) = flags.find('.') {
                            let precision_str = &flags[dot_pos + 1..];
                            if let Ok(precision) = precision_str.parse::<usize>() {
                                result.push_str(&format!(
                                    "{:.precision$}",
                                    num,
                                    precision = precision
                                ));
                            } else {
                                result.push_str(&format!("{}", num));
                            }
                        } else {
                            result.push_str(&format!("{}", num));
                        }
                        arg_index += 1;
                    }
                    'g' => {
                        // Automatic format (lowercase) - use shorter of %e or %f
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;

                        // Use scientific notation for very large or very small numbers
                        if num.abs() < 0.0001 || num.abs() >= 1e10 {
                            result.push_str(&format!("{:e}", num));
                        } else {
                            result.push_str(&format!("{}", num));
                        }
                        arg_index += 1;
                    }
                    'G' => {
                        // Automatic format (uppercase) - use shorter of %E or %f
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                vm.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;

                        // Use scientific notation for very large or very small numbers
                        if num.abs() < 0.0001 || num.abs() >= 1e10 {
                            result.push_str(&format!("{:E}", num));
                        } else {
                            result.push_str(&format!("{}", num));
                        }
                        arg_index += 1;
                    }
                    's' => {
                        // String
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;

                        // Try each type in order
                        let s;

                        // Check if string first using ObjectPool
                        let maybe_string = if let Some(str_id) = val.as_string_id() {
                            vm.object_pool
                                .get_string(str_id)
                                .map(|s| s.as_str().to_string())
                        } else {
                            None
                        };

                        if let Some(str_val) = maybe_string {
                            s = str_val;
                        } else if let Some(n) = val.as_integer() {
                            s = n.to_string();
                        } else if let Some(n) = val.as_number() {
                            s = n.to_string();
                        } else {
                            // Call __tostring metamethod
                            s = match vm.call_tostring_metamethod(&val) {
                                Ok(Some(meta_result)) => vm
                                    .get_string(&meta_result)
                                    .map(|st| st.as_str().to_string())
                                    .unwrap_or_else(|| val.type_name().to_string()),
                                Ok(None) => val.type_name().to_string(),
                                Err(e) => return Err(e),
                            };
                        }

                        result.push_str(&s);
                        arg_index += 1;
                    }
                    'q' => {
                        // Quoted string
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            vm.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let Some(str_id) = val.as_string_id() else {
                            return Err(vm.error(format!(
                                "bad argument #{} to 'format' (string expected)",
                                arg_index + 1
                            )));
                        };
                        let Some(s) = vm.object_pool.get_string(str_id) else {
                            return Err(vm.error(format!(
                                "bad argument #{} to 'format' (string expected)",
                                arg_index + 1
                            )));
                        };
                        let s_str = s.as_str().to_string();

                        result.push('"');
                        for ch in s_str.chars() {
                            match ch {
                                '"' => result.push_str("\\\""),
                                '\\' => result.push_str("\\\\"),
                                '\n' => result.push_str("\\n"),
                                '\r' => result.push_str("\\r"),
                                '\t' => result.push_str("\\t"),
                                _ if ch.is_control() => result.push_str(&format!("\\{}", ch as u8)),
                                _ => result.push(ch),
                            }
                        }
                        result.push('"');
                        arg_index += 1;
                    }
                    _ => {
                        return Err(
                            vm.error(format!("invalid option '%{}' to 'format'", format_char))
                        );
                    }
                }
            } else {
                return Err(vm.error("incomplete format string".to_string()));
            }
        } else {
            result.push(ch);
        }
    }

    let result_str = vm.create_string_owned(result);
    Ok(MultiValue::single(result_str))
}

/// string.find(s, pattern [, init [, plain]]) - Find pattern
/// ULTRA-OPTIMIZED: Avoid string cloning in hot path
fn string_find(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.find")?;
    let Some(s_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.find' (string expected)".to_string()));
    };
    let pattern_str_value = require_arg(vm, 2, "string.find")?;
    let Some(pattern_id) = pattern_str_value.as_string_id() else {
        return Err(vm.error("bad argument #2 to 'string.find' (string expected)".to_string()));
    };

    let init = get_arg(vm, 3).and_then(|v| v.as_integer()).unwrap_or(1);
    let plain = get_arg(vm, 4).map(|v| v.is_truthy()).unwrap_or(false);
    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };

    // OPTIMIZATION: Get string references without cloning first
    // Only clone when absolutely necessary (for complex pattern matching)
    let Some(s_lua) = vm.object_pool.get_string(s_id) else {
        return Err(vm.error("bad argument #1 to 'string.find' (string expected)".to_string()));
    };
    let s_str = s_lua.as_str();

    let Some(pattern_lua) = vm.object_pool.get_string(pattern_id) else {
        return Err(vm.error("bad argument #2 to 'string.find' (string expected)".to_string()));
    };
    let pattern = pattern_lua.as_str();

    // Fast path: check if pattern contains special characters
    // If not, use plain search even if plain=false (major optimization)
    let has_special = pattern.bytes().any(|c| {
        matches!(
            c,
            b'%' | b'.' | b'[' | b']' | b'*' | b'+' | b'-' | b'?' | b'^' | b'$' | b'(' | b')'
        )
    });

    if plain || !has_special {
        // Plain string search (no pattern matching) - NO ALLOCATION!
        if start_pos > s_str.len() {
            return Ok(MultiValue::single(LuaValue::nil()));
        }

        if let Some(pos) = s_str[start_pos..].find(pattern) {
            let actual_pos = start_pos + pos;
            let end_pos = actual_pos + pattern.len();
            Ok(MultiValue::two(
                LuaValue::integer((actual_pos + 1) as i64),
                LuaValue::integer(end_pos as i64),
            ))
        } else {
            Ok(MultiValue::single(LuaValue::nil()))
        }
    } else {
        // Complex pattern - need to clone for pattern parser (it takes ownership)
        let pattern_owned = pattern.to_string();
        let s_owned = s_str.to_string();

        // Pattern matching - parse and check if it's a simple literal
        match lua_pattern::parse_pattern(&pattern_owned) {
            Ok(parsed_pattern) => {
                // Fast path: if pattern is just a literal string, use plain search
                if let Some(literal) = parsed_pattern.as_literal_string() {
                    if start_pos > s_owned.len() {
                        return Ok(MultiValue::single(LuaValue::nil()));
                    }

                    if let Some(pos) = s_owned[start_pos..].find(&literal) {
                        let actual_pos = start_pos + pos;
                        let end_pos = actual_pos + literal.len();
                        Ok(MultiValue::two(
                            LuaValue::integer((actual_pos + 1) as i64),
                            LuaValue::integer(end_pos as i64),
                        ))
                    } else {
                        Ok(MultiValue::single(LuaValue::nil()))
                    }
                } else {
                    // Complex pattern - use full pattern matcher
                    if let Some((start, end, captures)) =
                        lua_pattern::find(&s_owned, &parsed_pattern, start_pos)
                    {
                        let mut results = vec![
                            LuaValue::integer((start + 1) as i64),
                            LuaValue::integer(end as i64),
                        ];
                        // Add captures
                        for cap in captures {
                            results.push(vm.create_string(&cap));
                        }
                        Ok(MultiValue::multiple(results))
                    } else {
                        Ok(MultiValue::single(LuaValue::nil()))
                    }
                }
            }
            Err(e) => Err(vm.error(format!("invalid pattern: {}", e))),
        }
    }
}

/// string.match(s, pattern [, init]) - Match pattern
fn string_match(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.match")?;
    let Some(string_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.match' (string expected)".to_string()));
    };
    let Some(s) = vm.object_pool.get_string(string_id) else {
        return Err(vm.error("bad argument #1 to 'string.match' (string expected)".to_string()));
    };
    let s_str = s.as_str().to_string();

    let pattern_str_value = require_arg(vm, 2, "string.match")?;
    let Some(pattern_id) = pattern_str_value.as_string_id() else {
        return Err(vm.error("bad argument #2 to 'string.match' (string expected)".to_string()));
    };
    let Some(pattern_s) = vm.object_pool.get_string(pattern_id) else {
        return Err(vm.error("bad argument #2 to 'string.match' (string expected)".to_string()));
    };
    let pattern_str = pattern_s.as_str().to_string();

    let init = get_arg(vm, 3).and_then(|v| v.as_integer()).unwrap_or(1);

    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };
    let text = s_str[start_pos..].to_string();

    match lua_pattern::parse_pattern(&pattern_str) {
        Ok(pattern) => {
            if let Some((start, end, captures)) = crate::lua_pattern::find(&text, &pattern, 0) {
                if captures.is_empty() {
                    // No captures, return the matched portion
                    let matched = text[start..end].to_string();
                    Ok(MultiValue::single(vm.create_string(&matched)))
                } else {
                    // Return captures
                    let results: Vec<LuaValue> =
                        captures.into_iter().map(|s| vm.create_string(&s)).collect();
                    Ok(MultiValue::multiple(results))
                }
            } else {
                Ok(MultiValue::single(LuaValue::nil()))
            }
        }
        Err(e) => Err(vm.error(format!("invalid pattern: {}", e))),
    }
}

/// string.gsub(s, pattern, repl [, n]) - Global substitution
fn string_gsub(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let arg0 = require_arg(vm, 1, "string.gsub")?;
    let Some(s_id) = arg0.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.gsub' (string expected)".to_string()));
    };
    let s_str = {
        let Some(s) = vm.object_pool.get_string(s_id) else {
            return Err(vm.error("bad argument #1 to 'string.gsub' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    let arg1 = require_arg(vm, 2, "string.gsub")?;
    let Some(pattern_id) = arg1.as_string_id() else {
        return Err(vm.error("bad argument #2 to 'string.gsub' (string expected)".to_string()));
    };
    let pattern_str = {
        let Some(p) = vm.object_pool.get_string(pattern_id) else {
            return Err(vm.error("bad argument #2 to 'string.gsub' (string expected)".to_string()));
        };
        p.as_str().to_string()
    };

    let arg2 = require_arg(vm, 3, "string.gsub")?;

    let max = get_arg(vm, 4)
        .and_then(|v| v.as_integer())
        .map(|n| n as usize);

    let pattern = match lua_pattern::parse_pattern(&pattern_str) {
        Ok(p) => p,
        Err(e) => return Err(vm.error(format!("invalid pattern: {}", e))),
    };

    // Check replacement type: string, function, or table
    if arg2.is_string() {
        // String replacement with capture substitution
        let repl_str = {
            let Some(repl_id) = arg2.as_string_id() else {
                return Err(
                    vm.error("bad argument #3 to 'string.gsub' (string expected)".to_string())
                );
            };
            let Some(repl) = vm.object_pool.get_string(repl_id) else {
                return Err(
                    vm.error("bad argument #3 to 'string.gsub' (string expected)".to_string())
                );
            };
            repl.as_str().to_string()
        };
        match lua_pattern::gsub(&s_str, &pattern, &repl_str, max) {
            Ok((result_str, count)) => {
                let result = vm.create_string(&result_str);
                Ok(MultiValue::multiple(vec![
                    result,
                    LuaValue::integer(count as i64),
                ]))
            }
            Err(e) => Err(vm.error(e)),
        }
    } else if arg2.is_function() || arg2.is_cfunction() {
        // Function replacement
        gsub_with_function(vm, &s_str, &pattern, arg2, max)
    } else if arg2.is_table() {
        // Table replacement (lookup)
        gsub_with_table(vm, &s_str, &pattern, arg2, max)
    } else {
        Err(vm
            .error("bad argument #3 to 'string.gsub' (string/function/table expected)".to_string()))
    }
}

/// Helper for gsub with function replacement
fn gsub_with_function(
    vm: &mut LuaVM,
    text: &str,
    pattern: &crate::lua_pattern::Pattern,
    func: LuaValue,
    max: Option<usize>,
) -> LuaResult<MultiValue> {
    let mut result = String::new();
    let mut count = 0;
    let mut pos = 0;
    let text_chars: Vec<char> = text.chars().collect();

    while pos < text_chars.len() {
        if let Some(max_count) = max {
            if count >= max_count {
                result.extend(&text_chars[pos..]);
                break;
            }
        }

        if let Some((end_pos, captures)) = crate::lua_pattern::try_match(pattern, &text_chars, pos)
        {
            count += 1;

            // Prepare arguments for function call
            let args = if captures.is_empty() {
                // No captures, pass the whole match
                let matched: String = text_chars[pos..end_pos].iter().collect();
                vec![vm.create_string(&matched)]
            } else {
                // Pass captures as arguments
                captures.iter().map(|cap| vm.create_string(cap)).collect()
            };

            // Call the function
            match vm.protected_call(func, args) {
                Ok((success, mut results)) => {
                    if !success {
                        let error_msg = results
                            .first()
                            .and_then(|v| v.as_string_id())
                            .and_then(|id| vm.object_pool.get_string(id))
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_else(|| "unknown error".to_string());
                        return Err(vm.error(error_msg));
                    }

                    // Get the replacement from function result
                    let replacement = results.pop().unwrap_or(LuaValue::nil());

                    if replacement.is_string() {
                        // Use the returned string
                        if let Some(repl_id) = replacement.as_string_id() {
                            if let Some(repl_str) = vm.object_pool.get_string(repl_id) {
                                result.push_str(repl_str.as_str());
                            }
                        }
                    } else if !replacement.is_nil() && !replacement.is_boolean() {
                        // Convert to string (numbers, etc)
                        let repl_str = vm.value_to_string(&replacement)?;
                        result.push_str(&repl_str);
                    } else {
                        // nil or false: keep original
                        result.extend(&text_chars[pos..end_pos]);
                    }
                }
                Err(e) => return Err(e),
            }

            pos = end_pos.max(pos + 1);
        } else {
            result.push(text_chars[pos]);
            pos += 1;
        }
    }

    let result_value = vm.create_string(&result);
    Ok(MultiValue::multiple(vec![
        result_value,
        LuaValue::integer(count as i64),
    ]))
}

/// Helper for gsub with table replacement
fn gsub_with_table(
    vm: &mut LuaVM,
    text: &str,
    pattern: &crate::lua_pattern::Pattern,
    table: LuaValue,
    max: Option<usize>,
) -> LuaResult<MultiValue> {
    let mut result = String::new();
    let mut count = 0;
    let mut pos = 0;
    let text_chars: Vec<char> = text.chars().collect();

    while pos < text_chars.len() {
        if let Some(max_count) = max {
            if count >= max_count {
                result.extend(&text_chars[pos..]);
                break;
            }
        }

        if let Some((end_pos, captures)) = lua_pattern::try_match(pattern, &text_chars, pos) {
            count += 1;

            // Get the key for table lookup
            let key = if captures.is_empty() {
                // No captures, use whole match
                let matched: String = text_chars[pos..end_pos].iter().collect();
                vm.create_string(&matched)
            } else {
                // Use first capture as key
                vm.create_string(&captures[0])
            };

            // Lookup in table
            let replacement = vm.table_get(&table, &key);

            if replacement.is_string() {
                // Use the value from table
                if let Some(repl_id) = replacement.as_string_id() {
                    if let Some(repl_str) = vm.object_pool.get_string(repl_id) {
                        result.push_str(repl_str.as_str());
                    }
                }
            } else if !replacement.is_nil() && !replacement.is_boolean() {
                // Convert to string
                let repl_str = vm.value_to_string(&replacement)?;
                result.push_str(&repl_str);
            } else {
                // nil or false: keep original
                result.extend(&text_chars[pos..end_pos]);
            }

            pos = end_pos.max(pos + 1);
        } else {
            result.push(text_chars[pos]);
            pos += 1;
        }
    }

    let result_value = vm.create_string(&result);
    Ok(MultiValue::multiple(vec![
        result_value,
        LuaValue::integer(count as i64),
    ]))
}

/// string.gmatch(s, pattern) - Returns an iterator function
/// Usage: for capture in string.gmatch(s, pattern) do ... end
fn string_gmatch(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let s_value = require_arg(vm, 1, "string.gmatch")?;
    if !s_value.is_string() {
        return Err(vm.error("bad argument #1 to 'string.gmatch' (string expected)".to_string()));
    };

    let pattern_value = require_arg(vm, 2, "string.gmatch")?;
    if !pattern_value.is_string() {
        return Err(vm.error("bad argument #2 to 'string.gmatch' (string expected)".to_string()));
    };

    // Create state table: {string = s, pattern = p, position = 0}
    let state_table = vm.create_table(3, 0);
    let Some(table_id) = state_table.as_table_id() else {
        return Err(vm.error("failed to create state table for gmatch".to_string()));
    };
    if let Some(state_ref) = vm.object_pool.get_table_mut(table_id) {
        state_ref.set_int(1, s_value);
        state_ref.set_int(2, pattern_value);
        state_ref.set_int(3, LuaValue::integer(0));
    }
    // Return: iterator function, state table, nil (initial control variable)
    Ok(MultiValue::multiple(vec![
        LuaValue::cfunction(gmatch_iterator),
        state_table,
        LuaValue::nil(),
    ]))
}

/// Iterator function for string.gmatch
/// Called as: f(state, control_var)
fn gmatch_iterator(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Arg 0: state table
    // Arg 1: control variable (unused, we use state.position)
    let state_table_value = require_arg(vm, 1, "gmatch iterator")?;

    // Extract string, pattern, and position from state
    let Some(table_id) = state_table_value.as_table_id() else {
        return Err(vm.error("gmatch iterator: state is not a table".to_string()));
    };

    // Extract all values from table first
    let (s_str, pattern_str_owned, position) = {
        let Some(state_ref) = vm.object_pool.get_table(table_id) else {
            return Err(vm.error("gmatch iterator: state is not a table".to_string()));
        };

        let Some(s_val) = state_ref.get_int(1) else {
            return Err(vm.error("gmatch iterator: string not found in state".to_string()));
        };
        let Some(s_id) = s_val.as_string_id() else {
            return Err(vm.error("gmatch iterator: string invalid".to_string()));
        };

        let Some(pattern_val) = state_ref.get_int(2) else {
            return Err(vm.error("gmatch iterator: pattern not found in state".to_string()));
        };
        let Some(pattern_id) = pattern_val.as_string_id() else {
            return Err(vm.error("gmatch iterator: pattern invalid".to_string()));
        };

        let position_value = state_ref.get_int(3).unwrap_or(LuaValue::integer(0));
        let position = position_value.as_integer().unwrap_or(0) as usize;

        // Get string contents
        let Some(s_obj) = vm.object_pool.get_string(s_id) else {
            return Err(vm.error("gmatch iterator: string invalid".to_string()));
        };
        let Some(pattern_obj) = vm.object_pool.get_string(pattern_id) else {
            return Err(vm.error("gmatch iterator: pattern invalid".to_string()));
        };

        (
            s_obj.as_str().to_string(),
            pattern_obj.as_str().to_string(),
            position,
        )
    };

    // Parse pattern
    let pattern = match lua_pattern::parse_pattern(&pattern_str_owned) {
        Ok(p) => p,
        Err(e) => return Err(vm.error(format!("invalid pattern: {}", e))),
    };

    // Find next match
    if let Some((start, end, captures)) = lua_pattern::find(&s_str, &pattern, position) {
        // Update position for next iteration
        let next_pos = if end > start { end } else { end + 1 };
        if let Some(state_ref) = vm.object_pool.get_table_mut(table_id) {
            state_ref.set_int(3, LuaValue::integer(next_pos as i64));
        }

        // Return captures if any, otherwise return the matched string
        if captures.is_empty() {
            let matched = &s_str[start..end];
            Ok(MultiValue::single(vm.create_string(matched)))
        } else {
            let mut results = Vec::new();
            for cap in captures {
                results.push(vm.create_string(&cap));
            }
            Ok(MultiValue::multiple(results))
        }
    } else {
        // No more matches
        Ok(MultiValue::single(LuaValue::nil()))
    }
}

/// string.pack(fmt, v1, v2, ...) - Pack values into binary string
/// Simplified implementation supporting basic format codes
fn string_pack(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let fmt_value = require_arg(vm, 1, "string.pack")?;
    let Some(fmt_id) = fmt_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.pack' (string expected)".to_string()));
    };
    let fmt_str = {
        let Some(fmt) = vm.object_pool.get_string(fmt_id) else {
            return Err(vm.error("bad argument #1 to 'string.pack' (string expected)".to_string()));
        };
        fmt.as_str().to_string()
    };

    let args = get_args(vm);
    let values = &args[1..]; // Skip format string

    let mut result = Vec::new();
    let mut value_idx = 0;
    let mut chars = fmt_str.chars();

    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue, // Skip whitespace
            'b' => {
                // signed byte
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let n = values[value_idx].as_integer().ok_or_else(|| {
                    vm.error("bad argument to 'string.pack' (number expected)".to_string())
                })?;
                result.push((n & 0xFF) as u8);
                value_idx += 1;
            }
            'B' => {
                // unsigned byte
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let n = values[value_idx].as_integer().ok_or_else(|| {
                    vm.error("bad argument to 'string.pack' (number expected)".to_string())
                })?;
                result.push((n & 0xFF) as u8);
                value_idx += 1;
            }
            'h' => {
                // signed short (2 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let n = values[value_idx].as_integer().ok_or_else(|| {
                    vm.error("bad argument to 'string.pack' (number expected)".to_string())
                })? as i16;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'H' => {
                // unsigned short (2 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let n = values[value_idx].as_integer().ok_or_else(|| {
                    vm.error("bad argument to 'string.pack' (number expected)".to_string())
                })? as u16;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'i' | 'l' => {
                // signed int (4 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let n = values[value_idx].as_integer().ok_or_else(|| {
                    vm.error("bad argument to 'string.pack' (number expected)".to_string())
                })? as i32;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'I' | 'L' => {
                // unsigned int (4 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let n = values[value_idx].as_integer().ok_or_else(|| {
                    vm.error("bad argument to 'string.pack' (number expected)".to_string())
                })? as u32;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'f' => {
                // float (4 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let n = values[value_idx].as_number().ok_or_else(|| {
                    vm.error("bad argument to 'string.pack' (number expected)".to_string())
                })? as f32;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'd' => {
                // double (8 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let n = values[value_idx].as_number().ok_or_else(|| {
                    vm.error("bad argument to 'string.pack' (number expected)".to_string())
                })?;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'z' => {
                // zero-terminated string
                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let s_str = {
                    let Some(s_id) = values[value_idx].as_string_id() else {
                        return Err(
                            vm.error("bad argument to 'string.pack' (string expected)".to_string())
                        );
                    };
                    let Some(s) = vm.object_pool.get_string(s_id) else {
                        return Err(
                            vm.error("bad argument to 'string.pack' (string expected)".to_string())
                        );
                    };
                    s.as_str().to_string()
                };
                result.extend_from_slice(s_str.as_bytes());
                result.push(0); // null terminator
                value_idx += 1;
            }
            'c' => {
                // fixed-length string - need to read size
                let mut size_str = String::new();
                loop {
                    match chars.next() {
                        Some(digit) if digit.is_ascii_digit() => size_str.push(digit),
                        _ => break,
                    }
                }
                let size: usize = size_str.parse().map_err(|_| {
                    vm.error("bad argument to 'string.pack' (invalid size)".to_string())
                })?;

                if value_idx >= values.len() {
                    return Err(
                        vm.error("bad argument to 'string.pack' (not enough values)".to_string())
                    );
                }
                let s_str = {
                    let Some(s_id) = values[value_idx].as_string_id() else {
                        return Err(
                            vm.error("bad argument to 'string.pack' (string expected)".to_string())
                        );
                    };
                    let Some(s) = vm.object_pool.get_string(s_id) else {
                        return Err(
                            vm.error("bad argument to 'string.pack' (string expected)".to_string())
                        );
                    };
                    s.as_str().to_string()
                };
                let bytes = s_str.as_bytes();
                result.extend_from_slice(&bytes[..size.min(bytes.len())]);
                // Pad with zeros if needed
                for _ in bytes.len()..size {
                    result.push(0);
                }
                value_idx += 1;
            }
            _ => {
                return Err(vm.error(format!(
                    "bad argument to 'string.pack' (invalid format option '{}')",
                    ch
                )));
            }
        }
    }

    // Create a string directly from bytes without UTF-8 validation
    // Lua strings can contain arbitrary binary data
    let packed = unsafe { String::from_utf8_unchecked(result) };
    Ok(MultiValue::single(vm.create_string(&packed)))
}

/// string.packsize(fmt) - Return size of packed data
fn string_packsize(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let fmt_value = require_arg(vm, 1, "string.packsize")?;
    let Some(fmt_id) = fmt_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.packsize' (string expected)".to_string()));
    };
    let fmt_str = {
        let Some(fmt) = vm.object_pool.get_string(fmt_id) else {
            return Err(
                vm.error("bad argument #1 to 'string.packsize' (string expected)".to_string())
            );
        };
        fmt.as_str().to_string()
    };

    let mut size = 0usize;
    let mut chars = fmt_str.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue,
            'b' | 'B' => size += 1,
            'h' | 'H' => size += 2,
            'l' | 'L' | 'f' => size += 4,
            // 'i' and 'I' can have optional size specifier
            'i' | 'I' => {
                // Check for size specifier
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                let n: usize = if size_str.is_empty() {
                    4 // default size
                } else {
                    size_str.parse().unwrap_or(4)
                };
                size += n;
            }
            'd' | 'n' => size += 8,  // 'n' is lua_Number (double)
            'j' | 'J' | 'T' => size += std::mem::size_of::<i64>(),  // lua_Integer / size_t
            's' => {
                // Check for size specifier
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                let n: usize = if size_str.is_empty() {
                    std::mem::size_of::<usize>() // default size_t
                } else {
                    size_str.parse().unwrap_or(std::mem::size_of::<usize>())
                };
                return Err(vm.error("variable-length format 's' in 'string.packsize'".to_string()));
            }
            'z' => {
                return Err(vm.error("variable-length format in 'string.packsize'".to_string()));
            }
            'c' => {
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                let n: usize = size_str.parse().map_err(|_| {
                    vm.error("bad argument to 'string.packsize' (invalid size)".to_string())
                })?;
                size += n;
            }
            'x' => size += 1, // padding byte
            'X' => {} // empty alignment
            '<' | '>' | '=' | '!' => {} // endianness/alignment modifiers
            _ => {
                return Err(vm.error(format!(
                    "bad argument to 'string.packsize' (invalid format option '{}')",
                    ch
                )));
            }
        }
    }

    Ok(MultiValue::single(LuaValue::integer(size as i64)))
}

/// string.unpack(fmt, s [, pos]) - Unpack binary string
fn string_unpack(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let fmt_value = require_arg(vm, 1, "string.unpack")?;
    let Some(fmt_id) = fmt_value.as_string_id() else {
        return Err(vm.error("bad argument #1 to 'string.unpack' (string expected)".to_string()));
    };
    let fmt_str = {
        let Some(fmt) = vm.object_pool.get_string(fmt_id) else {
            return Err(
                vm.error("bad argument #1 to 'string.unpack' (string expected)".to_string())
            );
        };
        fmt.as_str().to_string()
    };

    let s_value = require_arg(vm, 2, "string.unpack")?;
    let Some(s_id) = s_value.as_string_id() else {
        return Err(vm.error("bad argument #2 to 'string.unpack' (string expected)".to_string()));
    };
    let s_str = {
        let Some(s) = vm.object_pool.get_string(s_id) else {
            return Err(
                vm.error("bad argument #2 to 'string.unpack' (string expected)".to_string())
            );
        };
        s.as_str().to_string()
    };
    let bytes = s_str.as_bytes();

    let pos = get_arg(vm, 3).and_then(|v| v.as_integer()).unwrap_or(1) as usize - 1; // Convert to 0-based

    let mut results = Vec::new();
    let mut idx = pos;
    let mut chars = fmt_str.chars();

    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue,
            'b' => {
                if idx >= bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                results.push(LuaValue::integer(bytes[idx] as i8 as i64));
                idx += 1;
            }
            'B' => {
                if idx >= bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                results.push(LuaValue::integer(bytes[idx] as i64));
                idx += 1;
            }
            'h' => {
                if idx + 2 > bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                let val = i16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
                results.push(LuaValue::integer(val as i64));
                idx += 2;
            }
            'H' => {
                if idx + 2 > bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                let val = u16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
                results.push(LuaValue::integer(val as i64));
                idx += 2;
            }
            'i' | 'l' => {
                if idx + 4 > bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                let val = i32::from_le_bytes([
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                ]);
                results.push(LuaValue::integer(val as i64));
                idx += 4;
            }
            'I' | 'L' => {
                if idx + 4 > bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                let val = u32::from_le_bytes([
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                ]);
                results.push(LuaValue::integer(val as i64));
                idx += 4;
            }
            'f' => {
                if idx + 4 > bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                let val = f32::from_le_bytes([
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                ]);
                results.push(LuaValue::float(val as f64));
                idx += 4;
            }
            'd' => {
                if idx + 8 > bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                let mut arr = [0u8; 8];
                arr.copy_from_slice(&bytes[idx..idx + 8]);
                let val = f64::from_le_bytes(arr);
                results.push(LuaValue::float(val));
                idx += 8;
            }
            'z' => {
                // Read null-terminated string
                let start = idx;
                while idx < bytes.len() && bytes[idx] != 0 {
                    idx += 1;
                }
                let s = String::from_utf8_lossy(&bytes[start..idx]);
                results.push(vm.create_string(&s));
                idx += 1; // Skip null terminator
            }
            'c' => {
                let mut size_str = String::new();
                loop {
                    match chars.next() {
                        Some(digit) if digit.is_ascii_digit() => size_str.push(digit),
                        _ => break,
                    }
                }
                let size: usize = size_str.parse().map_err(|_| {
                    vm.error("bad argument to 'string.unpack' (invalid size)".to_string())
                })?;

                if idx + size > bytes.len() {
                    return Err(vm.error("data string too short".to_string()));
                }
                let s = String::from_utf8_lossy(&bytes[idx..idx + size]);
                results.push(vm.create_string(&s));
                idx += size;
            }
            _ => {
                return Err(vm.error(format!(
                    "bad argument to 'string.unpack' (invalid format option '{}')",
                    ch
                )));
            }
        }
    }

    // Return unpacked values plus next position
    results.push(LuaValue::integer((idx + 1) as i64)); // Convert back to 1-based
    Ok(MultiValue::multiple(results))
}
