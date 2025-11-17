// UTF-8 library
// Implements: char, charpattern, codes, codepoint, len, offset

use crate::lib_registry::{LibraryModule, get_arg, get_args, require_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;

pub fn create_utf8_lib() -> LibraryModule {
    let mut module = crate::lib_module!("utf8", {
        "len" => utf8_len,
        "char" => utf8_char,
        "codes" => utf8_codes,
        "codepoint" => utf8_codepoint,
        "offset" => utf8_offset,
    });

    // Add charpattern constant
    module = module.with_value("charpattern", |vm| {
        // UTF-8 character pattern for pattern matching
        // This pattern matches any valid UTF-8 character sequence
        let pattern = "[\\x00-\\x7F\\xC2-\\xF4][\\x80-\\xBF]*";
        vm.create_string(&pattern)
    });

    module
}

fn utf8_len(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s_value = require_arg(vm, 0, "utf8.len")?;
    let s = vm.get_string(&s_value)
        .ok_or_else(|| "bad argument #1 to 'utf8.len' (string expected)".to_string())?;

    let bytes = s.as_str().as_bytes();
    let len = bytes.len() as i64;

    // Fast path: no range specified, count entire string
    let i_arg = get_arg(vm, 1);
    let j_arg = get_arg(vm, 2);
    let lax = get_arg(vm, 3).and_then(|v| v.as_bool()).unwrap_or(false);

    if i_arg.is_none() && j_arg.is_none() {
        // Fast path: validate and count entire string
        match std::str::from_utf8(bytes) {
            Ok(valid_str) => {
                return Ok(MultiValue::single(LuaValue::integer(
                    valid_str.chars().count() as i64,
                )));
            }
            Err(e) if !lax => {
                // Return nil and position of first invalid byte (1-based)
                return Ok(MultiValue::multiple(vec![
                    LuaValue::nil(),
                    LuaValue::integer(e.valid_up_to() as i64 + 1),
                ]));
            }
            Err(_) if lax => {
                // In lax mode, just return nil
                return Ok(MultiValue::single(LuaValue::nil()));
            }
            _ => unreachable!(),
        }
    }

    // Slow path: i and j are BYTE positions (1-based), not character positions
    let i = i_arg.and_then(|v| v.as_integer()).unwrap_or(1);
    let j = j_arg.and_then(|v| v.as_integer()).unwrap_or(len);

    // Convert 1-based byte positions to 0-based byte indices
    let start_byte = ((i - 1).max(0) as usize).min(bytes.len());
    let end_byte = (j.max(0) as usize).min(bytes.len());

    if start_byte > end_byte {
        return Ok(MultiValue::multiple(vec![
            LuaValue::nil(),
            LuaValue::integer(start_byte as i64 + 1),
        ]));
    }

    // Count UTF-8 characters in byte range
    match std::str::from_utf8(&bytes[start_byte..end_byte]) {
        Ok(valid_str) => {
            let len = valid_str.chars().count();
            Ok(MultiValue::single(LuaValue::integer(len as i64)))
        }
        Err(e) if !lax => {
            // Return nil and position of first invalid byte (1-based)
            let error_pos = start_byte + e.valid_up_to() + 1;
            Ok(MultiValue::multiple(vec![
                LuaValue::nil(),
                LuaValue::integer(error_pos as i64),
            ]))
        }
        Err(_) if lax => {
            // In lax mode, just return nil
            Ok(MultiValue::single(LuaValue::nil()))
        }
        _ => unreachable!(),
    }
}

fn utf8_char(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let args = get_args(vm);

    let mut result = String::new();
    for arg in args {
        if let Some(code) = arg.as_integer() {
            if code < 0 || code > 0x10FFFF {
                return Err(format!("bad argument to 'utf8.char' (value out of range)"));
            }
            if let Some(ch) = char::from_u32(code as u32) {
                result.push(ch);
            } else {
                return Err(format!("bad argument to 'utf8.char' (invalid code point)"));
            }
        } else {
            return Err("bad argument to 'utf8.char' (number expected)".to_string());
        }
    }

    let s = vm.create_string(&result);
    Ok(MultiValue::single(s))
}

/// utf8.codes(s) - Returns an iterator for UTF-8 characters
fn utf8_codes(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s_value = require_arg(vm, 0, "utf8.codes")?;
    if !s_value.is_string() {
        return Err("bad argument #1 to 'utf8.codes' (string expected)".to_string());
    }

    // Create state table: {string = s, position = 0}
    let state_table = vm.create_table();
    let string_key = vm.create_string("string");
    let position_key = vm.create_string("position");
    let state_ref = vm.get_table(&state_table).ok_or("Invalid state table")?;
    state_ref.borrow_mut().raw_set(string_key, s_value);
    state_ref
        .borrow_mut()
        .raw_set(position_key, LuaValue::integer(0));

    Ok(MultiValue::multiple(vec![
        LuaValue::cfunction(utf8_codes_iterator),
        state_table,
        LuaValue::nil(),
    ]))
}

/// Iterator function for utf8.codes
fn utf8_codes_iterator(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let t_value = require_arg(vm, 0, "utf8.codes iterator")?;
    let state_table = t_value
        .as_table_id()
        .ok_or_else(|| "utf8.codes iterator: state table expected".to_string())?;

    let string_key = vm.create_string("string");
    let position_key = vm.create_string("position");

    let state_ref_cell = vm.get_table(&t_value).ok_or("Invalid state table")?;
    let s_val = state_ref_cell
        .borrow()
        .raw_get(&string_key)
        .ok_or_else(|| "utf8.codes iterator: string not found".to_string())?;
    let s = unsafe {
        s_val
            .as_string()
            .ok_or_else(|| "utf8.codes iterator: invalid string".to_string())?
    };

    let pos = state_ref_cell
        .borrow()
        .raw_get(&position_key)
        .and_then(|v| v.as_integer())
        .ok_or_else(|| "utf8.codes iterator: position not found".to_string())?
        as usize;

    let bytes = s.as_str().as_bytes();
    if pos >= bytes.len() {
        return Ok(MultiValue::single(LuaValue::nil()));
    }

    // Decode next UTF-8 character
    let remaining = &s.as_str()[pos..];
    if let Some(ch) = remaining.chars().next() {
        let char_len = ch.len_utf8();
        let code_point = ch as u32;

        // Update position
        state_ref_cell
            .borrow_mut()
            .raw_set(position_key, LuaValue::integer((pos + char_len) as i64));

        Ok(MultiValue::multiple(vec![
            LuaValue::integer((pos + 1) as i64), // 1-based position
            LuaValue::integer(code_point as i64),
        ]))
    } else {
        Ok(MultiValue::single(LuaValue::nil()))
    }
}

/// utf8.codepoint(s [, i [, j]]) - Returns code points of characters
fn utf8_codepoint(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s_value = require_arg(vm, 0, "utf8.codepoint")?;
    let s = vm.get_string(&s_value)
        .ok_or_else(|| "bad argument #1 to 'utf8.codepoint' (string expected)".to_string())?;

    let i = get_arg(vm, 1).and_then(|v| v.as_integer()).unwrap_or(1) as usize;

    let j = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .map(|v| v as usize)
        .unwrap_or(i);

    let bytes = s.as_str().as_bytes();
    let start_byte = if i > 0 { i - 1 } else { 0 };
    let end_byte = if j > 0 { j } else { bytes.len() };

    if start_byte >= bytes.len() {
        return Err("bad argument #2 to 'utf8.codepoint' (out of range)".to_string());
    }

    let mut results = Vec::new();
    let mut pos = start_byte;

    while pos < end_byte && pos < bytes.len() {
        let remaining = &s.as_str()[pos..];
        if let Some(ch) = remaining.chars().next() {
            results.push(LuaValue::integer(ch as u32 as i64));
            pos += ch.len_utf8();
        } else {
            break;
        }
    }

    Ok(MultiValue::multiple(results))
}

/// utf8.offset(s, n [, i]) - Returns byte position of n-th character
fn utf8_offset(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s_value = require_arg(vm, 0, "utf8.offset")?;
    let s = vm.get_string(&s_value)
        .ok_or_else(|| "bad argument #1 to 'utf8.offset' (string expected)".to_string())?;

    let n = require_arg(vm, 1, "utf8.offset")?
        .as_integer()
        .ok_or_else(|| "bad argument #2 to 'utf8.offset' (number expected)".to_string())?;

    let i = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(if n >= 0 {
            1
        } else {
            (s.as_str().len() + 1) as i64
        }) as usize;

    let bytes = s.as_str().as_bytes();
    let start_byte = if i > 0 { i - 1 } else { 0 };

    if start_byte > bytes.len() {
        return Ok(MultiValue::single(LuaValue::nil()));
    }

    let mut pos = start_byte;
    let mut count = n;

    if n >= 0 {
        // Forward: find the n-th character from position i
        // When n=1, we want the position of the 1st character starting from i
        // So we move (n-1) characters forward
        count -= 1; // Adjust: we want to arrive at the n-th char, not move n chars
        while count > 0 && pos < bytes.len() {
            let remaining = &s.as_str()[pos..];
            if let Some(ch) = remaining.chars().next() {
                pos += ch.len_utf8();
                count -= 1;
            } else {
                return Ok(MultiValue::single(LuaValue::nil()));
            }
        }
        if count == 0 {
            Ok(MultiValue::single(LuaValue::integer((pos + 1) as i64)))
        } else {
            Ok(MultiValue::single(LuaValue::nil()))
        }
    } else {
        // Backward
        while count < 0 && pos > 0 {
            pos -= 1;
            // Find start of UTF-8 character
            while pos > 0 && (bytes[pos] & 0xC0) == 0x80 {
                pos -= 1;
            }
            count += 1;
        }
        if count == 0 {
            Ok(MultiValue::single(LuaValue::integer((pos + 1) as i64)))
        } else {
            Ok(MultiValue::single(LuaValue::nil()))
        }
    }
}
