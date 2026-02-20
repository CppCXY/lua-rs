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

    // Get format string as raw bytes — avoid cloning.
    // SAFETY: format string is Lua arg 1, held on the Lua stack, won't be GC'd
    // during this function. GC only runs at `create_string_owned` at the very end.
    let (fmt_bytes, fmt_len) = {
        let format_str = format_str_value
            .as_str()
            .ok_or_else(|| l.error("invalid format string".to_string()))?;
        let ptr = format_str.as_ptr();
        let len = format_str.len();
        (unsafe { std::slice::from_raw_parts(ptr, len) }, len)
    };

    // Collect arguments
    let args = l.get_args();
    let mut arg_index = 1;

    let mut pos = 0;

    // Pre-allocate result (estimate: format length + 50% for expansions)
    let mut result = String::with_capacity(fmt_len + fmt_len / 2);

    while pos < fmt_len {
        // Find next '%' — copy non-format sections in bulk
        let start = pos;
        while pos < fmt_len && fmt_bytes[pos] != b'%' {
            pos += 1;
        }
        if pos > start {
            // SAFETY: format string is valid UTF-8, non-% ASCII segments are valid UTF-8
            result.push_str(unsafe { std::str::from_utf8_unchecked(&fmt_bytes[start..pos]) });
        }
        if pos >= fmt_len {
            break;
        }

        // Skip the '%'
        pos += 1;
        if pos >= fmt_len {
            return Err(l.error("incomplete format".to_string()));
        }

        // Check for %%
        if fmt_bytes[pos] == b'%' {
            result.push('%');
            pos += 1;
            continue;
        }

        // Parse flags (-, +, space, #, 0) and width/precision as a byte slice — no allocation
        let flags_start = pos;
        while pos < fmt_len {
            let c = fmt_bytes[pos];
            if matches!(c, b'-' | b'+' | b' ' | b'#' | b'0' | b'1'..=b'9' | b'.') {
                pos += 1;
                if pos - flags_start > 200 {
                    return Err(l.error("invalid format (too long)".to_string()));
                }
            } else {
                break;
            }
        }
        // SAFETY: format string is valid UTF-8, flag chars are ASCII
        let flags = unsafe { std::str::from_utf8_unchecked(&fmt_bytes[flags_start..pos]) };

        // Get format character
        if pos >= fmt_len {
            return Err(l.error("incomplete format".to_string()));
        }
        let fmt_char = fmt_bytes[pos] as char;
        pos += 1;

        // Validate format specifier and flags combination
        validate_format(&flags, fmt_char, l)?;

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
            'c' => format_char(&mut result, arg, &flags, l)?,
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
            'g' => format_auto(&mut result, arg, &flags, false, l)?,
            'G' => format_auto(&mut result, arg, &flags, true, l)?,
            's' => format_string(&mut result, arg, &flags, l)?,
            'q' => format_quoted(&mut result, arg, l)?,
            'p' => format_pointer(&mut result, arg, &flags)?,
            'F' => {
                return Err(
                    l.error("invalid option '%F' to 'format' (invalid conversion)".to_string())
                );
            }
            _ => {
                return Err(l.error(format!(
                    "invalid option '%{}' to 'format' (invalid conversion)",
                    fmt_char
                )));
            }
        }
    }

    let result_str = l.create_string_owned(result)?;
    l.push_value(result_str)?;
    Ok(1)
}

// Validate format specifier and flags combination
fn validate_format(flags: &str, fmt_char: char, l: &mut LuaState) -> LuaResult<()> {
    // Fast path: no flags (most common case)
    if flags.is_empty() {
        return Ok(());
    }

    // Check for modifiers on %q
    if fmt_char == 'q' {
        return Err(
            l.error("specifier '%q' cannot have modifiers (width, precision, flags)".to_string())
        );
    }

    // Parse flags properly
    let has_zero_flag = flags.starts_with('0')
        || flags
            .chars()
            .skip_while(|c| matches!(c, '-' | '+' | ' ' | '#'))
            .next()
            == Some('0');
    let has_hash = flags.contains('#');

    let (width, precision) = parse_width_precision(flags);

    // Check width/precision limits (max 99)
    if let Some(w) = width {
        if w > 99 {
            return Err(l.error("invalid format (invalid conversion)".to_string()));
        }
    }
    if let Some(p) = precision {
        if p > 99 {
            return Err(l.error("invalid format (invalid conversion)".to_string()));
        }
    }

    // Format-specific validation
    match fmt_char {
        'c' => {
            // %c cannot have precision or 0 flag
            if precision.is_some() {
                return Err(l.error("invalid format (invalid conversion)".to_string()));
            }
            if has_zero_flag {
                return Err(l.error("invalid format (invalid conversion)".to_string()));
            }
        }
        's' => {
            // %s cannot have 0 flag
            if has_zero_flag {
                return Err(l.error("invalid format (invalid conversion)".to_string()));
            }
        }
        'd' | 'i' => {
            // %d/%i cannot have # flag
            if has_hash {
                return Err(l.error("invalid format (invalid conversion)".to_string()));
            }
        }
        'p' => {
            // %p cannot have precision
            if precision.is_some() {
                return Err(l.error("invalid format (invalid conversion)".to_string()));
            }
        }
        _ => {}
    }

    Ok(())
}

// Parse width and precision from flags string
fn parse_width_precision(flags: &str) -> (Option<usize>, Option<usize>) {
    let mut width = None;
    let mut precision = None;

    if let Some(dot_pos) = flags.find('.') {
        // Parse width before dot
        let width_str: String = flags[..dot_pos]
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !width_str.is_empty() {
            width = width_str.parse().ok();
        }

        // Parse precision after dot
        let prec_str: String = flags[dot_pos + 1..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !prec_str.is_empty() {
            precision = prec_str.parse().ok();
        } else {
            precision = Some(0); // "." without digits means precision 0
        }
    } else {
        // No dot, only width
        let width_str: String = flags
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !width_str.is_empty() {
            width = width_str.parse().ok();
        }
    }

    (width, precision)
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
fn format_char(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    if !(0..=255).contains(&num) {
        return Err(l.error("bad argument to 'format' (value out of range for %c)".to_string()));
    }

    let ch = num as u8 as char;

    // Parse flags for width and left align
    let left_align = flags.contains('-');
    let width = flags
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<usize>()
        .ok();

    if let Some(w) = width {
        if w > 1 {
            if left_align {
                buf.push(ch);
                buf.extend(std::iter::repeat(' ').take(w - 1));
            } else {
                buf.extend(std::iter::repeat(' ').take(w - 1));
                buf.push(ch);
            }
        } else {
            buf.push(ch);
        }
    } else {
        buf.push(ch);
    }

    Ok(())
}

#[inline]
fn format_int(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;

    // Fast path: no flags (most common case: %d or %i)
    if flags.is_empty() {
        let mut itoa_buf = itoa::Buffer::new();
        buf.push_str(itoa_buf.format(num));
        return Ok(());
    }

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

    // Format the absolute number using itoa (no heap allocation)
    let mut itoa_buf = itoa::Buffer::new();
    let mut num_str = itoa_buf.format(abs_num).to_string();

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
                '#' => {}
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
fn format_hex_fraction(
    buf: &mut String,
    mantissa_bits: u64,
    precision: Option<usize>,
    upper: bool,
) {
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

    // Parse flags
    let plus_sign = flags.contains('+');
    let space_sign = flags.contains(' ');

    // Parse precision
    let precision = if let Some(dot_pos) = flags.find('.') {
        let prec_str = &flags[dot_pos + 1..];
        let prec_str = prec_str
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        if prec_str.is_empty() {
            Some(6)
        } else {
            prec_str.parse::<usize>().ok()
        }
    } else {
        None
    };

    // Format the number
    let mut result = if let Some(prec) = precision {
        if upper {
            format!("{:.prec$E}", num, prec = prec)
        } else {
            format!("{:.prec$e}", num, prec = prec)
        }
    } else {
        if upper {
            format!("{:E}", num)
        } else {
            format!("{:e}", num)
        }
    };

    // Fix exponent format: ensure it has sign and at least 2 digits (ISO C requirement)
    let exp_char = if upper { 'E' } else { 'e' };
    if let Some(exp_pos) = result.find(exp_char) {
        let (mantissa, exponent) = result.split_at(exp_pos);
        let exp_str = &exponent[1..]; // Skip 'E' or 'e'

        // Parse exponent
        let (sign, exp_digits) = if exp_str.starts_with('+') || exp_str.starts_with('-') {
            let s = exp_str.chars().next().unwrap();
            (s.to_string(), &exp_str[1..])
        } else {
            ("+".to_string(), exp_str)
        };

        // Ensure at least 2 digits
        let exp_num = exp_digits.parse::<i32>().unwrap_or(0);
        let formatted_exp = format!("{:02}", exp_num);

        result = format!("{}{}{}{}", mantissa, exp_char, sign, formatted_exp);
    }

    // Add sign
    if !result.starts_with('-') {
        if plus_sign {
            result.insert(0, '+');
        } else if space_sign {
            result.insert(0, ' ');
        }
    }

    buf.push_str(&result);
    Ok(())
}

#[inline]
fn format_float(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;

    // Parse flags
    let plus_sign = flags.contains('+');
    let alt_form = flags.contains('#');
    let zero_pad = flags.contains('0');
    let left_align = flags.contains('-');

    // Parse width
    let width = flags
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() && *c != '.')
        .collect::<String>()
        .parse::<usize>()
        .ok();

    // Parse precision
    let precision = if let Some(dot_pos) = flags.find('.') {
        let prec_str = &flags[dot_pos + 1..];
        let prec_str = prec_str
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        if prec_str.is_empty() {
            Some(0)
        } else {
            prec_str.parse::<usize>().ok()
        }
    } else {
        None
    };

    // Format the number
    let mut result = if let Some(prec) = precision {
        format!("{:.prec$}", num, prec = prec)
    } else {
        format!("{}", num)
    };

    // Add decimal point if # flag and no decimal point
    if alt_form && !result.contains('.') && !result.contains('e') && !result.contains('E') {
        result.push('.');
    }

    // Add sign
    if !result.starts_with('-') && plus_sign {
        result.insert(0, '+');
    }

    // Apply width
    if let Some(w) = width {
        if result.len() < w {
            let padding = w - result.len();
            if left_align {
                result.push_str(&" ".repeat(padding));
            } else if zero_pad {
                let sign_char = if result.starts_with('-') || result.starts_with('+') {
                    Some(result.remove(0))
                } else {
                    None
                };
                if let Some(sign) = sign_char {
                    result.insert(0, sign);
                    result.insert_str(1, &"0".repeat(padding));
                } else {
                    result.insert_str(0, &"0".repeat(padding));
                }
            } else {
                result.insert_str(0, &" ".repeat(padding));
            }
        }
    }

    buf.push_str(&result);
    Ok(())
}

#[inline]
fn format_auto(
    buf: &mut String,
    arg: &LuaValue,
    flags: &str,
    upper: bool,
    l: &mut LuaState,
) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;

    // Parse flags
    let plus_sign = flags.contains('+');
    let space_sign = flags.contains(' ');

    // Parse precision (for %g, precision is significant figures, default 6, minimum 1)
    let precision = if let Some(dot_pos) = flags.find('.') {
        let prec_str = &flags[dot_pos + 1..];
        let prec_str = prec_str
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        if prec_str.is_empty() {
            6
        } else {
            prec_str.parse::<usize>().unwrap_or(6).max(1)
        }
    } else {
        6
    };

    // Determine if we should use scientific notation
    // %g uses scientific notation if exponent < -4 or >= precision
    let abs_num = num.abs();
    let use_scientific = if abs_num == 0.0 {
        false
    } else {
        let exponent = abs_num.log10().floor() as i32;
        exponent < -4 || exponent >= precision as i32
    };

    let mut result = if use_scientific {
        // Use scientific notation
        let exp_char = if upper { 'E' } else { 'e' };
        let formatted = format!("{:.prec$e}", num, prec = precision - 1);

        // Fix exponent format
        if let Some(exp_pos) = formatted.find(exp_char) {
            let (mantissa, exponent) = formatted.split_at(exp_pos);
            let exp_str = &exponent[1..];

            let (sign, exp_digits) = if exp_str.starts_with('+') || exp_str.starts_with('-') {
                let s = exp_str.chars().next().unwrap();
                (s.to_string(), &exp_str[1..])
            } else {
                ("+".to_string(), exp_str)
            };

            let exp_num = exp_digits.parse::<i32>().unwrap_or(0);
            let formatted_exp = format!("{:03}", exp_num);

            let mut res = format!("{}{}{}{}", mantissa, exp_char, sign, formatted_exp);
            // Remove trailing zeros from mantissa
            if res.contains('.') {
                if let Some(e_pos) = res.find(exp_char) {
                    let mantissa_part = &res[..e_pos];
                    let exp_part = &res[e_pos..];
                    let trimmed = mantissa_part.trim_end_matches('0').trim_end_matches('.');
                    res = format!("{}{}", trimmed, exp_part);
                }
            }
            res
        } else {
            formatted
        }
    } else {
        // Use fixed notation
        let mut res = format!("{:.prec$}", num, prec = precision - 1);
        // Remove trailing zeros
        if res.contains('.') {
            res = res.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        res
    };

    // Add sign
    if !result.starts_with('-') {
        if plus_sign {
            result.insert(0, '+');
        } else if space_sign {
            result.insert(0, ' ');
        }
    }

    buf.push_str(&result);
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
    let has_modifiers =
        !flags.is_empty() && (flags.chars().any(|c| c.is_ascii_digit()) || flags.contains('.'));

    // If there's width/precision modifiers and string contains \0, error
    if has_modifiers && s_content.contains('\0') {
        return Err(l.error("string contains zeros".to_string()));
    }

    // Check for precision (e.g., %.20s means max 20 chars, %.s means 0 chars)
    let final_str = if let Some(dot_pos) = flags.find('.') {
        let prec_str = &flags[dot_pos + 1..];
        let prec_str = prec_str
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        let precision = if prec_str.is_empty() {
            0 // "." without number means precision 0
        } else {
            prec_str.parse::<usize>().unwrap_or(s_content.len())
        };

        if precision < s_content.len() {
            &s_content[..precision]
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

    // For strings, get bytes - handle both string and binary types
    let bytes = if let Some(s) = arg.as_str() {
        s.as_bytes()
    } else if let Some(b) = arg.as_binary() {
        b
    } else {
        return Err(l.error("no literal representation for value in 'format'".to_string()));
    };

    buf.push('"');
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
        LuaValueKind::String
        | LuaValueKind::Binary
        | LuaValueKind::Table
        | LuaValueKind::Function
        | LuaValueKind::CFunction
        | LuaValueKind::Userdata
        | LuaValueKind::Thread => {
            let ptr = unsafe { arg.value.ptr as usize };
            format!("0x{:x}", ptr)
        }
        _ => "(null)".to_string(),
    };

    // Apply width formatting if specified
    apply_width_format(buf, &ptr_str, flags);
    Ok(())
}

/// Apply width formatting to a string (handles %10s, %-10s etc.)
fn apply_width_format(buf: &mut String, s: &str, flags: &str) {
    // Parse width - find the numeric part
    let left_align = flags.starts_with('-');
    let width_str = flags
        .trim_start_matches('-')
        .trim_start_matches('+')
        .trim_start_matches(' ')
        .trim_start_matches('#')
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
