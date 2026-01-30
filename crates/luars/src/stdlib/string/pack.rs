// String pack/unpack functions
// Implements: string.pack, string.unpack, string.packsize
//
// Supports Lua 5.4+ binary data packing:
// - b/B: signed/unsigned byte (1 byte)
// - h/H: signed/unsigned short (2 bytes)
// - i/I: signed/unsigned int (4 bytes)
// - l/L: signed/unsigned long (4 bytes, same as i/I)
// - j: lua_Integer (8 bytes, i64)
// - T: size_t (8 bytes on 64-bit platforms)
// - f: float (4 bytes)
// - d: double (8 bytes)
// - n: lua_Number (8 bytes, f64)
// - z: zero-terminated string
// - cn: fixed-length string of n bytes
// All formats use little-endian byte order by default

use crate::LuaValue;
use crate::lua_vm::{LuaResult, LuaState};

/// string.pack(fmt, v1, v2, ...) - Pack values into binary string
pub fn string_pack(l: &mut LuaState) -> LuaResult<usize> {
    let fmt_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'pack' (string expected)".to_string()))?;

    let Some(fmt_str) = fmt_value.as_str() else {
        return Err(l.error("bad argument #1 to 'pack' (string expected)".to_string()));
    };
    let fmt_str = fmt_str.to_string();

    let argc = l.arg_count();
    let mut result = Vec::new();
    let mut value_idx = 2; // Start from argument 2 (after format string)
    let mut chars = fmt_str.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue, // Skip whitespace

            'b' => {
                // signed byte
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;
                result.push((n & 0xFF) as u8);
                value_idx += 1;
            }

            'B' => {
                // unsigned byte
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;
                result.push((n & 0xFF) as u8);
                value_idx += 1;
            }

            'h' => {
                // signed short (2 bytes, little-endian)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as i16;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }

            'H' => {
                // unsigned short (2 bytes, little-endian)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as u16;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }

            'i' | 'l' => {
                // signed int - check for size suffix (i[n] where n is 1-16)
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                
                let size = if size_str.is_empty() {
                    4 // default int size
                } else {
                    let n: usize = size_str.parse().map_err(|_| {
                        l.error("bad argument to 'pack' (invalid size)".to_string())
                    })?;
                    if n < 1 || n > 16 {
                        return Err(l.error("integral size out of limits".to_string()));
                    }
                    n
                };

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let mut val = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;
                
                // Pack the value as signed integer with specified size
                for _ in 0..size {
                    result.push((val & 0xFF) as u8);
                    val >>= 8;
                }
                value_idx += 1;
            }

            'I' | 'L' => {
                // unsigned int - check for size suffix (I[n] where n is 1-16)
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                
                let size = if size_str.is_empty() {
                    4 // default int size
                } else {
                    let n: usize = size_str.parse().map_err(|_| {
                        l.error("bad argument to 'pack' (invalid size)".to_string())
                    })?;
                    if n < 1 || n > 16 {
                        return Err(l.error("integral size out of limits".to_string()));
                    }
                    n
                };

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let mut val = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as u64;
                
                // Pack the value as unsigned integer with specified size
                for _ in 0..size {
                    result.push((val & 0xFF) as u8);
                    val >>= 8;
                }
                value_idx += 1;
            }

            'f' => {
                // float (4 bytes, little-endian)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as f32;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }

            'd' => {
                // double (8 bytes, little-endian)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }

            'j' => {
                // lua_Integer (8 bytes, i64, little-endian)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }

            'T' => {
                // size_t (8 bytes on 64-bit, little-endian)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as u64;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }

            'n' => {
                // lua_Number (8 bytes, f64, little-endian) - same as 'd'
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;
                result.extend_from_slice(&n.to_le_bytes());
                value_idx += 1;
            }

            'z' => {
                // zero-terminated string
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let s_value = l.get_arg(value_idx).ok_or_else(|| {
                    l.error("bad argument to 'pack' (string expected)".to_string())
                })?;
                let Some(s_str) = s_value.as_str() else {
                    return Err(l.error("bad argument to 'pack' (string expected)".to_string()));
                };
                result.extend_from_slice(s_str.as_bytes());
                result.push(0); // null terminator
                value_idx += 1;
            }

            'c' => {
                // fixed-length string - read size
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                let size: usize = size_str
                    .parse()
                    .map_err(|_| l.error("bad argument to 'pack' (invalid size)".to_string()))?;

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let s_value = l.get_arg(value_idx).ok_or_else(|| {
                    l.error("bad argument to 'pack' (string expected)".to_string())
                })?;
                let Some(s_str) = s_value.as_str() else {
                    return Err(l.error("bad argument to 'pack' (string expected)".to_string()));
                };
                let bytes = s_str.as_bytes();
                result.extend_from_slice(&bytes[..size.min(bytes.len())]);
                // Pad with zeros if needed
                for _ in bytes.len()..size {
                    result.push(0);
                }
                value_idx += 1;
            }

            'x' => {
                // padding byte (zero)
                result.push(0);
            }

            '<' | '>' | '=' | '!' => {
                // endianness/alignment modifiers - we ignore these for now
                // (always use little-endian)
            }

            _ => {
                return Err(l.error(format!(
                    "bad argument to 'pack' (invalid format option '{}')",
                    ch
                )));
            }
        }
    }

    // Create a binary value from bytes - Lua strings can contain arbitrary binary data
    let packed_val = l.create_binary(result)?;
    l.push_value(packed_val)?;
    Ok(1)
}

/// string.packsize(fmt) - Return size of packed data
pub fn string_packsize(l: &mut LuaState) -> LuaResult<usize> {
    let fmt_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'packsize' (string expected)".to_string()))?;

    let Some(fmt_str) = fmt_value.as_str() else {
        return Err(l.error("bad argument #1 to 'packsize' (string expected)".to_string()));
    };
    let fmt_str = fmt_str.to_string();

    let mut size = 0usize;
    let mut chars = fmt_str.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue,
            'b' | 'B' => size += 1,
            'h' | 'H' => size += 2,
            
            'i' | 'I' | 'l' | 'L' => {
                // Check for size suffix (i[n] or I[n] where n is 1-16)
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                
                if size_str.is_empty() {
                    size += 4; // default int size
                } else {
                    let n: usize = size_str.parse().map_err(|_| {
                        l.error("bad argument to 'packsize' (invalid size)".to_string())
                    })?;
                    if n < 1 || n > 16 {
                        return Err(l.error("integral size out of limits".to_string()));
                    }
                    size += n;
                }
            }
            
            'f' => size += 4,
            'd' => size += 8,
            'j' | 'n' | 'T' => {
                // j: lua_Integer (8 bytes, i64)
                // n: lua_Number (8 bytes, f64)
                // T: size_t (8 bytes on 64-bit platforms)
                size += 8;
            }
            'x' => size += 1,

            'c' => {
                // fixed-length string - read size
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                let n: usize = size_str.parse().map_err(|_| {
                    l.error("bad argument to 'packsize' (invalid size)".to_string())
                })?;
                size += n;
            }

            'z' | 's' => {
                return Err(l.error("variable-length format in 'packsize'".to_string()));
            }

            '<' | '>' | '=' | '!' => {
                // endianness/alignment modifiers - ignore
            }

            _ => {
                return Err(l.error(format!(
                    "bad argument to 'packsize' (invalid format option '{}')",
                    ch
                )));
            }
        }
    }

    l.push_value(LuaValue::integer(size as i64))?;
    Ok(1)
}

/// string.unpack(fmt, s [, pos]) - Unpack binary string
pub fn string_unpack(l: &mut LuaState) -> LuaResult<usize> {
    let fmt_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'unpack' (string expected)".to_string()))?;

    let Some(fmt_str) = fmt_value.as_str() else {
        return Err(l.error("bad argument #1 to 'unpack' (string expected)".to_string()));
    };
    let fmt_str = fmt_str.to_string();

    let s_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'unpack' (string expected)".to_string()))?;

    let Some(s_str) = s_value.as_str() else {
        return Err(l.error("bad argument #2 to 'unpack' (string expected)".to_string()));
    };
    let bytes = s_str.as_bytes().to_vec();

    let pos = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1) as usize;

    if pos < 1 {
        return Err(l.error("bad argument #3 to 'unpack' (position out of range)".to_string()));
    }

    let mut idx = pos - 1; // Convert to 0-based
    let mut results = Vec::new();
    let mut chars = fmt_str.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue,

            'b' => {
                if idx >= bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                results.push(LuaValue::integer(bytes[idx] as i8 as i64));
                idx += 1;
            }

            'B' => {
                if idx >= bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                results.push(LuaValue::integer(bytes[idx] as i64));
                idx += 1;
            }

            'h' => {
                if idx + 2 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val = i16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
                results.push(LuaValue::integer(val as i64));
                idx += 2;
            }

            'H' => {
                if idx + 2 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val = u16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
                results.push(LuaValue::integer(val as i64));
                idx += 2;
            }

            'i' | 'l' => {
                // signed int - check for size suffix (i[n] where n is 1-16)
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                
                let size = if size_str.is_empty() {
                    4 // default int size
                } else {
                    let n: usize = size_str.parse().map_err(|_| {
                        l.error("bad argument to 'unpack' (invalid size)".to_string())
                    })?;
                    if n < 1 || n > 16 {
                        return Err(l.error("integral size out of limits".to_string()));
                    }
                    n
                };

                if idx + size > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                
                // Unpack signed integer with specified size
                let mut val: i64 = 0;
                for i in (0..size).rev() {
                    val = (val << 8) | (bytes[idx + i] as i64);
                }
                // Sign extend if the highest bit is set
                if size < 8 && (bytes[idx + size - 1] & 0x80) != 0 {
                    val |= !0i64 << (size * 8);
                }
                results.push(LuaValue::integer(val));
                idx += size;
            }

            'I' | 'L' => {
                // unsigned int - check for size suffix (I[n] where n is 1-16)
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                
                let size = if size_str.is_empty() {
                    4 // default int size
                } else {
                    let n: usize = size_str.parse().map_err(|_| {
                        l.error("bad argument to 'unpack' (invalid size)".to_string())
                    })?;
                    if n < 1 || n > 16 {
                        return Err(l.error("integral size out of limits".to_string()));
                    }
                    n
                };

                if idx + size > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                
                // Unpack unsigned integer with specified size
                let mut val: u64 = 0;
                for i in (0..size).rev() {
                    val = (val << 8) | (bytes[idx + i] as u64);
                }
                results.push(LuaValue::integer(val as i64));
                idx += size;
            }

            'f' => {
                if idx + 4 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val = f32::from_le_bytes([
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                ]);
                results.push(LuaValue::number(val as f64));
                idx += 4;
            }

            'd' => {
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val = f64::from_le_bytes([
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                    bytes[idx + 4],
                    bytes[idx + 5],
                    bytes[idx + 6],
                    bytes[idx + 7],
                ]);
                results.push(LuaValue::number(val));
                idx += 8;
            }

            'j' => {
                // lua_Integer (8 bytes, i64, little-endian)
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val = i64::from_le_bytes([
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                    bytes[idx + 4],
                    bytes[idx + 5],
                    bytes[idx + 6],
                    bytes[idx + 7],
                ]);
                results.push(LuaValue::integer(val));
                idx += 8;
            }

            'T' => {
                // size_t (8 bytes on 64-bit, little-endian)
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val = u64::from_le_bytes([
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                    bytes[idx + 4],
                    bytes[idx + 5],
                    bytes[idx + 6],
                    bytes[idx + 7],
                ]);
                results.push(LuaValue::integer(val as i64));
                idx += 8;
            }

            'n' => {
                // lua_Number (8 bytes, f64, little-endian) - same as 'd'
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val = f64::from_le_bytes([
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                    bytes[idx + 4],
                    bytes[idx + 5],
                    bytes[idx + 6],
                    bytes[idx + 7],
                ]);
                results.push(LuaValue::number(val));
                idx += 8;
            }

            'z' => {
                // zero-terminated string
                let start = idx;
                while idx < bytes.len() && bytes[idx] != 0 {
                    idx += 1;
                }
                if idx >= bytes.len() {
                    return Err(l.error("unfinished string in data".to_string()));
                }
                // Create binary value for the extracted bytes
                let binary_val = l.create_binary(bytes[start..idx].to_vec())?;
                results.push(binary_val);
                idx += 1; // Skip null terminator
            }

            'c' => {
                // fixed-length string - read size
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                let size: usize = size_str
                    .parse()
                    .map_err(|_| l.error("bad argument to 'unpack' (invalid size)".to_string()))?;

                if idx + size > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                // Create binary value for the fixed-length data
                let binary_val = l.create_binary(bytes[idx..idx + size].to_vec())?;
                results.push(binary_val);
                idx += size;
            }

            'x' => {
                // padding byte - skip
                if idx >= bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                idx += 1;
            }

            '<' | '>' | '=' | '!' => {
                // endianness/alignment modifiers - ignore
            }

            _ => {
                return Err(l.error(format!(
                    "bad argument to 'unpack' (invalid format option '{}')",
                    ch
                )));
            }
        }
    }

    // Push all results
    let result_count = results.len();
    for result in results {
        l.push_value(result)?;
    }

    // Also push the next position
    l.push_value(LuaValue::integer(idx as i64 + 1))?; // Convert back to 1-based

    Ok(result_count + 1) // Return number of results + position
}
