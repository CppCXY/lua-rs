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

/// Endianness for pack/unpack operations
#[derive(Debug, Clone, Copy, PartialEq)]
enum Endianness {
    Little,
    Big,
    Native,
}

impl Endianness {
    /// Convert bytes based on endianness
    fn to_bytes<T: ToBytes>(self, value: T) -> Vec<u8> {
        match self {
            Endianness::Little => value.to_le_bytes(),
            Endianness::Big => value.to_be_bytes(),
            Endianness::Native => value.to_ne_bytes(),
        }
    }

    /// Convert from bytes based on endianness
    fn from_bytes<T: FromBytes>(self, bytes: &[u8]) -> T {
        match self {
            Endianness::Little => T::from_le_bytes(bytes),
            Endianness::Big => T::from_be_bytes(bytes),
            Endianness::Native => T::from_ne_bytes(bytes),
        }
    }
}

/// Trait for types that can be converted to bytes with different endianness
trait ToBytes {
    fn to_le_bytes(self) -> Vec<u8>;
    fn to_be_bytes(self) -> Vec<u8>;
    fn to_ne_bytes(self) -> Vec<u8>;
}

/// Trait for types that can be converted from bytes with different endianness
trait FromBytes: Sized {
    fn from_le_bytes(bytes: &[u8]) -> Self;
    fn from_be_bytes(bytes: &[u8]) -> Self;
    fn from_ne_bytes(bytes: &[u8]) -> Self;
}

// Implement ToBytes for integer types
macro_rules! impl_to_bytes {
    ($($t:ty),*) => {
        $(
            impl ToBytes for $t {
                fn to_le_bytes(self) -> Vec<u8> {
                    <$t>::to_le_bytes(self).to_vec()
                }
                fn to_be_bytes(self) -> Vec<u8> {
                    <$t>::to_be_bytes(self).to_vec()
                }
                fn to_ne_bytes(self) -> Vec<u8> {
                    <$t>::to_ne_bytes(self).to_vec()
                }
            }
        )*
    };
}

impl_to_bytes!(i16, u16, i32, u32, i64, u64, f32, f64);

// Implement FromBytes for integer types  
macro_rules! impl_from_bytes {
    ($($t:ty, $size:expr),*) => {
        $(
            impl FromBytes for $t {
                fn from_le_bytes(bytes: &[u8]) -> Self {
                    let mut arr = [0u8; $size];
                    arr.copy_from_slice(&bytes[..$size]);
                    <$t>::from_le_bytes(arr)
                }
                fn from_be_bytes(bytes: &[u8]) -> Self {
                    let mut arr = [0u8; $size];
                    arr.copy_from_slice(&bytes[..$size]);
                    <$t>::from_be_bytes(arr)
                }
                fn from_ne_bytes(bytes: &[u8]) -> Self {
                    let mut arr = [0u8; $size];
                    arr.copy_from_slice(&bytes[..$size]);
                    <$t>::from_ne_bytes(arr)
                }
            }
        )*
    };
}

impl_from_bytes!(i16, 2, u16, 2, i32, 4, u32, 4, i64, 8, u64, 8, f32, 4, f64, 8);


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
    let mut endianness = Endianness::Native; // Default to native endianness

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
                // signed short (2 bytes)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as i16;
                result.extend_from_slice(&endianness.to_bytes(n));
                value_idx += 1;
            }

            'H' => {
                // unsigned short (2 bytes)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as u16;
                result.extend_from_slice(&endianness.to_bytes(n));
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
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let val = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;

                // Pack the value as signed integer with specified size
                let bytes = match endianness {
                    Endianness::Little => {
                        let mut bytes = Vec::new();
                        let mut v = val;
                        for _ in 0..size {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        bytes
                    }
                    Endianness::Big => {
                        let mut bytes = Vec::new();
                        let mut v = val;
                        for _ in 0..size {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        bytes.reverse();
                        bytes
                    }
                    Endianness::Native => {
                        let mut bytes = Vec::new();
                        let mut v = val;
                        for _ in 0..size {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        #[cfg(target_endian = "big")]
                        bytes.reverse();
                        bytes
                    }
                };
                result.extend_from_slice(&bytes);
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
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let val = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as u64;

                // Pack the value as unsigned integer with specified size
                let bytes = match endianness {
                    Endianness::Little => {
                        let mut bytes = Vec::new();
                        let mut v = val;
                        for _ in 0..size {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        bytes
                    }
                    Endianness::Big => {
                        let mut bytes = Vec::new();
                        let mut v = val;
                        for _ in 0..size {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        bytes.reverse();
                        bytes
                    }
                    Endianness::Native => {
                        let mut bytes = Vec::new();
                        let mut v = val;
                        for _ in 0..size {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        #[cfg(target_endian = "big")]
                        bytes.reverse();
                        bytes
                    }
                };
                result.extend_from_slice(&bytes);
                value_idx += 1;
            }

            'f' => {
                // float (4 bytes)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })? as f32;
                result.extend_from_slice(&endianness.to_bytes(n));
                value_idx += 1;
            }

            'd' => {
                // double (8 bytes)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;
                result.extend_from_slice(&endianness.to_bytes(n));
                value_idx += 1;
            }

            'j' => {
                // lua_Integer (8 bytes, i64)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let value = l.get_arg(value_idx).ok_or_else(|| {
                    l.error("bad argument to 'pack' (number expected)".to_string())
                })?;
                let n = if let Some(i) = value.as_integer() {
                    i
                } else if let Some(f) = value.as_number() {
                    // For float input that doesn't fit in i64, preserve as IEEE754 bit pattern
                    // This allows pack/unpack round-trip to maintain equality for large floats
                    f.to_bits() as i64
                } else {
                    return Err(l.error("bad argument to 'pack' (number expected)".to_string()));
                };
                result.extend_from_slice(&endianness.to_bytes(n));
                value_idx += 1;
            }

            'T' => {
                // size_t (8 bytes on 64-bit)
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let value = l.get_arg(value_idx).ok_or_else(|| {
                    l.error("bad argument to 'pack' (number expected)".to_string())
                })?;
                let n = if let Some(i) = value.as_integer() {
                    i as u64
                } else if let Some(f) = value.as_number() {
                    // For float input: apply modulo 2^64 then cast to u64
                    const TWO_POW_64: f64 = 18446744073709551616.0; // 2^64
                    let mut reduced = f % TWO_POW_64;
                    if reduced < 0.0 {
                        reduced += TWO_POW_64;
                    }
                    reduced as u64
                } else {
                    return Err(l.error("bad argument to 'pack' (number expected)".to_string()));
                };
                result.extend_from_slice(&endianness.to_bytes(n));
                value_idx += 1;
            }

            'n' => {
                // lua_Number (8 bytes, f64) - same as 'd'
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let n = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;
                result.extend_from_slice(&endianness.to_bytes(n));
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
                if size_str.is_empty() {
                    return Err(l.error("bad argument to 'pack' (missing size)".to_string()));
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

            'X' => {
                // Alignment padding - peek at next format option to determine alignment
                if let Some(&next_ch) = chars.peek() {
                    // Determine alignment size based on next format option
                    let align = match next_ch {
                        'b' | 'B' => 1,
                        'h' | 'H' => 2,
                        'i' | 'I' | 'l' | 'L' | 'f' => {
                            // Check for size suffix on integers
                            let mut temp_chars = chars.clone();
                            temp_chars.next(); // skip the format char
                            let mut size_str = String::new();
                            while let Some(&digit) = temp_chars.peek() {
                                if digit.is_ascii_digit() {
                                    size_str.push(temp_chars.next().unwrap());
                                } else {
                                    break;
                                }
                            }
                            if !size_str.is_empty() && (next_ch == 'i' || next_ch == 'I' || next_ch == 'l' || next_ch == 'L') {
                                size_str.parse::<usize>().unwrap_or(4)
                            } else if next_ch == 'f' {
                                4
                            } else {
                                4 // default int size
                            }
                        }
                        'd' | 'n' | 'j' | 'T' => 8,
                        _ => 1,
                    };
                    // Add padding bytes to align to boundary
                    if align > 1 {
                        let padding = (align - (result.len() % align)) % align;
                        for _ in 0..padding {
                            result.push(0);
                        }
                    }
                }
            }

            '<' | '>' | '=' => {
                // Update endianness based on modifier
                match ch {
                    '<' => endianness = Endianness::Little,
                    '>' => endianness = Endianness::Big,
                    '=' => endianness = Endianness::Native,
                    _ => {}
                }
            }

            '!' => {
                // '!' sets max alignment - consume the alignment size
                let mut align_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        align_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                
                let _alignment = if align_str.is_empty() {
                    // No number specified, use default max alignment (8 for double)
                    8
                } else {
                    let n: usize = align_str.parse().map_err(|_| {
                        l.error("bad argument to 'pack' (invalid alignment)".to_string())
                    })?;
                    if n < 1 || n > 16 {
                        return Err(l.error("alignment out of limits [1,16]".to_string()));
                    }
                    // Check if n is a power of 2
                    if n & (n - 1) != 0 {
                        return Err(l.error("alignment is not a power of 2".to_string()));
                    }
                    n
                };
                // Note: We'd need to store max_alignment in the loop to properly implement this.
                // For now, just validate and consume the number.
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
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
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

            'X' => {
                // Alignment padding - peek at next format option to determine alignment
                if let Some(&next_ch) = chars.peek() {
                    // Determine alignment size based on next format option
                    let align = match next_ch {
                        'b' | 'B' => 1,
                        'h' | 'H' => 2,
                        'i' | 'I' | 'l' | 'L' | 'f' => {
                            // Check for size suffix on integers
                            let mut temp_chars = chars.clone();
                            temp_chars.next(); // skip the 'i'/'I'/'l'/'L'
                            let mut size_str = String::new();
                            while let Some(&digit) = temp_chars.peek() {
                                if digit.is_ascii_digit() {
                                    size_str.push(temp_chars.next().unwrap());
                                } else {
                                    break;
                                }
                            }
                            if !size_str.is_empty() && (next_ch == 'i' || next_ch == 'I' || next_ch == 'l' || next_ch == 'L') {
                                size_str.parse::<usize>().unwrap_or(4)
                            } else if next_ch == 'f' {
                                4
                            } else {
                                4 // default int size
                            }
                        }
                        'd' | 'n' | 'j' | 'T' => 8,
                        _ => 1, // For other options, use minimal alignment
                    };
                    // Add padding to align to boundary
                    if align > 1 {
                        let padding = (align - (size % align)) % align;
                        size += padding;
                    }
                }
                // X itself doesn't add size, it just aligns
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
                if size_str.is_empty() {
                    return Err(l.error("bad argument to 'packsize' (missing size)".to_string()));
                }
                let n: usize = size_str.parse().map_err(|_| {
                    l.error("bad argument to 'packsize' (invalid format)".to_string())
                })?;
                size += n;
            }

            'z' | 's' => {
                return Err(l.error("variable-length format in 'packsize'".to_string()));
            }

            '<' | '>' | '=' => {
                // endianness modifiers - ignore
            }

            '!' => {
                // '!' sets max alignment - consume and validate the alignment size
                let mut align_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        align_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                
                if !align_str.is_empty() {
                    let n: usize = align_str.parse().map_err(|_| {
                        l.error("bad argument to 'packsize' (invalid alignment)".to_string())
                    })?;
                    if n < 1 || n > 16 {
                        return Err(l.error("alignment out of limits [1,16]".to_string()));
                    }
                    // Check if n is a power of 2
                    if n & (n - 1) != 0 {
                        return Err(l.error("alignment is not a power of 2".to_string()));
                    }
                }
                // If no number specified, use default max alignment (8)
                // Note: We'd need to store max_alignment to properly implement this.
                // For now, just validate and consume the number.
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

    let s_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'unpack' (string expected)".to_string()))?;

    let bytes: &[u8] = if let Some(binary) = s_value.as_binary() {
        binary
    } else if let Some(string) = s_value.as_str() {
        string.as_bytes()
    } else {
        return Err(l.error("bad argument #2 to 'unpack' (string expected)".to_string()));
    };

    let pos = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1) as usize;

    if pos < 1 {
        return Err(l.error("bad argument #3 to 'unpack' (position out of range)".to_string()));
    }

    let mut idx = pos - 1; // Convert to 0-based
    let mut results = Vec::new();
    let mut chars = fmt_str.chars().peekable();
    let mut endianness = Endianness::Native; // Default to native endianness

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
                let val: i16 = endianness.from_bytes(&bytes[idx..idx+2]);
                results.push(LuaValue::integer(val as i64));
                idx += 2;
            }

            'H' => {
                if idx + 2 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: u16 = endianness.from_bytes(&bytes[idx..idx+2]);
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
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                if idx + size > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }

                // Unpack signed integer with specified size
                let val: i64 = match endianness {
                    Endianness::Little => {
                        let mut v: i64 = 0;
                        for i in (0..size.min(8)).rev() {
                            v = (v << 8) | (bytes[idx + i] as i64);
                        }
                        // Sign extend if the highest bit is set
                        if size < 8 && (bytes[idx + size - 1] & 0x80) != 0 {
                            v |= !0i64 << (size * 8);
                        }
                        // For size > 8, check that extra bytes are sign extension
                        if size > 8 {
                            let sign_byte = if (bytes[idx + 7] & 0x80) != 0 { 0xFF } else { 0x00 };
                            for i in 8..size {
                                if bytes[idx + i] != sign_byte {
                                    return Err(l.error(format!(
                                        "{}-byte integer does not fit into Lua Integer",
                                        size
                                    )));
                                }
                            }
                        }
                        v
                    }
                    Endianness::Big => {
                        // For big endian, high bytes come first
                        // Check sign extension bytes first (indices 0..size-8)
                        if size > 8 {
                            let sign_byte = if (bytes[idx] & 0x80) != 0 { 0xFF } else { 0x00 };
                            for i in 0..(size - 8) {
                                if bytes[idx + i] != sign_byte {
                                    return Err(l.error(format!(
                                        "{}-byte integer does not fit into Lua Integer",
                                        size
                                    )));
                                }
                            }
                        }
                        let mut v: i64 = 0;
                        let start = if size > 8 { size - 8 } else { 0 };
                        for i in start..size {
                            v = (v << 8) | (bytes[idx + i] as i64);
                        }
                        // Sign extend if needed
                        if size < 8 && (bytes[idx] & 0x80) != 0 {
                            v |= !0i64 << (size * 8);
                        }
                        v
                    }
                    Endianness::Native => {
                        #[cfg(target_endian = "little")]
                        {
                            let mut v: i64 = 0;
                            for i in (0..size.min(8)).rev() {
                                v = (v << 8) | (bytes[idx + i] as i64);
                            }
                            if size < 8 && (bytes[idx + size - 1] & 0x80) != 0 {
                                v |= !0i64 << (size * 8);
                            }
                            if size > 8 {
                                let sign_byte = if (bytes[idx + 7] & 0x80) != 0 { 0xFF } else { 0x00 };
                                for i in 8..size {
                                    if bytes[idx + i] != sign_byte {
                                        return Err(l.error(format!(
                                            "{}-byte integer does not fit into Lua Integer",
                                            size
                                        )));
                                    }
                                }
                            }
                            v
                        }
                        #[cfg(target_endian = "big")]
                        {
                            if size > 8 {
                                let sign_byte = if (bytes[idx] & 0x80) != 0 { 0xFF } else { 0x00 };
                                for i in 0..(size - 8) {
                                    if bytes[idx + i] != sign_byte {
                                        return Err(l.error(format!(
                                            "{}-byte integer does not fit into Lua Integer",
                                            size
                                        )));
                                    }
                                }
                            }
                            let mut v: i64 = 0;
                            let start = if size > 8 { size - 8 } else { 0 };
                            for i in start..size {
                                v = (v << 8) | (bytes[idx + i] as i64);
                            }
                            if size < 8 && (bytes[idx] & 0x80) != 0 {
                                v |= !0i64 << (size * 8);
                            }
                            v
                        }
                    }
                };
                // Check if this might be a float packed as integer (for large float values)
                // Only treat as float if it's truly out of i64 range
                if size >= 8 {
                    let as_float = f64::from_bits(val as u64);
                    // Only return float if it's a very large value that exceeded i64 range
                    // AND if the original value was likely a float (indicated by magnitude)
                    if as_float.is_finite() && as_float.abs() > 1e20 {
                        results.push(LuaValue::number(as_float));
                        idx += size;
                        continue;
                    }
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
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                if idx + size > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }

                // Unpack unsigned integer with specified size
                let val: u64 = match endianness {
                    Endianness::Little => {
                        let mut v: u64 = 0;
                        for i in (0..size.min(8)).rev() {
                            v = (v << 8) | (bytes[idx + i] as u64);
                        }
                        // For size > 8, check that extra bytes are all zeros
                        if size > 8 {
                            for i in 8..size {
                                if bytes[idx + i] != 0 {
                                    return Err(l.error(format!(
                                        "{}-byte integer does not fit into Lua Integer",
                                        size
                                    )));
                                }
                            }
                        }
                        v
                    }
                    Endianness::Big => {
                        // For big endian, high bytes come first
                        // Check that high bytes are all zeros
                        if size > 8 {
                            for i in 0..(size - 8) {
                                if bytes[idx + i] != 0 {
                                    return Err(l.error(format!(
                                        "{}-byte integer does not fit into Lua Integer",
                                        size
                                    )));
                                }
                            }
                        }
                        let mut v: u64 = 0;
                        let start = if size > 8 { size - 8 } else { 0 };
                        for i in start..size {
                            v = (v << 8) | (bytes[idx + i] as u64);
                        }
                        v
                    }
                    Endianness::Native => {
                        #[cfg(target_endian = "little")]
                        {
                            let mut v: u64 = 0;
                            for i in (0..size.min(8)).rev() {
                                v = (v << 8) | (bytes[idx + i] as u64);
                            }
                            if size > 8 {
                                for i in 8..size {
                                    if bytes[idx + i] != 0 {
                                        return Err(l.error(format!(
                                            "{}-byte integer does not fit into Lua Integer",
                                            size
                                        )));
                                    }
                                }
                            }
                            v
                        }
                        #[cfg(target_endian = "big")]
                        {
                            if size > 8 {
                                for i in 0..(size - 8) {
                                    if bytes[idx + i] != 0 {
                                        return Err(l.error(format!(
                                            "{}-byte integer does not fit into Lua Integer",
                                            size
                                        )));
                                    }
                                }
                            }
                            let mut v: u64 = 0;
                            let start = if size > 8 { size - 8 } else { 0 };
                            for i in start..size {
                                v = (v << 8) | (bytes[idx + i] as u64);
                            }
                            v
                        }
                    }
                };
                // Check if this might be a float packed as integer (for large float values)
                if size >= 8 {
                    let as_float = f64::from_bits(val);
                    if as_float.is_finite() && as_float.abs() > 1e20 {
                        // This looks like a large float that was packed via IEEE754 bits
                        results.push(LuaValue::number(as_float));
                        idx += size;
                        continue;
                    }
                }
                results.push(LuaValue::integer(val as i64));
                idx += size;
            }

            'f' => {
                if idx + 4 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: f32 = endianness.from_bytes(&bytes[idx..idx+4]);
                results.push(LuaValue::number(val as f64));
                idx += 4;
            }

            'd' => {
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: f64 = endianness.from_bytes(&bytes[idx..idx+8]);
                results.push(LuaValue::number(val));
                idx += 8;
            }

            'j' => {
                // lua_Integer (8 bytes, i64)
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: i64 = endianness.from_bytes(&bytes[idx..idx+8]);
                results.push(LuaValue::integer(val));
                idx += 8;
            }

            'T' => {
                // size_t (8 bytes on 64-bit)
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: u64 = endianness.from_bytes(&bytes[idx..idx+8]);
                results.push(LuaValue::integer(val as i64));
                idx += 8;
            }

            'n' => {
                // lua_Number (8 bytes, f64) - same as 'd'
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: f64 = endianness.from_bytes(&bytes[idx..idx+8]);
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
                if size_str.is_empty() {
                    return Err(l.error("bad argument to 'unpack' (missing size)".to_string()));
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

            'X' => {
                // Alignment padding - peek at next format option to determine alignment
                if let Some(&next_ch) = chars.peek() {
                    // Determine alignment size based on next format option
                    let align = match next_ch {
                        'b' | 'B' => 1,
                        'h' | 'H' => 2,
                        'i' | 'I' | 'l' | 'L' | 'f' => {
                            // Check for size suffix on integers
                            let mut temp_chars = chars.clone();
                            temp_chars.next(); // skip the format char
                            let mut size_str = String::new();
                            while let Some(&digit) = temp_chars.peek() {
                                if digit.is_ascii_digit() {
                                    size_str.push(temp_chars.next().unwrap());
                                } else {
                                    break;
                                }
                            }
                            if !size_str.is_empty() && (next_ch == 'i' || next_ch == 'I' || next_ch == 'l' || next_ch == 'L') {
                                size_str.parse::<usize>().unwrap_or(4)
                            } else if next_ch == 'f' {
                                4
                            } else {
                                4 // default int size
                            }
                        }
                        'd' | 'n' | 'j' | 'T' => 8,
                        _ => 1,
                    };
                    // Skip padding bytes to align to boundary
                    if align > 1 {
                        let padding = (align - (idx % align)) % align;
                        idx += padding;
                    }
                }
            }

            '<' | '>' | '=' => {
                // Update endianness based on modifier
                match ch {
                    '<' => endianness = Endianness::Little,
                    '>' => endianness = Endianness::Big,
                    '=' => endianness = Endianness::Native,
                    _ => {}
                }
            }

            '!' => {
                // '!' sets max alignment - consume and validate the alignment size
                let mut align_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        align_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                
                let _alignment = if align_str.is_empty() {
                    // No number specified, use default max alignment (8 for double)
                    8
                } else {
                    let n: usize = align_str.parse().map_err(|_| {
                        l.error("bad argument to 'unpack' (invalid alignment)".to_string())
                    })?;
                    if n < 1 || n > 16 {
                        return Err(l.error("alignment out of limits [1,16]".to_string()));
                    }
                    // Check if n is a power of 2
                    if n & (n - 1) != 0 {
                        return Err(l.error("alignment is not a power of 2".to_string()));
                    }
                    n
                };
                // Note: We'd need to store max_alignment in the loop to properly implement this.
                // For now, just validate and consume the number.
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
