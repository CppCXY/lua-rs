// String library
// Implements: byte, char, dump, find, format, gmatch, gsub, len, lower,
// match, pack, packsize, rep, reverse, sub, unpack, upper

use crate::lib_registry::{LibraryModule, get_arg, require_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;
use crate::lua_pattern;

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
fn string_byte(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.byte")?
        .as_string_rc()
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
fn string_char(vm: &mut LuaVM) -> Result<MultiValue, String> {
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
fn string_len(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.len")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'string.len' (string expected)".to_string())?;

    let len = s.as_str().len() as i64;
    Ok(MultiValue::single(LuaValue::integer(len)))
}

/// string.lower(s) - Convert to lowercase
fn string_lower(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.lower")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'string.lower' (string expected)".to_string())?;

    let result = vm.create_string(s.as_str().to_lowercase());
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.upper(s) - Convert to uppercase
fn string_upper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.upper")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'string.upper' (string expected)".to_string())?;

    let result = vm.create_string(s.as_str().to_uppercase());
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.rep(s, n [, sep]) - Repeat string
fn string_rep(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.rep")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'string.rep' (string expected)".to_string())?;

    let n = require_arg(vm, 1, "string.rep")?
        .as_integer()
        .ok_or_else(|| "bad argument #2 to 'string.rep' (number expected)".to_string())?;

    let sep = get_arg(vm, 2)
        .and_then(|v| v.as_string_rc())
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
fn string_reverse(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.reverse")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'string.reverse' (string expected)".to_string())?;

    let reversed: String = s.as_str().chars().rev().collect();
    let result = vm.create_string(reversed);
    Ok(MultiValue::single(LuaValue::from_string_rc(result)))
}

/// string.sub(s, i [, j]) - Extract substring
fn string_sub(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.sub")?
        .as_string_rc()
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
fn string_format(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let format_str = require_arg(vm, 0, "string.format")?
        .as_string_rc()
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

                        let s = unsafe {
                            if let Some(s) = val.as_string() {
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
                        let s = val.as_string_rc().ok_or_else(|| {
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
fn string_find(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.find")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'string.find' (string expected)".to_string())?;

    let pattern_str = require_arg(vm, 1, "string.find")?
        .as_string_rc()
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
fn string_match(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.match")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'string.match' (string expected)".to_string())?;

    let pattern_str = require_arg(vm, 1, "string.match")?
        .as_string_rc()
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
fn string_gsub(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let arg0 = require_arg(vm, 0, "string.gsub")?;
    let s = unsafe {
        arg0.as_string()
            .ok_or_else(|| "bad argument #1 to 'string.gsub' (string expected)".to_string())?
    };

    let arg1 = require_arg(vm, 1, "string.gsub")?;
    let pattern_str = unsafe {
        arg1.as_string()
            .ok_or_else(|| "bad argument #2 to 'string.gsub' (string expected)".to_string())?
    };

    let arg2 = require_arg(vm, 2, "string.gsub")?;
    let repl = unsafe {
        arg2.as_string()
            .ok_or_else(|| "bad argument #3 to 'string.gsub' (string expected)".to_string())?
    };

    let max = get_arg(vm, 3)
        .and_then(|v| v.as_integer())
        .map(|n| n as usize);

    match lua_pattern::parse_pattern(pattern_str.as_str()) {
        Ok(pattern) => {
            let (result_str, count) =
                lua_pattern::gsub(s.as_str(), &pattern, repl.as_str(), max);

            let result = vm.create_string(result_str);
            Ok(MultiValue::multiple(vec![
                LuaValue::from_string_rc(result),
                LuaValue::integer(count as i64),
            ]))
        }
        Err(e) => Err(format!("invalid pattern: {}", e)),
    }
}

/// string.gmatch(s, pattern) - Returns an iterator function
/// Usage: for capture in string.gmatch(s, pattern) do ... end
fn string_gmatch(vm: &mut LuaVM) -> Result<MultiValue, String> {
    use std::rc::Rc;
    use std::cell::RefCell;
    use crate::lua_value::LuaTable;

    let arg0 = require_arg(vm, 0, "string.gmatch")?;
    let s = unsafe {
        arg0.as_string()
            .ok_or_else(|| "bad argument #1 to 'string.gmatch' (string expected)".to_string())?
    };

    let arg1 = require_arg(vm, 1, "string.gmatch")?;
    let pattern_str = unsafe {
        arg1.as_string()
            .ok_or_else(|| "bad argument #2 to 'string.gmatch' (string expected)".to_string())?
    };

    // Validate pattern early
    let _pattern = match lua_pattern::parse_pattern(pattern_str.as_str()) {
        Ok(p) => p,
        Err(e) => return Err(format!("invalid pattern: {}", e)),
    };

    // Create state table: {string = s, pattern = p, position = 0}
    let state_table = Rc::new(RefCell::new(LuaTable::new()));
    state_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("string".to_string())),
        LuaValue::from_string_rc(vm.create_string(s.as_str().to_string())),
    );
    state_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("pattern".to_string())),
        LuaValue::from_string_rc(vm.create_string(pattern_str.as_str().to_string())),
    );
    state_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("position".to_string())),
        LuaValue::integer(0),
    );

    // Return: iterator function, state table, nil (initial control variable)
    Ok(MultiValue::multiple(vec![
        LuaValue::cfunction(gmatch_iterator),
        LuaValue::from_table_rc(state_table),
        LuaValue::nil(),
    ]))
}

/// Iterator function for string.gmatch
/// Called as: f(state, control_var)
fn gmatch_iterator(vm: &mut LuaVM) -> Result<MultiValue, String> {
    // Arg 0: state table
    // Arg 1: control variable (unused, we use state.position)
    let state_table = require_arg(vm, 0, "gmatch iterator")?
        .as_table_rc()
        .ok_or_else(|| "gmatch iterator: state table expected".to_string())?;

    // Extract string, pattern, and position from state
    let string_key = LuaValue::from_string_rc(vm.create_string("string".to_string()));
    let pattern_key = LuaValue::from_string_rc(vm.create_string("pattern".to_string()));
    let position_key = LuaValue::from_string_rc(vm.create_string("position".to_string()));

    let s_val = state_table
        .borrow()
        .raw_get(&string_key)
        .ok_or_else(|| "gmatch iterator: string not found in state".to_string())?;
    let s = unsafe {
        s_val.as_string()
            .ok_or_else(|| "gmatch iterator: string invalid".to_string())?
    };

    let pattern_val = state_table
        .borrow()
        .raw_get(&pattern_key)
        .ok_or_else(|| "gmatch iterator: pattern not found in state".to_string())?;
    let pattern_str = unsafe {
        pattern_val.as_string()
            .ok_or_else(|| "gmatch iterator: pattern invalid".to_string())?
    };

    let position = state_table
        .borrow()
        .raw_get(&position_key)
        .and_then(|v| v.as_integer())
        .ok_or_else(|| "gmatch iterator: position not found in state".to_string())? as usize;

    // Parse pattern
    let pattern = match lua_pattern::parse_pattern(pattern_str.as_str()) {
        Ok(p) => p,
        Err(e) => return Err(format!("invalid pattern: {}", e)),
    };

    // Find next match
    if let Some((start, end, captures)) = lua_pattern::find(s.as_str(), &pattern, position) {
        // Update position for next iteration
        let next_pos = if end > start { end } else { end + 1 };
        state_table.borrow_mut().raw_set(
            position_key,
            LuaValue::integer(next_pos as i64),
        );

        // Return captures if any, otherwise return the matched string
        if captures.is_empty() {
            let matched = &s.as_str()[start..end];
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
        // No more matches
        Ok(MultiValue::single(LuaValue::nil()))
    }
}

/// string.pack(fmt, v1, v2, ...) - Pack values into binary string
/// Simplified implementation supporting basic format codes
fn string_pack(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let fmt_arg = require_arg(vm, 0, "string.pack")?;
    let fmt = unsafe {
        fmt_arg.as_string()
            .ok_or_else(|| "bad argument #1 to 'string.pack' (string expected)".to_string())?
            .as_str()
    };
    
    let args = crate::lib_registry::get_args(vm);
    let values = &args[1..]; // Skip format string
    
    let mut result = Vec::new();
    let mut value_idx = 0;
    let mut chars = fmt.chars();
    
    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue, // Skip whitespace
            'b' => {
                // signed byte
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let n = values[value_idx].as_integer()
                    .ok_or_else(|| "bad argument to 'string.pack' (number expected)".to_string())?;
                result.push((n & 0xFF) as u8);
                value_idx += 1;
            }
            'B' => {
                // unsigned byte
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let n = values[value_idx].as_integer()
                    .ok_or_else(|| "bad argument to 'string.pack' (number expected)".to_string())?;
                result.push((n & 0xFF) as u8);
                value_idx += 1;
            }
            'h' => {
                // signed short (2 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let n = values[value_idx].as_integer()
                    .ok_or_else(|| "bad argument to 'string.pack' (number expected)".to_string())? as i16;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'H' => {
                // unsigned short (2 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let n = values[value_idx].as_integer()
                    .ok_or_else(|| "bad argument to 'string.pack' (number expected)".to_string())? as u16;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'i' | 'l' => {
                // signed int (4 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let n = values[value_idx].as_integer()
                    .ok_or_else(|| "bad argument to 'string.pack' (number expected)".to_string())? as i32;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'I' | 'L' => {
                // unsigned int (4 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let n = values[value_idx].as_integer()
                    .ok_or_else(|| "bad argument to 'string.pack' (number expected)".to_string())? as u32;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'f' => {
                // float (4 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let n = values[value_idx].as_number()
                    .ok_or_else(|| "bad argument to 'string.pack' (number expected)".to_string())? as f32;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'd' => {
                // double (8 bytes, little-endian)
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let n = values[value_idx].as_number()
                    .ok_or_else(|| "bad argument to 'string.pack' (number expected)".to_string())?;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }
            'z' => {
                // zero-terminated string
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let s = unsafe {
                    values[value_idx].as_string()
                        .ok_or_else(|| "bad argument to 'string.pack' (string expected)".to_string())?
                };
                result.extend_from_slice(s.as_str().as_bytes());
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
                let size: usize = size_str.parse()
                    .map_err(|_| "bad argument to 'string.pack' (invalid size)".to_string())?;;
                    
                if value_idx >= values.len() {
                    return Err("bad argument to 'string.pack' (not enough values)".to_string());
                }
                let s = unsafe {
                    values[value_idx].as_string()
                        .ok_or_else(|| "bad argument to 'string.pack' (string expected)".to_string())?
                };
                let bytes = s.as_str().as_bytes();
                result.extend_from_slice(&bytes[..size.min(bytes.len())]);
                // Pad with zeros if needed
                for _ in bytes.len()..size {
                    result.push(0);
                }
                value_idx += 1;
            }
            _ => {
                return Err(format!("bad argument to 'string.pack' (invalid format option '{}')", ch));
            }
        }
    }
    
    // Create a string directly from bytes without UTF-8 validation
    // Lua strings can contain arbitrary binary data
    let packed = unsafe { String::from_utf8_unchecked(result) };
    Ok(MultiValue::single(LuaValue::from_string_rc(vm.create_string(packed))))
}

/// string.packsize(fmt) - Return size of packed data
fn string_packsize(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let fmt_arg = require_arg(vm, 0, "string.packsize")?;
    let fmt = unsafe {
        fmt_arg.as_string()
            .ok_or_else(|| "bad argument #1 to 'string.packsize' (string expected)".to_string())?
            .as_str()
    };
    
    let mut size = 0usize;
    let mut chars = fmt.chars();
    
    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue,
            'b' | 'B' => size += 1,
            'h' | 'H' => size += 2,
            'i' | 'I' | 'l' | 'L' | 'f' => size += 4,
            'd' => size += 8,
            'z' => {
                return Err("variable-length format in 'string.packsize'".to_string());
            }
            'c' => {
                let mut size_str = String::new();
                loop {
                    match chars.next() {
                        Some(digit) if digit.is_ascii_digit() => size_str.push(digit),
                        _ => break,
                    }
                }
                let n: usize = size_str.parse()
                    .map_err(|_| "bad argument to 'string.packsize' (invalid size)".to_string())?;
                size += n;
            }
            _ => {
                return Err(format!("bad argument to 'string.packsize' (invalid format option '{}')", ch));
            }
        }
    }
    
    Ok(MultiValue::single(LuaValue::integer(size as i64)))
}

/// string.unpack(fmt, s [, pos]) - Unpack binary string
fn string_unpack(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let fmt_arg = require_arg(vm, 0, "string.unpack")?;
    let fmt = unsafe {
        fmt_arg.as_string()
            .ok_or_else(|| "bad argument #1 to 'string.unpack' (string expected)".to_string())?
            .as_str()
    };
    
    let s_arg = require_arg(vm, 1, "string.unpack")?;
    let s = unsafe {
        s_arg.as_string()
            .ok_or_else(|| "bad argument #2 to 'string.unpack' (string expected)".to_string())?
    };
    let bytes = s.as_str().as_bytes();
    
    let pos = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(1) as usize - 1; // Convert to 0-based
    
    let mut results = Vec::new();
    let mut idx = pos;
    let mut chars = fmt.chars();
    
    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue,
            'b' => {
                if idx >= bytes.len() {
                    return Err("data string too short".to_string());
                }
                results.push(LuaValue::integer(bytes[idx] as i8 as i64));
                idx += 1;
            }
            'B' => {
                if idx >= bytes.len() {
                    return Err("data string too short".to_string());
                }
                results.push(LuaValue::integer(bytes[idx] as i64));
                idx += 1;
            }
            'h' => {
                if idx + 2 > bytes.len() {
                    return Err("data string too short".to_string());
                }
                let val = i16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
                results.push(LuaValue::integer(val as i64));
                idx += 2;
            }
            'H' => {
                if idx + 2 > bytes.len() {
                    return Err("data string too short".to_string());
                }
                let val = u16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
                results.push(LuaValue::integer(val as i64));
                idx += 2;
            }
            'i' | 'l' => {
                if idx + 4 > bytes.len() {
                    return Err("data string too short".to_string());
                }
                let val = i32::from_le_bytes([bytes[idx], bytes[idx + 1], bytes[idx + 2], bytes[idx + 3]]);
                results.push(LuaValue::integer(val as i64));
                idx += 4;
            }
            'I' | 'L' => {
                if idx + 4 > bytes.len() {
                    return Err("data string too short".to_string());
                }
                let val = u32::from_le_bytes([bytes[idx], bytes[idx + 1], bytes[idx + 2], bytes[idx + 3]]);
                results.push(LuaValue::integer(val as i64));
                idx += 4;
            }
            'f' => {
                if idx + 4 > bytes.len() {
                    return Err("data string too short".to_string());
                }
                let val = f32::from_le_bytes([bytes[idx], bytes[idx + 1], bytes[idx + 2], bytes[idx + 3]]);
                results.push(LuaValue::float(val as f64));
                idx += 4;
            }
            'd' => {
                if idx + 8 > bytes.len() {
                    return Err("data string too short".to_string());
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
                let s = String::from_utf8_lossy(&bytes[start..idx]).to_string();
                results.push(LuaValue::from_string_rc(vm.create_string(s)));
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
                let size: usize = size_str.parse()
                    .map_err(|_| "bad argument to 'string.unpack' (invalid size)".to_string())?;
                    
                if idx + size > bytes.len() {
                    return Err("data string too short".to_string());
                }
                let s = String::from_utf8_lossy(&bytes[idx..idx + size]).to_string();
                results.push(LuaValue::from_string_rc(vm.create_string(s)));
                idx += size;
            }
            _ => {
                return Err(format!("bad argument to 'string.unpack' (invalid format option '{}')", ch));
            }
        }
    }
    
    // Return unpacked values plus next position
    results.push(LuaValue::integer((idx + 1) as i64)); // Convert back to 1-based
    Ok(MultiValue::multiple(results))
}
