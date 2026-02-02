use crate::{LuaResult, LuaValue, lua_vm::LuaState};
/// Optimized string.format implementation
/// Reduced from 400+ lines to ~200 lines with better performance
/// Uses std::fmt::Write for zero-allocation formatting directly to buffer
use std::fmt::Write as FmtWrite;

/// string.format(formatstring, ...) - Format with various specifiers
pub fn string_format(l: &mut LuaState) -> LuaResult<usize> {
    // Get format string
    let format_str_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'format' (string expected)".to_string()))?;

    // Get format string directly without object_pool
    let format = format_str_value
        .as_str()
        .ok_or_else(|| l.error("invalid format string".to_string()))?
        .to_string();

    // Collect arguments
    let args = l.get_args();
    let mut arg_index = 1;

    // Pre-allocate result (estimate: format length + 50% for expansions)
    let mut result = String::with_capacity(format.len() + format.len() / 2);
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            result.push(ch);
            continue;
        }

        // Check for %%
        if matches!(chars.peek(), Some(&'%')) {
            chars.next();
            result.push('%');
            continue;
        }

        // Parse flags (-, +, space, #, 0)
        let mut flags = String::new();
        while let Some(&c) = chars.peek() {
            if matches!(c, '-' | '+' | ' ' | '#' | '0' | '1'..='9' | '.') {
                flags.push(c);
                chars.next();
            } else {
                break;
            }
        }

        // Get format character
        let fmt_char = chars
            .next()
            .ok_or_else(|| l.error("incomplete format".to_string()))?;

        // Get argument
        let arg = args.get(arg_index).ok_or_else(|| {
            l.error(format!(
                "bad argument #{} to 'format' (no value)",
                arg_index + 1
            ))
        })?;
        arg_index += 1;

        // Format based on type
        match fmt_char {
            'c' => format_char(&mut result, arg, l)?,
            'd' | 'i' => format_int(&mut result, arg, &flags, l)?,
            'o' => format_octal(&mut result, arg, &flags, l)?,
            'u' => format_uint(&mut result, arg, &flags, l)?,
            'x' => format_hex(&mut result, arg, &flags, false, l)?,
            'X' => format_hex(&mut result, arg, &flags, true, l)?,
            'a' => format_hex_float(&mut result, arg, &flags, false, l)?,
            'A' => format_hex_float(&mut result, arg, &flags, true, l)?,
            'e' => format_sci(&mut result, arg, &flags, false, l)?,
            'E' => format_sci(&mut result, arg, &flags, true, l)?,
            'f' => format_float(&mut result, arg, &flags, l)?,
            'g' => format_auto(&mut result, arg, false, l)?,
            'G' => format_auto(&mut result, arg, true, l)?,
            's' => format_string(&mut result, arg, &flags, l)?,
            'q' => format_quoted(&mut result, arg, l)?,
            'p' => format_pointer(&mut result, arg, &flags)?,
            _ => return Err(l.error(format!("invalid option '%{}' to 'format'", fmt_char))),
        }
    }

    let result_str = l.create_string_owned(result)?;
    l.push_value(result_str)?;
    Ok(1)
}

// Helper functions - all inline for performance

#[inline]
fn get_num(arg: &LuaValue, _l: &LuaState) -> Result<f64, String> {
    arg.as_number()
        .or_else(|| arg.as_integer().map(|i| i as f64))
        .ok_or_else(|| "bad argument to 'format' (number expected)".to_string())
}

#[inline]
fn get_int(arg: &LuaValue, _l: &LuaState) -> Result<i64, String> {
    arg.as_integer()
        .or_else(|| arg.as_number().map(|n| n as i64))
        .ok_or_else(|| "bad argument to 'format' (number expected)".to_string())
}

#[inline]
fn format_char(buf: &mut String, arg: &LuaValue, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    if (0..=255).contains(&num) {
        buf.push(num as u8 as char);
        Ok(())
    } else {
        Err(l.error("bad argument to 'format' (value out of range for %c)".to_string()))
    }
}

#[inline]
fn format_int(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    
    // Parse flags: look for -, +, 0, width, and precision
    let mut left_align = false;
    let mut zero_pad = false;
    let mut plus_sign = false;
    let mut width = 0;
    let mut parsing_width = false;
    
    let mut chars = flags.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '-' if !parsing_width => left_align = true,
            '+' if !parsing_width => plus_sign = true,
            '0' if !parsing_width && width == 0 => zero_pad = true,
            '1'..='9' if !parsing_width => {
                parsing_width = true;
                width = ch.to_digit(10).unwrap() as usize;
            }
            '0'..='9' if parsing_width => {
                width = width * 10 + ch.to_digit(10).unwrap() as usize;
            }
            '.' => break,
            _ => {}
        }
    }
    
    // Parse precision
    let precision = if let Some(dot_pos) = flags.find('.') {
        flags[dot_pos + 1..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<usize>()
            .ok()
    } else {
        None
    };
    
    // Extract sign and absolute value
    let is_negative = num < 0;
    let abs_num = if is_negative {
        num.wrapping_neg() as u64
    } else {
        num as u64
    };
    
    // Format the absolute number
    let mut num_str = format!("{}", abs_num);
    
    // Apply precision (minimum digits)
    if let Some(prec) = precision {
        if num_str.len() < prec {
            let padding = prec - num_str.len();
            num_str.insert_str(0, &"0".repeat(padding));
        }
        zero_pad = false; // Precision overrides zero-padding
    }
    
    // Add sign
    let sign = if is_negative {
        "-"
    } else if plus_sign {
        "+"
    } else {
        ""
    };
    
    let total_len = sign.len() + num_str.len();
    
    if width > total_len {
        let padding = width - total_len;
        if left_align {
            buf.push_str(sign);
            buf.push_str(&num_str);
            buf.extend(std::iter::repeat(' ').take(padding));
        } else if zero_pad {
            buf.push_str(sign);
            buf.extend(std::iter::repeat('0').take(padding));
            buf.push_str(&num_str);
        } else {
            buf.extend(std::iter::repeat(' ').take(padding));
            buf.push_str(sign);
            buf.push_str(&num_str);
        }
    } else {
        buf.push_str(sign);
        buf.push_str(&num_str);
    }
    
    Ok(())
}

#[inline]
fn format_octal(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    
    // Parse flags: look for -, #, 0, and width
    let mut left_align = false;
    let mut zero_pad = false;
    let mut alt_form = false;
    let mut width = 0;
    let mut parsing_flags = true;
    
    for ch in flags.chars() {
        if parsing_flags {
            match ch {
                '-' => left_align = true,
                '#' => alt_form = true,
                '0' if width == 0 => zero_pad = true,
                '1'..='9' => {
                    parsing_flags = false;
                    width = ch.to_digit(10).unwrap() as usize;
                }
                _ => {}
            }
        } else if ch.is_ascii_digit() {
            width = width * 10 + ch.to_digit(10).unwrap() as usize;
        }
    }
    
    // Format the octal number
    let mut octal_str = format!("{:o}", num);
    
    // Add prefix if # flag and non-zero
    if alt_form && num != 0 && !octal_str.starts_with('0') {
        octal_str.insert(0, '0');
    }
    
    let num_len = octal_str.len();
    if width > num_len {
        let padding = width - num_len;
        if left_align {
            buf.push_str(&octal_str);
            buf.extend(std::iter::repeat(' ').take(padding));
        } else if zero_pad {
            buf.extend(std::iter::repeat('0').take(padding));
            buf.push_str(&octal_str);
        } else {
            buf.extend(std::iter::repeat(' ').take(padding));
            buf.push_str(&octal_str);
        }
    } else {
        buf.push_str(&octal_str);
    }
    
    Ok(())
}

#[inline]
fn format_uint(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    
    // Parse precision (e.g., ".0" or just ".")
    let precision = if let Some(dot_pos) = flags.find('.') {
        let prec_str = &flags[dot_pos + 1..];
        let prec_str = prec_str.trim_end_matches(|c: char| !c.is_ascii_digit());
        if prec_str.is_empty() {
            Some(0) // "." without number means precision 0
        } else {
            prec_str.parse::<usize>().ok()
        }
    } else {
        None
    };
    
    // If precision is 0 and value is 0, output empty string
    if precision == Some(0) && num == 0 {
        return Ok(());
    }
    
    write!(buf, "{}", num as u64).unwrap();
    Ok(())
}

#[inline]
fn format_hex(
    buf: &mut String,
    arg: &LuaValue,
    flags: &str,
    upper: bool,
    l: &mut LuaState,
) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;

    // Format the hex number
    let hex_str = if upper {
        format!("{:X}", num)
    } else {
        format!("{:x}", num)
    };
    
    // Parse flags for width and zero-padding
    let mut zero_pad = false;
    let mut left_align = false;
    let mut width = 0;
    let mut parsing_flags = true;
    
    for ch in flags.chars() {
        if parsing_flags {
            match ch {
                '0' if width == 0 => zero_pad = true,
                '-' => left_align = true,
                '#' => {},
                '1'..='9' => {
                    parsing_flags = false;
                    width = ch.to_digit(10).unwrap() as usize;
                }
                _ => {}
            }
        } else if ch.is_ascii_digit() {
            width = width * 10 + ch.to_digit(10).unwrap() as usize;
        }
    }
    
    // Add prefix if needed
    let prefix = if flags.contains('#') && num != 0 {
        if upper { "0X" } else { "0x" }
    } else {
        ""
    };
    
    let total_len = prefix.len() + hex_str.len();
    
    if width > total_len {
        let padding = width - total_len;
        if left_align {
            buf.push_str(prefix);
            buf.push_str(&hex_str);
            buf.extend(std::iter::repeat(' ').take(padding));
        } else if zero_pad {
            buf.push_str(prefix);
            buf.extend(std::iter::repeat('0').take(padding));
            buf.push_str(&hex_str);
        } else {
            buf.extend(std::iter::repeat(' ').take(padding));
            buf.push_str(prefix);
            buf.push_str(&hex_str);
        }
    } else {
        buf.push_str(prefix);
        buf.push_str(&hex_str);
    }
    
    Ok(())
}

/// Format hexadecimal float (%a/%A) - IEEE 754 hex representation
#[inline]
fn format_hex_float(
    buf: &mut String,
    arg: &LuaValue,
    flags: &str,
    upper: bool,
    l: &mut LuaState,
) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;
    
    // Parse flags for + sign
    let plus_sign = flags.contains('+');
    
    // Parse precision (number of hex digits after decimal point)
    let precision = if let Some(dot_pos) = flags.find('.') {
        flags[dot_pos + 1..]
            .trim_end_matches(|c: char| !c.is_ascii_digit())
            .parse::<usize>()
            .ok()
    } else {
        None
    };
    
    // Handle special cases
    if num.is_nan() {
        buf.push_str(if upper { "NAN" } else { "nan" });
        return Ok(());
    }
    
    if num.is_infinite() {
        if num.is_sign_positive() {
            if plus_sign {
                buf.push('+');
            }
            buf.push_str(if upper { "INF" } else { "inf" });
        } else {
            buf.push_str(if upper { "-INF" } else { "-inf" });
        }
        return Ok(());
    }
    
    // Handle zero
    if num == 0.0 {
        // Check for negative zero
        if num.is_sign_negative() {
            buf.push('-');
        } else if plus_sign {
            buf.push('+');
        }
        
        if let Some(prec) = precision {
            buf.push_str(if upper { "0X0" } else { "0x0" });
            if prec > 0 {
                buf.push('.');
                buf.extend(std::iter::repeat('0').take(prec));
            }
            buf.push(if upper { 'P' } else { 'p' });
            buf.push_str("+0");
        } else {
            buf.push_str(if upper { "0X0P+0" } else { "0x0p+0" });
        }
        return Ok(());
    }
    
    // Extract sign
    let is_negative = num.is_sign_negative();
    let abs_num = num.abs();
    
    if is_negative {
        buf.push('-');
    } else if plus_sign {
        buf.push('+');
    }
    
    // Decompose into mantissa and exponent
    // IEEE 754: value = mantissa * 2^exponent
    // We want: 0x1.hhhhhp±e format where mantissa is normalized to [1, 2)
    
    let bits = abs_num.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7FF) as i32;
    let mantissa_bits = bits & 0xFFFFFFFFFFFFF;
    
    if exponent_bits == 0 {
        // Subnormal number
        let binary_exp = -1022 - 52;
        
        // Normalize: find first 1 bit
        let leading_zeros = mantissa_bits.leading_zeros() - 12; // 64 - 52 bits
        let normalized_mantissa = mantissa_bits << (leading_zeros + 1);
        let actual_exp = binary_exp + leading_zeros as i32;
        
        buf.push_str(if upper { "0X1" } else { "0x1" });
        
        // Output fractional part
        let frac = (normalized_mantissa >> 1) & 0x7FFFFFFFFFFFF;
        format_hex_fraction(buf, frac, precision, upper);
        
        buf.push(if upper { 'P' } else { 'p' });
        buf.push_str(&format!("{:+}", actual_exp));
    } else {
        // Normal number
        let binary_exp = exponent_bits - 1023;
        
        buf.push_str(if upper { "0X1" } else { "0x1" });
        
        // Output fractional part
        format_hex_fraction(buf, mantissa_bits, precision, upper);
        
        buf.push(if upper { 'P' } else { 'p' });
        buf.push_str(&format!("{:+}", binary_exp));
    }
    
    Ok(())
}

// Helper function to format the fractional part of hex float
fn format_hex_fraction(buf: &mut String, mantissa_bits: u64, precision: Option<usize>, upper: bool) {
    if let Some(prec) = precision {
        if prec > 0 {
            buf.push('.');
            // Convert 52 bits to hex string (13 hex digits)
            let hex_str = format!("{:013x}", mantissa_bits);
            // Take only the requested precision
            let output = if prec < hex_str.len() {
                &hex_str[..prec]
            } else {
                &hex_str
            };
            if upper {
                buf.push_str(&output.to_uppercase());
            } else {
                buf.push_str(output);
            }
            // Pad with zeros if needed
            if prec > hex_str.len() {
                buf.extend(std::iter::repeat('0').take(prec - hex_str.len()));
            }
        }
    } else {
        // No precision specified: output all significant digits (trim trailing zeros)
        if mantissa_bits != 0 {
            buf.push('.');
            let hex_str = format!("{:013x}", mantissa_bits);
            let trimmed = hex_str.trim_end_matches('0');
            if upper {
                buf.push_str(&trimmed.to_uppercase());
            } else {
                buf.push_str(trimmed);
            }
        }
    }
}

#[inline]
fn format_sci(
    buf: &mut String,
    arg: &LuaValue,
    flags: &str,
    upper: bool,
    l: &mut LuaState,
) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;

    // Parse precision from flags (e.g., ".2")
    if let Some(dot_pos) = flags.find('.') {
        if let Ok(prec) = flags[dot_pos + 1..]
            .trim_end_matches(|c: char| !c.is_ascii_digit())
            .parse::<usize>()
        {
            if upper {
                write!(buf, "{:.prec$E}", num, prec = prec).unwrap();
            } else {
                write!(buf, "{:.prec$e}", num, prec = prec).unwrap();
            }
            return Ok(());
        }
    }

    if upper {
        write!(buf, "{:E}", num).unwrap();
    } else {
        write!(buf, "{:e}", num).unwrap();
    }
    Ok(())
}

#[inline]
fn format_float(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;

    // Parse precision from flags (e.g., ".2")
    if let Some(dot_pos) = flags.find('.') {
        if let Ok(prec) = flags[dot_pos + 1..]
            .trim_end_matches(|c: char| !c.is_ascii_digit())
            .parse::<usize>()
        {
            write!(buf, "{:.prec$}", num, prec = prec).unwrap();
            return Ok(());
        }
    }

    write!(buf, "{}", num).unwrap();
    Ok(())
}

#[inline]
fn format_auto(buf: &mut String, arg: &LuaValue, upper: bool, l: &mut LuaState) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;

    // Use scientific for very large/small numbers
    if num.abs() < 0.0001 || num.abs() >= 1e10 {
        if upper {
            write!(buf, "{:E}", num).unwrap();
        } else {
            write!(buf, "{:e}", num).unwrap();
        }
    } else {
        write!(buf, "{}", num).unwrap();
    }
    Ok(())
}

#[inline]
fn format_string(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let s_content = if let Some(s) = arg.as_str() {
        s.to_string()
    } else if let Some(n) = arg.as_integer() {
        format!("{}", n)
    } else if let Some(n) = arg.as_number() {
        format!("{}", n)
    } else {
        l.to_string(arg)?
    };
    
    // Check if format has width or precision
    let has_modifiers = !flags.is_empty() && (flags.chars().any(|c| c.is_ascii_digit()) || flags.contains('.'));
    
    // If there's width/precision modifiers and string contains \0, error
    if has_modifiers && s_content.contains('\0') {
        return Err(l.error("string contains zeros".to_string()));
    }
    
    // Check for precision (e.g., %.20s means max 20 chars)
    let final_str = if let Some(dot_pos) = flags.find('.') {
        if let Ok(precision) = flags[dot_pos + 1..].parse::<usize>() {
            if precision < s_content.len() {
                &s_content[..precision]
            } else {
                &s_content
            }
        } else {
            &s_content
        }
    } else {
        &s_content
    };
    
    // Apply width formatting if specified
    apply_width_format(buf, final_str, flags);
    Ok(())
}

#[inline]
fn format_quoted(buf: &mut String, arg: &LuaValue, l: &mut LuaState) -> LuaResult<()> {
    // For numbers and booleans and nil, output without quotes
    if arg.is_number() || arg.is_integer() {
        if let Some(i) = arg.as_integer() {
            // Special case for mininteger: Lua parser can't handle -9223372036854775808 as integer literal
            if i == i64::MIN {
                buf.push_str("(-9223372036854775807-1)");
            } else {
                write!(buf, "{}", i).unwrap();
            }
        } else if let Some(f) = arg.as_number() {
            // Special cases for inf and nan
            if f.is_infinite() {
                if f.is_sign_positive() {
                    buf.push_str("(1/0)");
                } else {
                    buf.push_str("(-1/0)");
                }
            } else if f.is_nan() {
                buf.push_str("(0/0)");
            } else {
                write!(buf, "{}", f).unwrap();
            }
        }
        return Ok(());
    }
    
    if arg.is_boolean() {
        let b = arg.as_boolean().unwrap();
        buf.push_str(if b { "true" } else { "false" });
        return Ok(());
    }
    
    if arg.is_nil() {
        buf.push_str("nil");
        return Ok(());
    }
    
    // For tables, functions, etc., there's no literal representation
    if !arg.is_string() {
        return Err(l.error("no literal representation for value in 'format'".to_string()));
    }
    
    // For strings, convert and quote
    let s = if let Some(s) = arg.as_str() {
        s.to_string()
    } else {
        l.to_string(arg)?
    };

    buf.push('"');
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => buf.push_str("\\\""),
            b'\\' => buf.push_str("\\\\"),
            b'\n' => buf.push_str("\\n"),
            b'\r' => buf.push_str("\\r"),
            b'\t' => buf.push_str("\\t"),
            b if b < 32 || b == 127 => {
                // Control characters: use \ddd format
                // If next character is a digit, we need 3 digits
                let need_padding = i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit();
                if need_padding {
                    write!(buf, "\\{:03}", b).unwrap();
                } else {
                    write!(buf, "\\{}", b).unwrap();
                }
            }
            b if b >= 128 => {
                // Non-ASCII bytes: use \ddd format (3 digits)
                // If next character is a digit, we need 3 digits
                let need_padding = i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit();
                if need_padding {
                    write!(buf, "\\{:03}", b).unwrap();
                } else {
                    write!(buf, "\\{}", b).unwrap();
                }
            }
            b => buf.push(b as char),
        }
        i += 1;
    }
    buf.push('"');
    Ok(())
}

/// Format pointer (%p) - shows address for tables/functions, "(null)" for others
fn format_pointer(buf: &mut String, arg: &LuaValue, flags: &str) -> LuaResult<()> {
    use crate::lua_value::LuaValueKind;
    
    // Format the pointer value first
    let ptr_str = match arg.kind() {
        LuaValueKind::String | LuaValueKind::Binary | 
        LuaValueKind::Table | LuaValueKind::Function | LuaValueKind::CFunction | 
        LuaValueKind::Userdata | LuaValueKind::Thread => {
            let ptr = unsafe { arg.value.ptr as usize };
            format!("0x{:x}", ptr)
        }
        _ => {
            "(null)".to_string()
        }
    };
    
    // Apply width formatting if specified
    apply_width_format(buf, &ptr_str, flags);
    Ok(())
}

/// Apply width formatting to a string (handles %10s, %-10s etc.)
fn apply_width_format(buf: &mut String, s: &str, flags: &str) {
    // Parse width - find the numeric part
    let left_align = flags.starts_with('-');
    let width_str = flags.trim_start_matches('-').trim_start_matches('+')
        .trim_start_matches(' ').trim_start_matches('#')
        .trim_start_matches('0');
    
    if let Ok(width) = width_str.parse::<usize>() {
        let s_len = s.len();
        if width > s_len {
            let padding = width - s_len;
            if left_align {
                // Left align: string then spaces
                buf.push_str(s);
                buf.extend(std::iter::repeat(' ').take(padding));
            } else {
                // Right align (default): spaces then string
                buf.extend(std::iter::repeat(' ').take(padding));
                buf.push_str(s);
            }
        } else {
            buf.push_str(s);
        }
    } else {
        // No width specified or parse failed, just append
        buf.push_str(s);
    }
}
