// String library
// Implements: byte, char, dump, find, format, gmatch, gsub, len, lower, 
// match, pack, packsize, rep, reverse, sub, unpack, upper

use crate::lib_registry::{LibraryModule, get_arg, require_arg, arg_count};
use crate::value::{LuaValue, MultiValue};
use crate::vm::VM;

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
    })
}

/// string.byte(s [, i [, j]]) - Return byte values
fn string_byte(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.byte")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.byte' (string expected)".to_string())?;
    
    let str_bytes = s.as_str().as_bytes();
    let len = str_bytes.len() as i64;
    
    let i = get_arg(vm, 1)
        .and_then(|v| v.as_integer())
        .unwrap_or(1);
    
    let j = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(i);
    
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
            result.push(LuaValue::Integer(byte as i64));
        }
    }
    
    Ok(MultiValue::multiple(result))
}

/// string.char(...) - Convert bytes to string
fn string_char(vm: &mut VM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);
    
    let mut bytes = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let byte = arg.as_integer()
            .ok_or_else(|| format!("bad argument #{} to 'string.char' (number expected)", i + 1))?;
        
        if byte < 0 || byte > 255 {
            return Err(format!("bad argument #{} to 'string.char' (value out of range)", i + 1));
        }
        
        bytes.push(byte as u8);
    }
    
    let result_str = String::from_utf8(bytes)
        .map_err(|_| "string.char: invalid UTF-8".to_string())?;
    
    let result = vm.create_string(result_str);
    Ok(MultiValue::single(LuaValue::String(result)))
}

/// string.len(s) - Return string length
fn string_len(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.len")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.len' (string expected)".to_string())?;
    
    let len = s.as_str().len() as i64;
    Ok(MultiValue::single(LuaValue::Integer(len)))
}

/// string.lower(s) - Convert to lowercase
fn string_lower(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.lower")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.lower' (string expected)".to_string())?;
    
    let result = vm.create_string(s.as_str().to_lowercase());
    Ok(MultiValue::single(LuaValue::String(result)))
}

/// string.upper(s) - Convert to uppercase
fn string_upper(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.upper")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.upper' (string expected)".to_string())?;
    
    let result = vm.create_string(s.as_str().to_uppercase());
    Ok(MultiValue::single(LuaValue::String(result)))
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
        return Ok(MultiValue::single(LuaValue::String(empty)));
    }
    
    let mut result = String::new();
    for i in 0..n {
        if i > 0 && !sep.is_empty() {
            result.push_str(&sep);
        }
        result.push_str(s.as_str());
    }
    
    let result = vm.create_string(result);
    Ok(MultiValue::single(LuaValue::String(result)))
}

/// string.reverse(s) - Reverse string
fn string_reverse(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.reverse")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.reverse' (string expected)".to_string())?;
    
    let reversed: String = s.as_str().chars().rev().collect();
    let result = vm.create_string(reversed);
    Ok(MultiValue::single(LuaValue::String(result)))
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
    
    let j = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(-1);
    
    // Convert negative indices
    let start = if i < 0 { len + i + 1 } else { i };
    let end = if j < 0 { len + j + 1 } else { j };
    
    let start = start.max(1).min(len + 1) as usize;
    let end = end.max(0).min(len) as usize;
    
    let result_str = if start <= end {
        s.as_str().chars()
            .skip(start - 1)
            .take(end - start + 1)
            .collect::<String>()
    } else {
        String::new()
    };
    
    let result = vm.create_string(result_str);
    Ok(MultiValue::single(LuaValue::String(result)))
}

/// string.format(formatstring, ...) - Format string (simplified)
fn string_format(vm: &mut VM) -> Result<MultiValue, String> {
    let format_str = require_arg(vm, 0, "string.format")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.format' (string expected)".to_string())?;
    
    // TODO: Implement proper format string parsing
    // For now, just return the format string
    let result = vm.create_string(format_str.as_str().to_string());
    Ok(MultiValue::single(LuaValue::String(result)))
}

/// string.find(s, pattern [, init [, plain]]) - Find pattern
fn string_find(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.find")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.find' (string expected)".to_string())?;
    
    let pattern_str = require_arg(vm, 1, "string.find")?
        .as_string()
        .ok_or_else(|| "bad argument #2 to 'string.find' (string expected)".to_string())?;
    
    let init = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(1);
    
    let plain = get_arg(vm, 3)
        .map(|v| v.is_truthy())
        .unwrap_or(false);
    
    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };
    
    if plain {
        // Plain string search (no pattern matching)
        if let Some(pos) = s.as_str()[start_pos..].find(pattern_str.as_str()) {
            let actual_pos = start_pos + pos;
            Ok(MultiValue::multiple(vec![
                LuaValue::Integer((actual_pos + 1) as i64),
                LuaValue::Integer((actual_pos + pattern_str.as_str().len()) as i64),
            ]))
        } else {
            Ok(MultiValue::single(LuaValue::Nil))
        }
    } else {
        // Pattern matching
        match crate::lua_pattern::parse_pattern(pattern_str.as_str()) {
            Ok(pattern) => {
                if let Some((start, end, captures)) = crate::lua_pattern::find(s.as_str(), &pattern, start_pos) {
                    let mut results = vec![
                        LuaValue::Integer((start + 1) as i64),
                        LuaValue::Integer(end as i64),
                    ];
                    // Add captures
                    for cap in captures {
                        results.push(LuaValue::String(vm.create_string(cap)));
                    }
                    Ok(MultiValue::multiple(results))
                } else {
                    Ok(MultiValue::single(LuaValue::Nil))
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
    
    let init = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(1);
    
    let start_pos = if init > 0 { (init - 1) as usize } else { 0 };
    let text = &s.as_str()[start_pos..];
    
    match crate::lua_pattern::parse_pattern(pattern_str.as_str()) {
        Ok(pattern) => {
            if let Some((start, end, captures)) = crate::lua_pattern::find(text, &pattern, 0) {
                if captures.is_empty() {
                    // No captures, return the matched portion
                    let matched = &text[start..end];
                    Ok(MultiValue::single(LuaValue::String(vm.create_string(matched.to_string()))))
                } else {
                    // Return captures
                    let results: Vec<LuaValue> = captures
                        .into_iter()
                        .map(|s| LuaValue::String(vm.create_string(s)))
                        .collect();
                    Ok(MultiValue::multiple(results))
                }
            } else {
                Ok(MultiValue::single(LuaValue::Nil))
            }
        }
        Err(e) => Err(format!("invalid pattern: {}", e)),
    }
}

/// string.gsub(s, pattern, repl [, n]) - Global substitution
fn string_gsub(vm: &mut VM) -> Result<MultiValue, String> {
    let s = require_arg(vm, 0, "string.gsub")?
        .as_string()
        .ok_or_else(|| "bad argument #1 to 'string.gsub' (string expected)".to_string())?;
    
    let pattern_str = require_arg(vm, 1, "string.gsub")?
        .as_string()
        .ok_or_else(|| "bad argument #2 to 'string.gsub' (string expected)".to_string())?;
    
    let repl = require_arg(vm, 2, "string.gsub")?
        .as_string()
        .ok_or_else(|| "bad argument #3 to 'string.gsub' (string expected)".to_string())?;
    
    let max = get_arg(vm, 3).and_then(|v| v.as_integer()).map(|n| n as usize);
    
    match crate::lua_pattern::parse_pattern(pattern_str.as_str()) {
        Ok(pattern) => {
            let (result_str, count) = crate::lua_pattern::gsub(s.as_str(), &pattern, repl.as_str(), max);
            
            let result = vm.create_string(result_str);
            Ok(MultiValue::multiple(vec![
                LuaValue::String(result),
                LuaValue::Integer(count as i64),
            ]))
        }
        Err(e) => Err(format!("invalid pattern: {}", e)),
    }
}

/// string.gmatch(s, pattern) - Iterator for pattern matches (stub)
fn string_gmatch(vm: &mut VM) -> Result<MultiValue, String> {
    // TODO: Implement pattern matching iterator
    let dummy_func = LuaValue::CFunction(|_| Ok(MultiValue::empty()));
    Ok(MultiValue::single(dummy_func))
}

