// UTF-8 library
// Implements: char, charpattern, codes, codepoint, len, offset

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::LuaResult;
use crate::lua_vm::LuaState;

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

fn utf8_len(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'len' (string expected)".to_string()))?;
    let Some(s_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'len' (string expected)".to_string()));
    };
    let s_str = {
        let vm = l.vm_mut();
        let Some(s) = vm.object_pool.get_string(s_id) else {
            return Err(l.error("bad argument #1 to 'len' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    let bytes = s_str.as_bytes();
    let len = bytes.len() as i64;

    // Fast path: no range specified, count entire string
    let i_arg = l.get_arg(2);
    let j_arg = l.get_arg(3);
    let lax = l.get_arg(4).and_then(|v| v.as_bool()).unwrap_or(false);

    if i_arg.is_none() && j_arg.is_none() {
        // Fast path: validate and count entire string
        match std::str::from_utf8(bytes) {
            Ok(valid_str) => {
                l.push_value(LuaValue::integer(valid_str.chars().count() as i64))?;
                return Ok(1);
            }
            Err(e) if !lax => {
                // Return nil and position of first invalid byte (1-based)
                l.push_value(LuaValue::nil())?;
                l.push_value(LuaValue::integer(e.valid_up_to() as i64 + 1))?;
                return Ok(2);
            }
            Err(_) if lax => {
                // In lax mode, just return nil
                l.push_value(LuaValue::nil())?;
                return Ok(1);
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
        l.push_value(LuaValue::nil())?;
        l.push_value(LuaValue::integer(start_byte as i64 + 1))?;
        return Ok(2);
    }

    // Count UTF-8 characters in byte range
    match std::str::from_utf8(&bytes[start_byte..end_byte]) {
        Ok(valid_str) => {
            let len = valid_str.chars().count();
            l.push_value(LuaValue::integer(len as i64))?;
            Ok(1)
        }
        Err(e) if !lax => {
            // Return nil and position of first invalid byte (1-based)
            let error_pos = start_byte + e.valid_up_to() + 1;
            l.push_value(LuaValue::nil())?;
            l.push_value(LuaValue::integer(error_pos as i64))?;
            Ok(2)
        }
        Err(_) if lax => {
            // In lax mode, just return nil
            l.push_value(LuaValue::nil())?;
            Ok(1)
        }
        _ => unreachable!(),
    }
}

fn utf8_char(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();

    let mut result = String::new();
    for arg in args {
        if let Some(code) = arg.as_integer() {
            if code < 0 || code > 0x10FFFF {
                return Err(l.error("bad argument to 'char' (value out of range)".to_string()));
            }
            if let Some(ch) = char::from_u32(code as u32) {
                result.push(ch);
            } else {
                return Err(l.error("bad argument to 'char' (invalid code point)".to_string()));
            }
        } else {
            return Err(l.error("bad argument to 'char' (number expected)".to_string()));
        }
    }

    let s = l.create_string(&result);
    l.push_value(s)?;
    Ok(1)
}

/// utf8.codes(s) - Returns an iterator for UTF-8 characters
fn utf8_codes(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'codes' (string expected)".to_string()))?;
    if !s_value.is_string() {
        return Err(l.error("bad argument #1 to 'codes' (string expected)".to_string()));
    }

    // Create state table: {string = s, position = 0}
    let state_table = l.create_table(2, 0);
    let string_key = LuaValue::integer(1);
    let position_key = LuaValue::integer(2);

    // Use ObjectPool for table access
    if let Some(table_id) = state_table.as_table_id() {
        let vm = l.vm_mut();
        if let Some(table) = vm.object_pool.get_table_mut(table_id) {
            table.raw_set(string_key, s_value);
            table.raw_set(position_key, LuaValue::integer(0));
        }
    }

    l.push_value(LuaValue::cfunction(utf8_codes_iterator))?;
    l.push_value(state_table)?;
    l.push_value(LuaValue::nil())?;
    Ok(3)
}

/// Iterator function for utf8.codes
fn utf8_codes_iterator(l: &mut LuaState) -> LuaResult<usize> {
    let t_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("utf8.codes iterator: invalid state".to_string()))?;

    let string_key = 1;
    let position_key = 2;

    let Some(table_id) = t_value.as_table_id() else {
        return Err(l.error("utf8.codes iterator: invalid state".to_string()));
    };

    // Extract string and position from state table
    let (s_str, pos) = {
        let vm = l.vm_mut();
        let Some(table) = vm.object_pool.get_table(table_id) else {
            return Err(l.error("utf8.codes iterator: invalid state".to_string()));
        };

        let Some(s_val) = table.get_int(string_key) else {
            return Err(l.error("utf8.codes iterator: string not found".to_string()));
        };

        let Some(s_id) = s_val.as_string_id() else {
            return Err(l.error("utf8.codes iterator: invalid string".to_string()));
        };

        let pos = table
            .get_int(position_key)
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as usize;

        // Get string content
        let Some(lua_str) = vm.object_pool.get_string(s_id) else {
            return Err(l.error("utf8.codes iterator: invalid string".to_string()));
        };

        (lua_str.as_str().to_string(), pos)
    };

    let bytes = s_str.as_bytes();
    if pos >= bytes.len() {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    }

    // Decode next UTF-8 character
    let remaining = &s_str[pos..];
    if let Some(ch) = remaining.chars().next() {
        let char_len = ch.len_utf8();
        let code_point = ch as u32;

        // Update position in the state table
        let vm = l.vm_mut();
        if let Some(table) = vm.object_pool.get_table_mut(table_id) {
            table.raw_set(
                LuaValue::integer(position_key),
                LuaValue::integer((pos + char_len) as i64),
            );
        }

        l.push_value(LuaValue::integer((pos + 1) as i64))?; // 1-based position
        l.push_value(LuaValue::integer(code_point as i64))?;
        Ok(2)
    } else {
        l.push_value(LuaValue::nil())?;
        Ok(1)
    }
}

/// utf8.codepoint(s [, i [, j]]) - Returns code points of characters
fn utf8_codepoint(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'codepoint' (string expected)".to_string()))?;
    let Some(s_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'codepoint' (string expected)".to_string()));
    };
    let s_str = {
        let vm = l.vm_mut();
        let Some(s) = vm.object_pool.get_string(s_id) else {
            return Err(l.error("bad argument #1 to 'codepoint' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    let i = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(1) as usize;

    let j = l
        .get_arg(3)
        .and_then(|v| v.as_integer())
        .map(|v| v as usize)
        .unwrap_or(i);

    let bytes = s_str.as_bytes();
    let start_byte = if i > 0 { i - 1 } else { 0 };
    let end_byte = if j > 0 { j } else { bytes.len() };

    if start_byte >= bytes.len() {
        return Err(l.error("bad argument #2 to 'codepoint' (out of range)".to_string()));
    }

    let mut count = 0;
    let mut pos = start_byte;

    while pos < end_byte && pos < bytes.len() {
        let remaining = &s_str[pos..];
        if let Some(ch) = remaining.chars().next() {
            l.push_value(LuaValue::integer(ch as u32 as i64))?;
            count += 1;
            pos += ch.len_utf8();
        } else {
            break;
        }
    }

    Ok(count)
}

/// utf8.offset(s, n [, i]) - Returns byte position of n-th character
fn utf8_offset(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'offset' (string expected)".to_string()))?;
    let Some(s_id) = s_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'offset' (string expected)".to_string()));
    };
    let s_str = {
        let vm = l.vm_mut();
        let Some(s) = vm.object_pool.get_string(s_id) else {
            return Err(l.error("bad argument #1 to 'offset' (string expected)".to_string()));
        };
        s.as_str().to_string()
    };

    let n_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'offset' (number expected)".to_string()))?;
    let Some(n) = n_value.as_integer() else {
        return Err(l.error("bad argument #2 to 'offset' (number expected)".to_string()));
    };
    let i = l
        .get_arg(3)
        .and_then(|v| v.as_integer())
        .unwrap_or(if n >= 0 { 1 } else { (s_str.len() + 1) as i64 }) as usize;

    let bytes = s_str.as_bytes();
    let start_byte = if i > 0 { i - 1 } else { 0 };

    if start_byte > bytes.len() {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    }

    let mut pos = start_byte;
    let mut count = n;

    if n >= 0 {
        // Forward: find the n-th character from position i
        // When n=1, we want the position of the 1st character starting from i
        // So we move (n-1) characters forward
        count -= 1; // Adjust: we want to arrive at the n-th char, not move n chars
        while count > 0 && pos < bytes.len() {
            let remaining = &s_str[pos..];
            if let Some(ch) = remaining.chars().next() {
                pos += ch.len_utf8();
                count -= 1;
            } else {
                l.push_value(LuaValue::nil())?;
                return Ok(1);
            }
        }
        if count == 0 {
            l.push_value(LuaValue::integer((pos + 1) as i64))?;
            Ok(1)
        } else {
            l.push_value(LuaValue::nil())?;
            Ok(1)
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
            l.push_value(LuaValue::integer((pos + 1) as i64))?;
            Ok(1)
        } else {
            l.push_value(LuaValue::nil())?;
            Ok(1)
        }
    }
}
