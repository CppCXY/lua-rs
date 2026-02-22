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
        // With our UTF-8 char-based pattern matching, each character is a
        // Unicode codepoint, so '.' already matches any single character.
        // This is the correct equivalent of Lua's byte-based charpattern.
        vm.create_string(".")
    });

    module
}

/// Helper: translate a relative string position (negative means back from end)
/// Matches C Lua's u_posrelat
#[inline]
fn u_posrelat(pos: i64, len: usize) -> i64 {
    if pos >= 0 {
        pos
    } else {
        let upos = (-pos) as usize;
        if upos > len { 0 } else { len as i64 + pos + 1 }
    }
}

#[inline]
fn iscont(b: u8) -> bool {
    (b & 0xC0) == 0x80
}

const MAXUNICODE: u32 = 0x10FFFF;
const MAXUTF: u32 = 0x7FFFFFFF;

/// Decode one UTF-8 sequence from byte slice. Returns (codepoint, byte_length).
/// If strict is true, rejects surrogates and values > MAXUNICODE.
fn decode_utf8(s: &[u8], strict: bool) -> Result<(u32, usize), String> {
    if s.is_empty() {
        return Err("invalid UTF-8 code".to_string());
    }
    let c = s[0];
    if c < 0x80 {
        return Ok((c as u32, 1));
    }
    // Determine expected length and limits
    static LIMITS: [u32; 6] = [u32::MAX, 0x80, 0x800, 0x10000, 0x200000, 0x4000000];
    let mut count = 0usize;
    let mut mask = c;
    while mask & 0x40 != 0 {
        count += 1;
        mask <<= 1;
    }
    if count == 0 || count > 5 {
        return Err("invalid UTF-8 code".to_string());
    }
    // First byte contributes: c & ((1 << (7-count)) - 1) = c & (0x7F >> count)
    let mut res = (c & (0x7F >> count)) as u32;
    for i in 1..=count {
        if i >= s.len() || !iscont(s[i]) {
            return Err("invalid UTF-8 code".to_string());
        }
        res = (res << 6) | (s[i] & 0x3F) as u32;
    }
    if res > MAXUTF || res < LIMITS[count] {
        return Err("invalid UTF-8 code".to_string());
    }
    if strict && (res > MAXUNICODE || (0xD800..=0xDFFF).contains(&res)) {
        return Err("invalid UTF-8 code".to_string());
    }
    Ok((res, count + 1))
}

/// Encode a codepoint into extended UTF-8 bytes (supports up to 0x7FFFFFFF)
fn encode_utf8_extended(x: u32) -> Vec<u8> {
    if x < 0x80 {
        return vec![x as u8];
    }
    let mut bytes = Vec::new();
    let mut x = x;
    let mut mfb: u32 = 0x3f;
    loop {
        bytes.push(0x80 | (x & 0x3f) as u8);
        x >>= 6;
        mfb >>= 1;
        if x <= mfb {
            break;
        }
    }
    bytes.push(((!mfb << 1) | x) as u8);
    bytes.reverse();
    bytes
}

fn utf8_len(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'len' (string expected)".to_string()))?;
    let bytes: &[u8] = if let Some(s) = s_value.as_str() {
        s.as_bytes()
    } else if let Some(b) = s_value.as_binary() {
        b
    } else {
        return Err(l.error("bad argument #1 to 'len' (string expected)".to_string()));
    };

    let len = bytes.len();
    let lax = l.get_arg(4).and_then(|v| v.as_bool()).unwrap_or(false);

    // Get byte positions using u_posrelat (like C Lua)
    let i_raw = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(1);
    let j_raw = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(-1);

    let mut posi = u_posrelat(i_raw, len);
    let mut posj = u_posrelat(j_raw, len);

    // luaL_argcheck: 1 <= posi && --posi <= len
    if posi < 1 || {
        posi -= 1;
        posi
    } > len as i64
    {
        return Err(
            l.error("bad argument #2 to 'len' (initial position out of bounds)".to_string())
        );
    }
    // luaL_argcheck: --posj < len
    posj -= 1;
    if posj >= len as i64 {
        return Err(l.error("bad argument #3 to 'len' (final position out of bounds)".to_string()));
    }

    let mut n: i64 = 0;
    let mut pos = posi as usize;
    let end = posj as usize;

    while pos <= end {
        if pos >= bytes.len() {
            break;
        }
        match decode_utf8(&bytes[pos..], !lax) {
            Ok((_code, char_len)) => {
                pos += char_len;
                n += 1;
            }
            Err(_) => {
                // Conversion error: return nil + error position
                l.push_value(LuaValue::nil())?;
                l.push_value(LuaValue::integer(pos as i64 + 1))?;
                return Ok(2);
            }
        }
    }

    l.push_value(LuaValue::integer(n))?;
    Ok(1)
}

fn utf8_char(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();

    let mut result_bytes: Vec<u8> = Vec::new();
    for arg in args {
        if let Some(code) = arg.as_integer() {
            if code < 0 || code as u32 > MAXUTF {
                return Err(l.error("bad argument to 'char' (value out of range)".to_string()));
            }
            let code = code as u32;
            if let Some(ch) = char::from_u32(code) {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                result_bytes.extend_from_slice(s.as_bytes());
            } else {
                // Extended encoding for surrogates and values > 0x10FFFF
                result_bytes.extend_from_slice(&encode_utf8_extended(code));
            }
        } else {
            return Err(l.error("bad argument to 'char' (number expected)".to_string()));
        }
    }

    // Try to create as UTF-8 string, fallback to binary
    let val = match String::from_utf8(result_bytes.clone()) {
        Ok(s) => l.create_string(&s)?,
        Err(_) => l.create_binary(result_bytes)?,
    };
    l.push_value(val)?;
    Ok(1)
}

/// utf8.codes(s [, lax]) - Returns an iterator for UTF-8 characters
fn utf8_codes(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'codes' (string expected)".to_string()))?;
    if !s_value.is_string() && s_value.as_binary().is_none() {
        return Err(l.error("bad argument #1 to 'codes' (string expected)".to_string()));
    }

    // Create state table: {string = s, position = 0}
    let state_table = l.create_table(2, 0)?;
    let string_key = LuaValue::integer(1);
    let position_key = LuaValue::integer(2);

    if let Some(table) = state_table.as_table_mut() {
        table.raw_set(&string_key, s_value);
        table.raw_set(&position_key, LuaValue::integer(0));
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

    // Extract string and position from state table
    let Some(table) = t_value.as_table() else {
        return Err(l.error("utf8.codes iterator: invalid state".to_string()));
    };

    let Some(s_val) = table.raw_geti(string_key) else {
        return Err(l.error("utf8.codes iterator: string not found".to_string()));
    };

    // Accept both string and binary
    let bytes: &[u8] = if let Some(s) = s_val.as_str() {
        s.as_bytes()
    } else if let Some(b) = s_val.as_binary() {
        b
    } else {
        return Err(l.error("utf8.codes iterator: invalid string".to_string()));
    };

    let lax = false; // TODO: support lax codes iterator

    let pos = table
        .raw_geti(position_key)
        .and_then(|v| v.as_integer())
        .unwrap_or(0) as usize;

    if pos >= bytes.len() {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    }

    // Decode next UTF-8 character using decode_utf8
    let remaining = &bytes[pos..];
    match decode_utf8(remaining, !lax) {
        Ok((code_point, char_len)) => {
            // Update position in the state table
            l.raw_seti(
                &t_value,
                position_key,
                LuaValue::integer((pos + char_len) as i64),
            );

            l.push_value(LuaValue::integer((pos + 1) as i64))?; // 1-based position
            l.push_value(LuaValue::integer(code_point as i64))?;
            Ok(2)
        }
        Err(e) => Err(l.error(e)),
    }
}

/// utf8.codepoint(s [, i [, j [, lax]]]) - Returns code points of characters
/// Follows C Lua's codepoint() using u_posrelat for indices
fn utf8_codepoint(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'codepoint' (string expected)".to_string()))?;
    // Accept both string and binary values
    let bytes: &[u8] = if let Some(s) = s_value.as_str() {
        s.as_bytes()
    } else if let Some(b) = s_value.as_binary() {
        b
    } else {
        return Err(l.error("bad argument #1 to 'codepoint' (string expected)".to_string()));
    };

    let len = bytes.len();

    let i_raw = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(1);
    let posi = u_posrelat(i_raw, len);
    let j_raw = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(posi);
    let pose = u_posrelat(j_raw, len);
    let lax = l.get_arg(4).and_then(|v| v.as_bool()).unwrap_or(false);

    // luaL_argcheck
    if posi < 1 {
        return Err(l.error("bad argument #2 to 'codepoint' (out of bounds)".to_string()));
    }
    if pose > len as i64 {
        return Err(l.error("bad argument #3 to 'codepoint' (out of bounds)".to_string()));
    }
    if posi > pose {
        return Ok(0); // empty interval
    }

    let mut count = 0;
    let se = pose as usize; // end byte (1-based inclusive → byte index)
    let mut pos = (posi - 1) as usize; // 0-based start

    while pos < se {
        let remaining = &bytes[pos..];
        // Decode one UTF-8 character
        let (code, char_len) = decode_utf8(remaining, !lax).map_err(|e| l.error(e))?;
        l.push_value(LuaValue::integer(code as i64))?;
        count += 1;
        pos += char_len;
    }

    Ok(count)
}

/// utf8.offset(s, n [, i]) - Returns byte position where n-th character
/// counting from position 'i' starts and ends; 0 means character at 'i'.
/// Follows C Lua 5.5's byteoffset() exactly.
fn utf8_offset(l: &mut LuaState) -> LuaResult<usize> {
    let s_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'offset' (string expected)".to_string()))?;
    let bytes: &[u8] = if let Some(s) = s_value.as_str() {
        s.as_bytes()
    } else if let Some(b) = s_value.as_binary() {
        b
    } else {
        return Err(l.error("bad argument #1 to 'offset' (string expected)".to_string()));
    };

    let n_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'offset' (number expected)".to_string()))?;
    let Some(n) = n_value.as_integer() else {
        return Err(l.error("bad argument #2 to 'offset' (number expected)".to_string()));
    };

    let len = bytes.len();

    // Default i: if n >= 0 then 1 else len+1
    let default_i = if n >= 0 { 1i64 } else { len as i64 + 1 };
    let i_raw = l
        .get_arg(3)
        .and_then(|v| v.as_integer())
        .unwrap_or(default_i);
    let mut posi = u_posrelat(i_raw, len);

    // luaL_argcheck: 1 <= posi && --posi <= len
    if posi < 1 || {
        posi -= 1;
        posi
    } > len as i64
    {
        return Err(l.error("bad argument #3 to 'offset' (position out of bounds)".to_string()));
    }

    let mut n = n;

    if n == 0 {
        // Find beginning of current byte sequence
        while posi > 0 && (posi as usize) < len && iscont(bytes[posi as usize]) {
            posi -= 1;
        }
    } else {
        if (posi as usize) < len && iscont(bytes[posi as usize]) {
            return Err(l.error("initial position is a continuation byte".to_string()));
        }
        if n < 0 {
            while n < 0 && posi > 0 {
                // Find beginning of previous character
                loop {
                    posi -= 1;
                    if posi <= 0 || !iscont(bytes[posi as usize]) {
                        break;
                    }
                }
                n += 1;
            }
        } else {
            n -= 1; // do not move for 1st character
            while n > 0 && (posi as usize) < len {
                // Find beginning of next character
                loop {
                    posi += 1;
                    if (posi as usize) >= len || !iscont(bytes[posi as usize]) {
                        break;
                    }
                }
                n -= 1;
            }
        }
    }

    if n != 0 {
        // Did not find given character - return nil
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    }

    // Push initial position (1-based)
    l.push_value(LuaValue::integer(posi + 1))?;

    // Find end position of this character
    let pos_usize = posi as usize;
    if pos_usize < len && (bytes[pos_usize] & 0x80) != 0 {
        // Multi-byte character
        if iscont(bytes[pos_usize]) {
            return Err(l.error("initial position is a continuation byte".to_string()));
        }
        let mut end_pos = posi;
        while (end_pos as usize + 1) < len && iscont(bytes[end_pos as usize + 1]) {
            end_pos += 1;
        }
        // Push final position (1-based)
        l.push_value(LuaValue::integer(end_pos + 1))?;
    } else {
        // One-byte character (or position == len+1): final position is the initial one
        l.push_value(LuaValue::integer(posi + 1))?;
    }

    Ok(2)
}
