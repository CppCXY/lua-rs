// String library
// Implements: byte, char, dump, find, format, gmatch, gsub, len, lower,
// match, pack, packsize, rep, reverse, sub, unpack, upper

use crate::lib_registry::{LibraryModule, get_arg, require_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::vm::VM;
use crate::{LuaTable, lua_pattern};

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
        // "gmatch" => string_gmatch,
    })
}

/// string.byte(s [, i [, j]]) - Return byte values
fn string_byte(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.byte")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.byte' (string expected)".to_string())?;

    let str_bytes = s.as_str().as_bytes();
    let len = str_bytes.len() as i64;

    let i = get_arg(vm, 1).and_then(|v| v.as_integer()).unwrap_or(1);

    let j = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(i);

    // Convert negative indices
    let start = if i < 0 { len + i + 1 } else { i };
    let end = if j < 0 { len + j + 1 } else { j };

    if start < 1 || start > len {
        return Ok(MultiValue::empty());
    }

    let mut result = Vec::new();
    for idx in start..=end.min(len) {
        if idx >= 1 && idx <= len {
            let byte = str_bytes[(idx - 1) as usize];
            result.push(LuaValue::integer(byte as i64));
        }
    }

    Ok(MultiValue::multiple(result))
}

/// string.char(...) - Convert bytes to string
fn string_char(vm: &mut VM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);

    let mut bytes = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let byte = arg
            .as_integer()
            .ok_or_else(|| format!("bad argument #{} to 'string.char' (number expected)", i + 1))?;

        if byte < 0 || byte > 255 {
            return Err(format!(
                "bad argument #{} to 'string.char' (value out of range)",
                i + 1
            ));
        }

        bytes.push(byte as u8);
    }

    let result_str =
        String::from_utf8(bytes).map_err(|_| "string.char: invalid UTF-8".to_string())?;

    let result = vm.create_string(result_str);
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.len(s) - Return string length
fn string_len(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.len")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.len' (string expected)".to_string())?;

    let len = s.as_str().len() as i64;
    Ok(MultiValue::single(LuaValue::integer(len)))
}

/// string.lower(s) - Convert to lowercase
fn string_lower(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.lower")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.lower' (string expected)".to_string())?;

    let result = vm.create_string(s.as_str().to_lowercase());
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.upper(s) - Convert to uppercase
fn string_upper(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.upper")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.upper' (string expected)".to_string())?;

    let result = vm.create_string(s.as_str().to_uppercase());
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.rep(s, n [, sep]) - Repeat string
fn string_rep(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.rep")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.rep' (string expected)".to_string())?;

    let n = require_arg(vm, 1, "string.rep")?
        .as_integer()
        .ok_or_else(|| "bad argument #2 to 'string.rep' (number expected)".to_string())?;

    let sep = get_arg(vm, 2)
        .and_then(|v| v.as_string())
        .map(|s| s.as_str().to_string())
        .unwrap_or_default();

    if n <= 0 {
        let empty = vm.create_string(String::new());
        return Ok(MultiValue::single(LuaValue::from_string_rc(empty)));
    }

    let mut result = String::new();
    for i in 0..n {
        if i > 0 && !sep.is_empty() {
            result.push_str(&sep);
        }
        result.push_str(s.as_str());
    }

    let result = vm.create_string(result);
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.reverse(s) - Reverse string
fn string_reverse(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.reverse")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.reverse' (string expected)".to_string())?;

    let reversed: String = s.as_str().chars().rev().collect();
    let result = vm.create_string(reversed);
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.sub(s, i [, j]) - Extract substring
fn string_sub(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.sub")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.sub' (string expected)".to_string())?;

    let len = s.as_str().len() as i64;

    let i = require_arg(vm, 1, "string.sub")?
        .as_integer()
        .ok_or_else(|| "bad argument #2 to 'string.sub' (number expected)".to_string())?;

    let j = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(-1);

    // Convert negative indices
    let start = if i < 0 { len + i + 1 } else { i };
    let end = if j < 0 { len + j + 1 } else { j };

    let start = start.max(1).min(len + 1) as usize;
    let end = end.max(0).min(len) as usize;

    let result_str = if start <= end {
        s.as_str()
            .chars()
            .skip(start - 1)
            .take(end - start + 1)
            .collect::<String>()
    } else {
        String::new()
    };

    let result = vm.create_string(result_str);
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.format(formatstring, ...) - Format string (simplified)
fn string_format(vm: &mut VM) -> Result<MultiValue, String> {
    let format_str = require_arg(vm, 0, "string.format")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.format' (string expected)".to_string())?;

    let format = format_str.as_str();
    let mut result = String::new();
    let mut arg_index = 1;
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
                    return Err("incomplete format string".to_string());
                }

                let format_char = chars.next().unwrap();

                match format_char {
                    '%' => {
                        result.push('%');
                    }
                    'c' => {
                        // Character
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val.as_integer().ok_or_else(|| {
                            format!(
                                "bad argument #{} to 'format' (number expected)",
                                arg_index + 1
                            )
                        })?;
                        if num >= 0 && num <= 255 {
                            result.push(num as u8 as char);
                        } else {
                            return Err(format!(
                                "bad argument #{} to 'format' (invalid value for '%%c')",
                                arg_index + 1
                            ));
                        }
                        arg_index += 1;
                    }
                    'd' | 'i' => {
                        // Integer
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
                            })?;
                        result.push_str(&format!("{}", num));
                        arg_index += 1;
                    }
                    'o' => {
                        // Octal
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
                            })?;
                        result.push_str(&format!("{:o}", num));
                        arg_index += 1;
                    }
                    'u' => {
                        // Unsigned integer
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
                            })?;
                        result.push_str(&format!("{}", num as u64));
                        arg_index += 1;
                    }
                    'x' => {
                        // Lowercase hexadecimal
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
                            })?;
                        result.push_str(&format!("{:x}", num));
                        arg_index += 1;
                    }
                    'X' => {
                        // Uppercase hexadecimal
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
                            })?;
                        result.push_str(&format!("{:X}", num));
                        arg_index += 1;
                    }
                    'e' => {
                        // Scientific notation (lowercase)
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
                            })?;
                        result.push_str(&format!("{:e}", num));
                        arg_index += 1;
                    }
                    'E' => {
                        // Scientific notation (uppercase)
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
                            })?;
                        result.push_str(&format!("{:E}", num));
                        arg_index += 1;
                    }
                    'f' => {
                        // Floating point
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
                            })?;

                        // Parse precision from flags (e.g., ".2")
                        if let Some(dot_pos) = flags.find('.') {
                            let precision_str = &flags[dot_pos + 1..];
                            if let Ok(precision) = precision_str.parse::<usize>() {
                                result.push_str(&format!("{:.prec$}", num, prec = precision));
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
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
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
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                )
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
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;

                        let s = if let Some(s) = val.as_string() {
                            s.as_str().to_string()
                        } else if let Some(n) = val.as_integer() {
                            n.to_string()
                        } else if let Some(n) = val.as_number() {
                            n.to_string()
                        } else {
                            // Try __tostring metamethod
                            match vm.call_tostring_metamethod(&val) {
                                Ok(Some(s)) => s.as_str().to_string(),
                                Ok(None) => val.to_string_repr(),
                                Err(e) => return Err(e),
                            }
                        };
                        result.push_str(&s);
                        arg_index += 1;
                    }
                    'q' => {
                        // Quoted string
                        let val = get_arg(vm, arg_index).ok_or_else(|| {
                            format!("bad argument #{} to 'format' (no value)", arg_index + 1)
                        })?;
                        let s = val.as_string().ok_or_else(|| {
                            format!(
                                "bad argument #{} to 'format' (string expected)",
                                arg_index + 1
                            )
                        })?;

                        result.push('"');
                        for ch in s.as_str().chars() {
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
                        return Err(format!("invalid option '%{}' to 'format'", format_char));
                    }
                }
            } else {
                return Err("incomplete format string".to_string());
            }
        } else {
            result.push(ch);
        }
    }

    let result_str = vm.create_string(result);
    Ok(MultiValue::single(LuaValue::from_string_rc(result_str)))
}

/// string.find(s, pattern [, init [, plain]]) - Find pattern
fn string_find(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.find")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.find' (string expected)".to_string())?;

    let pattern_str = require_arg(vm, 1, "string.find")?
        .as_string()
        .ok_or_else(|| "bad argument #2 to 'string.find' (string expected)".to_string())?;

    let init = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(1);

    let plain = get_arg(vm, 3).map(|v| v.is_truthy()).unwrap_or(false);

    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };

    if plain {
        // Plain string search (no pattern matching)
        if let Some(pos) = s.as_str()[start_pos..].find(pattern_str.as_str()) {
            let actual_pos = start_pos + pos;
            Ok(MultiValue::multiple(vec![
                LuaValue::integer((actual_pos + 1) as i64),
                LuaValue::integer((actual_pos + pattern_str.as_str().len()) as i64),
            ]))
        } else {
            Ok(MultiValue::single(LuaValue::nil()))
        }
    } else {
        // Pattern matching
        match crate::lua_pattern::parse_pattern(pattern_str.as_str()) {
            Ok(pattern) => {
                if let Some((start, end, captures)) =
                    crate::lua_pattern::find(s.as_str(), &pattern, start_pos)
                {
                    let mut results = vec![
                        LuaValue::integer((start + 1) as i64),
                        LuaValue::integer(end as i64),
                    ];
                    // Add captures
                    for cap in captures {
                        results.push(LuaValue::from_string_rc(vm.create_string(cap)));
                    }
                    Ok(MultiValue::multiple(results))
                } else {
                    Ok(MultiValue::single(LuaValue::nil()))
                }
            }
            Err(e) => Err(format!("invalid pattern: {}", e)),
        }
    }
}

/// string.match(s, pattern [, init]) - Match pattern
fn string_match(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.match")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.match' (string expected)".to_string())?;

    let pattern_str = require_arg(vm, 1, "string.match")?
        .as_string()
        .ok_or_else(|| "bad argument #2 to 'string.match' (string expected)".to_string())?;

    let init = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(1);

    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };
    let text = &s.as_str()[start_pos..];

    match lua_pattern::parse_pattern(pattern_str.as_str()) {
        Ok(pattern) => {
            if let Some((start, end, captures)) = crate::lua_pattern::find(text, &pattern, 0) {
                if captures.is_empty() {
                    // No captures, return the matched portion
                    let matched = &text[start..end];
                    Ok(MultiValue::single(LuaValue::from_string_rc(
                        vm.create_string(matched.to_string()),
                    )))
                } else {
                    // Return captures
                    let results: Vec<LuaValue> = captures
                        .into_iter()
                        .map(|s| LuaValue::from_string_rc(vm.create_string(s)))
                        .collect();
                    Ok(MultiValue::multiple(results))
                }
            } else {
                Ok(MultiValue::single(LuaValue::nil()))
            }
        }
        Err(e) => Err(format!("invalid pattern: {}", e)),
    }
}

/// string.gsub(s, pattern, repl [, n]) - Global substitution
fn string_gsub(_vm: &mut VM) -> Result<MultiValue, String> {
    //     let s = require_arg(vm, 0, "string.gsub")?
    //         .as_string()
    //         .ok_or_else(|| "bad argument #1 to 'string.gsub' (string expected)".to_string())?;

    //     let pattern_str = require_arg(vm, 1, "string.gsub")?
    //         .as_string()
    //         .ok_or_else(|| "bad argument #2 to 'string.gsub' (string expected)".to_string())?;

    //     let repl = require_arg(vm, 2, "string.gsub")?
    //         .as_string()
    //         .ok_or_else(|| "bad argument #3 to 'string.gsub' (string expected)".to_string())?;

    //     let max = get_arg(vm, 3)
    //         .and_then(|v| v.as_integer())
    //         .map(|n| n as usize);

    //     match crate::lua_pattern::parse_pattern(pattern_str.as_str()) {
    //         Ok(pattern) => {
    //             let (result_str, count) =
    //                 crate::lua_pattern::gsub(s.as_str(), &pattern, repl.as_str(), max);

    //             let result = vm.create_string(result_str);
    //             Ok(MultiValue::multiple(vec![
    //                 LuaValue::from_string_rc(result),
    //                 LuaValue::integer(count as i64),
    //             ]))
    //         }
    //         Err(e) => Err(format!("invalid pattern: {}", e)),
    //     }
    // }

    // /// need improve
    // /// string.gmatch(s, pattern) - Iterator for pattern matches
    // fn string_gmatch(vm: &mut VM) -> Result<MultiValue, String> {
    //     let s = require_arg(vm, 0, "string.gmatch")?
    //         .as_string()
    //         .ok_or_else(|| "bad argument #1 to 'string.gmatch' (string expected)".to_string())?;

    //     let pattern_str = require_arg(vm, 1, "string.gmatch")?
    //         .as_string()
    //         .ok_or_else(|| "bad argument #2 to 'string.gmatch' (string expected)".to_string())?;

    //     // Parse and validate pattern
    //     let _pattern = match crate::lua_pattern::parse_pattern(pattern_str.as_str()) {
    //         Ok(p) => p,
    //         Err(e) => return Err(format!("invalid pattern: {}", e)),
    //     };

    //     // Create a state structure to store in userdata
    //     #[allow(dead_code)]
    //     #[derive(Clone)]
    //     struct GmatchState {
    //         string: String,
    //         pattern: String,
    //         position: usize,
    //     }

    //     let state = GmatchState {
    //         string: s.as_str().to_string(),
    //         pattern: pattern_str.as_str().to_string(),
    //         position: 0,
    //     };

    //     // Store state directly in userdata using Box for stable pointer
    //     use std::cell::RefCell;
    //     use std::rc::Rc;
    //     let state_box = Box::new(RefCell::new(state));
    //     let state_ptr = Box::into_raw(state_box) as usize;

    //     let userdata = LuaValue::from_userdata_rc(state_box);

    //     // Create metatable with __call and __gc
    //     if let Some(ref ud) = userdata.as_userdata() {
    //         let mt = Rc::new(RefCell::new(LuaTable::new()));
    //         mt.borrow_mut().raw_set(
    //             LuaValue::from_string_rc(vm.create_string("__call".to_string())),
    //             LuaValue::cfunction(gmatch_iterator_optimized),
    //         );
    //         mt.borrow_mut().raw_set(
    //             LuaValue::from_string_rc(vm.create_string("__gc".to_string())),
    //             LuaValue::cfunction(gmatch_gc_optimized),
    //         );

    //         ud.set_metatable(Some(mt));
    //     }

    //     Ok(MultiValue::single(userdata))
    todo!()
}

/// Optimized iterator that stores state directly in userdata
fn gmatch_iterator_optimized(vm: &mut VM) -> Result<MultiValue, String> {
    #[allow(dead_code)]
    #[derive(Clone)]
    struct GmatchState {
        string: String,
        pattern: String,
        position: usize,
    }

    // For __call metamethod, register 1 is self (the userdata)
    let frame = vm.frames.last().unwrap();
    if frame.registers.len() < 2 {
        return Err("gmatch iterator: insufficient arguments".to_string());
    }

    let state_val = &frame.registers[1];

    // Extract state pointer from userdata
    let state_ptr = if let Some(ud) = state_val.as_userdata() {
        let data = ud.get_data();
        let data_ref = data.borrow();
        if let Some(&ptr) = data_ref.downcast_ref::<usize>() {
            ptr
        } else {
            return Err("gmatch iterator: invalid state type".to_string());
        }
    } else {
        return Err("gmatch iterator: expected userdata".to_string());
    };

    // SAFETY: We created this pointer in string_gmatch and it's managed by the GC
    let state_box = unsafe { &mut *(state_ptr as *mut std::cell::RefCell<GmatchState>) };
    let mut state = state_box.borrow_mut();

    // Parse pattern
    let pattern = match crate::lua_pattern::parse_pattern(&state.pattern) {
        Ok(p) => p,
        Err(e) => return Err(format!("invalid pattern: {}", e)),
    };

    // Find next match
    if let Some((start, end, captures)) =
        crate::lua_pattern::find(&state.string, &pattern, state.position)
    {
        // Update position for next iteration
        state.position = if end > start { end } else { end + 1 };

        // Return captures if any, otherwise return the matched string
        if captures.is_empty() {
            let matched = &state.string[start..end];
            Ok(MultiValue::single(LuaValue::from_string_rc(
                vm.create_string(matched.to_string()),
            )))
        } else {
            let mut results = Vec::new();
            for cap in captures {
                results.push(LuaValue::from_string_rc(vm.create_string(cap)));
            }
            Ok(MultiValue::multiple(results))
        }
    } else {
        // No more matches, return nil
        Ok(MultiValue::single(LuaValue::nil()))
    }
}

/// Optimized cleanup function
fn gmatch_gc_optimized(vm: &mut VM) -> Result<MultiValue, String> {
    #[allow(dead_code)]
    #[derive(Clone)]
    struct GmatchState {
        string: String,
        pattern: String,
        position: usize,
    }

    // For __gc metamethod, register 0 is self (the userdata)
    let frame = vm.frames.last().unwrap();
    if frame.registers.is_empty() {
        return Ok(MultiValue::empty());
    }

    let state_val = &frame.registers[0];

    // Extract state pointer from userdata and free it
    if let Some(ud) = state_val.as_userdata() {
        let data = ud.get_data();
        let data_ref = data.borrow();
        if let Some(&state_ptr) = data_ref.downcast_ref::<usize>() {
            // SAFETY: We own this pointer and are cleaning it up
            unsafe {
                let _ = Box::from_raw(state_ptr as *mut std::cell::RefCell<GmatchState>);
                // Box will be dropped here, freeing the memory
            }
        }
    }

    Ok(MultiValue::empty())
}
