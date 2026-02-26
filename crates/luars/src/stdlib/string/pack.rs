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

impl_from_bytes!(
    i16, 2, u16, 2, i32, 4, u32, 4, i64, 8, u64, 8, f32, 4, f64, 8
);

// Helper function to check alignment requirements
fn check_alignment(size: usize, max_alignment: usize) -> Result<(), String> {
    // Alignment is min(size, max_alignment)
    let align = size.min(max_alignment);
    // Check if alignment is power of 2 (and > 1)
    if align > 1 && (align & (align - 1)) != 0 {
        return Err("format asks for alignment not power of 2".to_string());
    }
    Ok(())
}

// Helper function to add alignment padding to buffer
fn add_alignment_padding(buffer: &mut Vec<u8>, size: usize, max_alignment: usize) {
    if max_alignment > 1 {
        let align = size.min(max_alignment);
        if align > 1 {
            let padding = (align - (buffer.len() % align)) % align;
            for _ in 0..padding {
                buffer.push(0);
            }
        }
    }
}

// Helper function to skip alignment padding in unpack
fn skip_alignment_padding(idx: &mut usize, size: usize, max_alignment: usize) {
    if max_alignment > 1 {
        let align = size.min(max_alignment);
        if align > 1 {
            let padding = (align - (*idx % align)) % align;
            *idx += padding;
        }
    }
}

// Helper function to calculate alignment padding for size calculation
fn get_alignment_padding(current_size: usize, size: usize, max_alignment: usize) -> usize {
    if max_alignment > 1 {
        let align = size.min(max_alignment);
        if align > 1 {
            return (align - (current_size % align)) % align;
        }
    }
    0
}

// Helper function to check size overflow (for packsize)
fn checked_add_size(current: usize, add: usize) -> Result<usize, String> {
    const MAX_SIZE: usize = i64::MAX as usize;
    if current > MAX_SIZE - add {
        return Err("format result too large".to_string());
    }
    Ok(current + add)
}

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
    let mut max_alignment: usize = 1; // Default max alignment

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
                add_alignment_padding(&mut result, 2, max_alignment);
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
                add_alignment_padding(&mut result, 2, max_alignment);
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
                    if !(1..=16).contains(&n) {
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                // Check alignment requirements
                check_alignment(size, max_alignment).map_err(|e| l.error(e))?;
                // Add alignment padding
                add_alignment_padding(&mut result, size, max_alignment);

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let val = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;

                // For size < 8, check if value fits in the specified number of bytes
                if size < 8 {
                    let max_val = (1i64 << (size * 8 - 1)) - 1;
                    let min_val = -(1i64 << (size * 8 - 1));
                    if val > max_val || val < min_val {
                        return Err(l.error("integer overflow".to_string()));
                    }
                }

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
                    if !(1..=16).contains(&n) {
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                // Check alignment requirements
                check_alignment(size, max_alignment).map_err(|e| l.error(e))?;
                // Add alignment padding
                add_alignment_padding(&mut result, size, max_alignment);

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let val_i64 = l
                    .get_arg(value_idx)
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| {
                        l.error("bad argument to 'pack' (number expected)".to_string())
                    })?;

                // Check for negative values in unsigned format
                if val_i64 < 0 {
                    return Err(l.error("unsigned overflow".to_string()));
                }

                let val = val_i64 as u64;

                // For size < 8, check if value fits in the specified number of bytes
                if size < 8 {
                    let max_val = (1u64 << (size * 8)) - 1;
                    if val > max_val {
                        return Err(l.error("unsigned overflow".to_string()));
                    }
                }

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
                add_alignment_padding(&mut result, 4, max_alignment);
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
                add_alignment_padding(&mut result, 8, max_alignment);
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
                add_alignment_padding(&mut result, 8, max_alignment);
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

            'J' => {
                // unsigned lua_Integer (8 bytes, u64)
                add_alignment_padding(&mut result, 8, max_alignment);
                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let value = l.get_arg(value_idx).ok_or_else(|| {
                    l.error("bad argument to 'pack' (number expected)".to_string())
                })?;
                let n = if let Some(i) = value.as_integer() {
                    // Check for negative values
                    if i < 0 {
                        return Err(l.error("unsigned overflow".to_string()));
                    }
                    i as u64
                } else if let Some(f) = value.as_number() {
                    f.to_bits()
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
                let bytes = if let Some(s_str) = s_value.as_str() {
                    s_str.as_bytes()
                } else if let Some(bin) = s_value.as_binary() {
                    bin
                } else {
                    return Err(l.error("bad argument to 'pack' (string expected)".to_string()));
                };
                // Check for embedded null characters
                if bytes.contains(&0) {
                    return Err(l.error("string contains zeros".to_string()));
                }
                result.extend_from_slice(bytes);
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

                // Check if adding this size would overflow
                checked_add_size(result.len(), size)
                    .map_err(|_| l.error("pack result too long".to_string()))?;

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let s_value = l.get_arg(value_idx).ok_or_else(|| {
                    l.error("bad argument to 'pack' (string expected)".to_string())
                })?;
                let bytes = if let Some(s_str) = s_value.as_str() {
                    s_str.as_bytes()
                } else if let Some(bin) = s_value.as_binary() {
                    bin
                } else {
                    return Err(l.error("bad argument to 'pack' (string expected)".to_string()));
                };

                // Check if string is too long for the specified size
                if bytes.len() > size {
                    return Err(l.error("string longer than given size".to_string()));
                }

                result.extend_from_slice(bytes);
                // Pad with zeros if needed
                for _ in bytes.len()..size {
                    result.push(0);
                }
                value_idx += 1;
            }

            's' => {
                // variable-length string with size prefix
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }

                let size_bytes = if size_str.is_empty() {
                    8 // default size_t (8 bytes on 64-bit)
                } else {
                    let n: usize = size_str.parse().map_err(|_| {
                        l.error("bad argument to 'pack' (invalid size)".to_string())
                    })?;
                    if !(1..=16).contains(&n) {
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                if value_idx > argc {
                    return Err(l.error("bad argument to 'pack' (not enough values)".to_string()));
                }
                let s_value = l.get_arg(value_idx).ok_or_else(|| {
                    l.error("bad argument to 'pack' (string expected)".to_string())
                })?;
                let bytes = if let Some(s_str) = s_value.as_str() {
                    s_str.as_bytes()
                } else if let Some(bin) = s_value.as_binary() {
                    bin
                } else {
                    return Err(l.error("bad argument to 'pack' (string expected)".to_string()));
                };
                let str_len = bytes.len();

                // Check if string length fits in the specified size
                if size_bytes < 8 {
                    let max_len = (1usize << (size_bytes * 8)) - 1;
                    if str_len > max_len {
                        return Err(l.error("string length does not fit".to_string()));
                    }
                }

                // Pack the string length first
                let len_bytes = match endianness {
                    Endianness::Little => {
                        let mut bytes = Vec::new();
                        let mut v = str_len;
                        for _ in 0..size_bytes {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        bytes
                    }
                    Endianness::Big => {
                        let mut bytes = Vec::new();
                        let mut v = str_len;
                        for _ in 0..size_bytes {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        bytes.reverse();
                        bytes
                    }
                    Endianness::Native => {
                        let mut bytes = Vec::new();
                        let mut v = str_len;
                        for _ in 0..size_bytes {
                            bytes.push((v & 0xFF) as u8);
                            v >>= 8;
                        }
                        #[cfg(target_endian = "big")]
                        bytes.reverse();
                        bytes
                    }
                };

                // Check overflow before adding
                checked_add_size(result.len(), size_bytes)
                    .map_err(|_| l.error("pack result too long".to_string()))?;
                checked_add_size(result.len() + size_bytes, str_len)
                    .map_err(|_| l.error("pack result too long".to_string()))?;

                result.extend_from_slice(&len_bytes);
                result.extend_from_slice(bytes);
                value_idx += 1;
            }

            'x' => {
                // padding byte (zero)
                result.push(0);
            }

            'X' => {
                // Alignment padding - peek at next format option to determine alignment
                let next_ch = chars.peek().copied();
                if next_ch.is_none() {
                    return Err(l.error("invalid next option for option 'X'".to_string()));
                }
                let next_ch = next_ch.unwrap();

                // Determine alignment size based on next format option
                let natural_align = match next_ch {
                    'b' | 'B' => {
                        chars.next(); // consume the next format char
                        1
                    }
                    'h' | 'H' => {
                        chars.next(); // consume the next format char
                        2
                    }
                    'i' | 'I' | 'l' | 'L' | 'f' => {
                        // Check for size suffix on integers
                        chars.next(); // consume the next format char
                        let mut size_str = String::new();
                        while let Some(&digit) = chars.peek() {
                            if digit.is_ascii_digit() {
                                size_str.push(chars.next().unwrap());
                            } else {
                                break;
                            }
                        }
                        if !size_str.is_empty()
                            && (next_ch == 'i'
                                || next_ch == 'I'
                                || next_ch == 'l'
                                || next_ch == 'L')
                        {
                            let n = size_str.parse::<usize>().unwrap_or(4);
                            if !(1..=16).contains(&n) {
                                return Err(l.error(format!("({}) out of limits [1,16]", n)));
                            }
                            n
                        } else if next_ch == 'f' {
                            4
                        } else {
                            4 // default int size
                        }
                    }
                    'd' | 'n' | 'j' | 'J' | 'T' => {
                        chars.next(); // consume the next format char
                        8
                    }
                    ' ' | '\t' | '\n' | '\r' | 'X' | '<' | '>' | '=' | '!' | 'c' | 's' | 'z'
                    | 'x' => {
                        // Invalid options for X alignment
                        return Err(l.error("invalid next option for option 'X'".to_string()));
                    }
                    _ => {
                        chars.next(); // consume the next format char
                        1 // For other options, use minimal alignment
                    }
                };
                // Apply max_alignment constraint
                let align = natural_align.min(max_alignment);
                // Add padding bytes to align to boundary
                if align > 1 {
                    let padding = (align - (result.len() % align)) % align;
                    for _ in 0..padding {
                        result.push(0);
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

                max_alignment = if align_str.is_empty() {
                    // No number specified, use default max alignment (8 for double)
                    8
                } else {
                    let n: usize = align_str.parse().map_err(|_| {
                        l.error("bad argument to 'pack' (invalid alignment)".to_string())
                    })?;
                    if !(1..=16).contains(&n) {
                        return Err(l.error("alignment out of limits [1,16]".to_string()));
                    }
                    // Check if n is a power of 2
                    if n & (n - 1) != 0 {
                        return Err(l.error("alignment is not a power of 2".to_string()));
                    }
                    n
                };
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
    let mut max_alignment: usize = 1; // Default max alignment
    let mut chars = fmt_str.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue,
            'b' | 'B' => size = checked_add_size(size, 1).map_err(|e| l.error(e))?,
            'h' | 'H' => {
                // Add alignment padding
                let padding = get_alignment_padding(size, 2, max_alignment);
                size = checked_add_size(size, padding).map_err(|e| l.error(e))?;
                // Add actual size
                size = checked_add_size(size, 2).map_err(|e| l.error(e))?;
            }

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

                let item_size = if size_str.is_empty() {
                    4 // default int size
                } else {
                    let n: usize = size_str.parse().map_err(|_| {
                        l.error("bad argument to 'packsize' (invalid size)".to_string())
                    })?;
                    if !(1..=16).contains(&n) {
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };
                // Add alignment padding
                let padding = get_alignment_padding(size, item_size, max_alignment);
                size = checked_add_size(size, padding).map_err(|e| l.error(e))?;
                // Add actual size
                size = checked_add_size(size, item_size).map_err(|e| l.error(e))?;
            }

            'f' => {
                let padding = get_alignment_padding(size, 4, max_alignment);
                size = checked_add_size(size, padding).map_err(|e| l.error(e))?;
                size = checked_add_size(size, 4).map_err(|e| l.error(e))?;
            }
            'd' => {
                let padding = get_alignment_padding(size, 8, max_alignment);
                size = checked_add_size(size, padding).map_err(|e| l.error(e))?;
                size = checked_add_size(size, 8).map_err(|e| l.error(e))?;
            }
            'j' | 'J' | 'n' | 'T' => {
                // j: lua_Integer (8 bytes, i64)
                // J: unsigned lua_Integer (8 bytes, u64)
                // n: lua_Number (8 bytes, f64)
                // T: size_t (8 bytes on 64-bit platforms)
                let padding = get_alignment_padding(size, 8, max_alignment);
                size = checked_add_size(size, padding).map_err(|e| l.error(e))?;
                size = checked_add_size(size, 8).map_err(|e| l.error(e))?;
            }
            'x' => size = checked_add_size(size, 1).map_err(|e| l.error(e))?,

            'X' => {
                // Alignment padding - peek at next format option to determine alignment
                let next_ch = chars.peek().copied();
                if next_ch.is_none() {
                    return Err(l.error("invalid next option for option 'X'".to_string()));
                }
                let next_ch = next_ch.unwrap();

                // Determine alignment size based on next format option
                let natural_align = match next_ch {
                    'b' | 'B' => {
                        chars.next(); // consume the next format char
                        1
                    }
                    'h' | 'H' => {
                        chars.next(); // consume the next format char
                        2
                    }
                    'i' | 'I' | 'l' | 'L' | 'f' => {
                        // Check for size suffix on integers
                        chars.next(); // consume the next format char
                        let mut size_str = String::new();
                        while let Some(&digit) = chars.peek() {
                            if digit.is_ascii_digit() {
                                size_str.push(chars.next().unwrap());
                            } else {
                                break;
                            }
                        }
                        if !size_str.is_empty()
                            && (next_ch == 'i'
                                || next_ch == 'I'
                                || next_ch == 'l'
                                || next_ch == 'L')
                        {
                            let n = size_str.parse::<usize>().unwrap_or(4);
                            if !(1..=16).contains(&n) {
                                return Err(l.error(format!("({}) out of limits [1,16]", n)));
                            }
                            n
                        } else if next_ch == 'f' {
                            4
                        } else {
                            4 // default int size
                        }
                    }
                    'd' | 'n' | 'j' | 'J' | 'T' => {
                        chars.next(); // consume the next format char
                        8
                    }
                    ' ' | '\t' | '\n' | '\r' | 'X' | '<' | '>' | '=' | '!' | 'c' | 's' | 'z'
                    | 'x' => {
                        // Invalid options for X alignment
                        return Err(l.error("invalid next option for option 'X'".to_string()));
                    }
                    _ => {
                        chars.next(); // consume the next format char
                        1 // For other options, use minimal alignment
                    }
                };
                // Apply max_alignment constraint
                let align = natural_align.min(max_alignment);
                // Add padding to align to boundary
                if align > 1 {
                    let padding = (align - (size % align)) % align;
                    size = checked_add_size(size, padding).map_err(|e| l.error(e))?;
                }
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
                size = checked_add_size(size, n).map_err(|e| l.error(e))?;
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
                    if !(1..=16).contains(&n) {
                        return Err(l.error("alignment out of limits [1,16]".to_string()));
                    }
                    // Check if n is a power of 2
                    if n & (n - 1) != 0 {
                        return Err(l.error("alignment is not a power of 2".to_string()));
                    }
                    max_alignment = n;
                } else {
                    // If no number specified, use default max alignment (8)
                    max_alignment = 8;
                }
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

    let pos_arg = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);

    // Handle negative indices (count from end)
    let pos = if pos_arg < 0 {
        let len = bytes.len() as i64;
        let adjusted = len + pos_arg + 1;
        if adjusted < 1 {
            return Err(l.error("initial position out of string".to_string()));
        }
        adjusted
    } else if pos_arg == 0 {
        return Err(l.error("initial position out of string".to_string()));
    } else {
        pos_arg
    };

    // Check if initial position is within string bounds
    let pos_usize = pos as usize;
    if pos_usize > bytes.len() + 1 {
        return Err(l.error("initial position out of string".to_string()));
    }

    // Convert to usize
    let pos_usize = pos as usize;

    let mut idx = pos_usize - 1; // Convert to 0-based
    let mut results = Vec::new();
    let mut chars = fmt_str.chars().peekable();
    let mut endianness = Endianness::Native; // Default to native endianness
    let mut max_alignment: usize = 1; // Default max alignment

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
                skip_alignment_padding(&mut idx, 2, max_alignment);
                if idx + 2 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: i16 = endianness.from_bytes(&bytes[idx..idx + 2]);
                results.push(LuaValue::integer(val as i64));
                idx += 2;
            }

            'H' => {
                skip_alignment_padding(&mut idx, 2, max_alignment);
                if idx + 2 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: u16 = endianness.from_bytes(&bytes[idx..idx + 2]);
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
                    if !(1..=16).contains(&n) {
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                skip_alignment_padding(&mut idx, size, max_alignment);

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
                            let sign_byte = if (bytes[idx + 7] & 0x80) != 0 {
                                0xFF
                            } else {
                                0x00
                            };
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
                        let start = size.saturating_sub(8);
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
                                let sign_byte = if (bytes[idx + 7] & 0x80) != 0 {
                                    0xFF
                                } else {
                                    0x00
                                };
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
                    if !(1..=16).contains(&n) {
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                skip_alignment_padding(&mut idx, size, max_alignment);

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
                        let start = size.saturating_sub(8);
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
                results.push(LuaValue::integer(val as i64));
                idx += size;
            }

            'f' => {
                skip_alignment_padding(&mut idx, 4, max_alignment);
                if idx + 4 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: f32 = endianness.from_bytes(&bytes[idx..idx + 4]);
                results.push(LuaValue::number(val as f64));
                idx += 4;
            }

            'd' => {
                skip_alignment_padding(&mut idx, 8, max_alignment);
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: f64 = endianness.from_bytes(&bytes[idx..idx + 8]);
                results.push(LuaValue::number(val));
                idx += 8;
            }

            'j' => {
                // lua_Integer (8 bytes, i64)
                skip_alignment_padding(&mut idx, 8, max_alignment);
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: i64 = endianness.from_bytes(&bytes[idx..idx + 8]);
                results.push(LuaValue::integer(val));
                idx += 8;
            }

            'J' => {
                // unsigned lua_Integer (8 bytes, u64)
                skip_alignment_padding(&mut idx, 8, max_alignment);
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: u64 = endianness.from_bytes(&bytes[idx..idx + 8]);
                results.push(LuaValue::integer(val as i64));
                idx += 8;
            }

            'T' => {
                // size_t (8 bytes on 64-bit)
                skip_alignment_padding(&mut idx, 8, max_alignment);
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: u64 = endianness.from_bytes(&bytes[idx..idx + 8]);
                results.push(LuaValue::integer(val as i64));
                idx += 8;
            }

            'n' => {
                // lua_Number (8 bytes, f64) - same as 'd'
                skip_alignment_padding(&mut idx, 8, max_alignment);
                if idx + 8 > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }
                let val: f64 = endianness.from_bytes(&bytes[idx..idx + 8]);
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
                    return Err(l.error("unfinished string for format 'z'".to_string()));
                }
                // Create string if valid UTF-8, otherwise binary
                let str_bytes = &bytes[start..idx];
                let val = match std::str::from_utf8(str_bytes) {
                    Ok(valid_str) => l.create_string(valid_str)?,
                    Err(_) => l.create_binary(str_bytes.to_vec())?,
                };
                results.push(val);
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
                // Create string if valid UTF-8, otherwise binary
                let str_bytes = &bytes[idx..idx + size];
                let val = match std::str::from_utf8(str_bytes) {
                    Ok(valid_str) => l.create_string(valid_str)?,
                    Err(_) => l.create_binary(str_bytes.to_vec())?,
                };
                results.push(val);
                idx += size;
            }

            's' => {
                // variable-length string with size prefix
                let mut size_str = String::new();
                while let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() {
                        size_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }

                let size_bytes = if size_str.is_empty() {
                    8 // default size_t (8 bytes on 64-bit)
                } else {
                    let n: usize = size_str.parse().map_err(|_| {
                        l.error("bad argument to 'unpack' (invalid size)".to_string())
                    })?;
                    if !(1..=16).contains(&n) {
                        return Err(l.error(format!("({}) out of limits [1,16]", n)));
                    }
                    n
                };

                // Read the string length
                if idx + size_bytes > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }

                let str_len: usize = match endianness {
                    Endianness::Little => {
                        let mut v: usize = 0;
                        for i in 0..size_bytes.min(std::mem::size_of::<usize>()) {
                            v |= (bytes[idx + i] as usize) << (i * 8);
                        }
                        v
                    }
                    Endianness::Big => {
                        let mut v: usize = 0;
                        let start = if size_bytes > std::mem::size_of::<usize>() {
                            size_bytes - std::mem::size_of::<usize>()
                        } else {
                            0
                        };
                        for i in start..size_bytes {
                            v = (v << 8) | (bytes[idx + i] as usize);
                        }
                        v
                    }
                    Endianness::Native => {
                        let mut v: usize = 0;
                        #[cfg(target_endian = "little")]
                        for i in 0..size_bytes.min(std::mem::size_of::<usize>()) {
                            v |= (bytes[idx + i] as usize) << (i * 8);
                        }
                        #[cfg(target_endian = "big")]
                        {
                            let start = if size_bytes > std::mem::size_of::<usize>() {
                                size_bytes - std::mem::size_of::<usize>()
                            } else {
                                0
                            };
                            for i in start..size_bytes {
                                v = (v << 8) | (bytes[idx + i] as usize);
                            }
                        }
                        v
                    }
                };

                idx += size_bytes;

                // Read the string data
                if idx + str_len > bytes.len() {
                    return Err(l.error("data string too short".to_string()));
                }

                // Create string if valid UTF-8, otherwise binary
                let str_bytes = &bytes[idx..idx + str_len];
                let val = match std::str::from_utf8(str_bytes) {
                    Ok(valid_str) => l.create_string(valid_str)?,
                    Err(_) => l.create_binary(str_bytes.to_vec())?,
                };
                results.push(val);
                idx += str_len;
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
                let next_ch = chars.peek().copied();
                if next_ch.is_none() {
                    return Err(l.error("invalid next option for option 'X'".to_string()));
                }
                let next_ch = next_ch.unwrap();

                // Determine alignment size based on next format option
                let natural_align = match next_ch {
                    'b' | 'B' => {
                        chars.next(); // consume the next format char
                        1
                    }
                    'h' | 'H' => {
                        chars.next(); // consume the next format char
                        2
                    }
                    'i' | 'I' | 'l' | 'L' | 'f' => {
                        // Check for size suffix on integers
                        chars.next(); // consume the next format char
                        let mut size_str = String::new();
                        while let Some(&digit) = chars.peek() {
                            if digit.is_ascii_digit() {
                                size_str.push(chars.next().unwrap());
                            } else {
                                break;
                            }
                        }
                        if !size_str.is_empty()
                            && (next_ch == 'i'
                                || next_ch == 'I'
                                || next_ch == 'l'
                                || next_ch == 'L')
                        {
                            size_str.parse::<usize>().unwrap_or(4)
                        } else if next_ch == 'f' {
                            4
                        } else {
                            4 // default int size
                        }
                    }
                    'd' | 'n' | 'j' | 'J' | 'T' => {
                        chars.next(); // consume the next format char
                        8
                    }
                    ' ' | '\t' | '\n' | '\r' | 'X' | '<' | '>' | '=' | '!' | 'c' | 's' | 'z'
                    | 'x' => {
                        // Invalid options for X alignment
                        return Err(l.error("invalid next option for option 'X'".to_string()));
                    }
                    _ => {
                        chars.next(); // consume the next format char
                        1
                    }
                };
                // Apply max_alignment constraint
                let align = natural_align.min(max_alignment);
                // Skip padding bytes to align to boundary
                if align > 1 {
                    let padding = (align - (idx % align)) % align;
                    idx += padding;
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

                max_alignment = if align_str.is_empty() {
                    // No number specified, use default max alignment (8 for double)
                    8
                } else {
                    let n: usize = align_str.parse().map_err(|_| {
                        l.error("bad argument to 'unpack' (invalid alignment)".to_string())
                    })?;
                    if !(1..=16).contains(&n) {
                        return Err(l.error("alignment out of limits [1,16]".to_string()));
                    }
                    // Check if n is a power of 2
                    if n & (n - 1) != 0 {
                        return Err(l.error("alignment is not a power of 2".to_string()));
                    }
                    n
                };
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
