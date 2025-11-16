// UTF-8 library
// Implements: char, charpattern, codes, codepoint, len, offset

use crate::lib_registry::LibraryModule;
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
        LuaValue::from_string_rc(vm.create_string(pattern.to_string()))
    });
    
    module
}

fn utf8_len(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let s = crate::lib_registry::require_arg(vm, 0, "utf8.len")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'utf8.len' (string expected)".to_string())?;

    let i = crate::lib_registry::get_arg(vm, 1)
        .and_then(|v| v.as_integer())
        .unwrap_or(1) as usize;
    
    let j = crate::lib_registry::get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);

    let s_str = s.as_str();
    let bytes = s_str.as_bytes();
    
    // Convert 1-based indices to 0-based byte positions
    let start_byte = if i > 0 { i - 1 } else { 0 };
    let end_byte = j.unwrap_or(bytes.len());
    
    if start_byte > bytes.len() || end_byte > bytes.len() || start_byte > end_byte {
        return Ok(MultiValue::multiple(vec![
            LuaValue::nil(),
            LuaValue::integer(start_byte as i64 + 1),
        ]));
    }
    
    // Count UTF-8 characters
    let slice = &s_str[start_byte..end_byte];
    match std::str::from_utf8(slice.as_bytes()) {
        Ok(valid_str) => {
            let len = valid_str.chars().count();
            Ok(MultiValue::single(LuaValue::integer(len as i64)))
        }
        Err(e) => {
            // Return nil and position of invalid byte
            let error_pos = start_byte + e.valid_up_to();
            Ok(MultiValue::multiple(vec![
                LuaValue::nil(),
                LuaValue::integer(error_pos as i64 + 1),
            ]))
        }
    }
}

fn utf8_char(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);

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

    let s = vm.create_string(result);
    Ok(MultiValue::single(LuaValue::from_string_rc(s)))
}

/// utf8.codes(s) - Returns an iterator for UTF-8 characters
fn utf8_codes(vm: &mut LuaVM) -> Result<MultiValue, String> {
    use std::rc::Rc;
    use std::cell::RefCell;
    use crate::lua_value::LuaTable;
    
    let s = crate::lib_registry::require_arg(vm, 0, "utf8.codes")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'utf8.codes' (string expected)".to_string())?;
    
    // Create state table: {string = s, position = 0}
    let state_table = Rc::new(RefCell::new(LuaTable::new()));
    state_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("string".to_string())),
        LuaValue::from_string_rc(s),
    );
    state_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("position".to_string())),
        LuaValue::integer(0),
    );
    
    Ok(MultiValue::multiple(vec![
        LuaValue::cfunction(utf8_codes_iterator),
        LuaValue::from_table_rc(state_table),
        LuaValue::nil(),
    ]))
}

/// Iterator function for utf8.codes
fn utf8_codes_iterator(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let state_table = crate::lib_registry::require_arg(vm, 0, "utf8.codes iterator")?
        .as_table_rc()
        .ok_or_else(|| "utf8.codes iterator: state table expected".to_string())?;
    
    let string_key = LuaValue::from_string_rc(vm.create_string("string".to_string()));
    let position_key = LuaValue::from_string_rc(vm.create_string("position".to_string()));
    
    let s_val = state_table.borrow().raw_get(&string_key)
        .ok_or_else(|| "utf8.codes iterator: string not found".to_string())?;
    let s = unsafe {
        s_val.as_string()
            .ok_or_else(|| "utf8.codes iterator: invalid string".to_string())?
    };
    
    let pos = state_table.borrow().raw_get(&position_key)
        .and_then(|v| v.as_integer())
        .ok_or_else(|| "utf8.codes iterator: position not found".to_string())? as usize;
    
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
        state_table.borrow_mut().raw_set(
            position_key,
            LuaValue::integer((pos + char_len) as i64),
        );
        
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
    let s = crate::lib_registry::require_arg(vm, 0, "utf8.codepoint")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'utf8.codepoint' (string expected)".to_string())?;
    
    let i = crate::lib_registry::get_arg(vm, 1)
        .and_then(|v| v.as_integer())
        .unwrap_or(1) as usize;
    
    let j = crate::lib_registry::get_arg(vm, 2)
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
    let s = crate::lib_registry::require_arg(vm, 0, "utf8.offset")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'utf8.offset' (string expected)".to_string())?;
    
    let n = crate::lib_registry::require_arg(vm, 1, "utf8.offset")?
        .as_integer()
        .ok_or_else(|| "bad argument #2 to 'utf8.offset' (number expected)".to_string())?;
    
    let i = crate::lib_registry::get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(if n >= 0 { 1 } else { (s.as_str().len() + 1) as i64 }) as usize;
    
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
