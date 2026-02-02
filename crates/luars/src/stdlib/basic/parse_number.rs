use crate::LuaValue;

pub fn parse_lua_number(s: &str) -> LuaValue {
    let s = s.trim();
    if s.is_empty() {
        return LuaValue::nil();
    }

    // Handle sign
    let (sign, rest) = if s.starts_with('-') {
        (-1i64, &s[1..])
    } else if s.starts_with('+') {
        (1i64, &s[1..])
    } else {
        (1i64, s)
    };

    let rest = rest.trim_start();

    // Check for hex prefix (0x or 0X)
    if rest.starts_with("0x") || rest.starts_with("0X") {
        let hex_part = &rest[2..];

        // Hex float contains '.' or 'p'/'P' - always treat as float
        if hex_part.contains('.') || hex_part.to_lowercase().contains('p') {
            if let Some(f) = parse_hex_float(hex_part) {
                return LuaValue::float(sign as f64 * f);
            }
            return LuaValue::nil();
        }

        // Plain hex integer
        if let Ok(i) = u64::from_str_radix(hex_part, 16) {
            let i = i as i64;
            return LuaValue::integer(sign * i);
        }
        return LuaValue::nil();
    }

    // Decimal number - determine if integer or float
    let has_dot = rest.contains('.');
    let has_exponent = rest.to_lowercase().contains('e');

    if !has_dot && !has_exponent {
        // Try as integer
        if let Ok(i) = s.parse::<i64>() {
            return LuaValue::integer(i);
        }
    }

    // Try as float (either has '.'/e' or integer parse failed due to overflow)
    if let Ok(f) = s.parse::<f64>() {
        return LuaValue::float(f);
    }

    LuaValue::nil()
}

/// Parse hexadecimal float format (e.g., "0x1.8p+1" = 3.0)
/// Format: [integer_part][.fractional_part][p|P[+|-]exponent]
/// Returns Some(f64) on success, None on parse error
fn parse_hex_float(s: &str) -> Option<f64> {
    // Split by 'p' or 'P' to separate mantissa and exponent
    let (mantissa_str, exp_str) = if let Some(pos) = s.to_lowercase().find('p') {
        (&s[..pos], &s[pos + 1..])
    } else {
        // No exponent, treat whole string as mantissa with exponent 0
        (s, "0")
    };

    // Parse mantissa (hex digits with optional decimal point)
    let mut mantissa = 0.0f64;
    let mut found_dot = false;
    let mut fraction_digits = 0u32;

    for ch in mantissa_str.chars() {
        if ch == '.' {
            if found_dot {
                return None; // Multiple decimal points
            }
            found_dot = true;
        } else if let Some(digit) = ch.to_digit(16) {
            mantissa = mantissa * 16.0 + digit as f64;
            if found_dot {
                fraction_digits += 1;
            }
        } else if !ch.is_whitespace() {
            return None; // Invalid character
        }
    }

    // Apply fractional part scaling (each hex digit after '.' is divided by 16^n)
    if fraction_digits > 0 {
        mantissa /= 16.0f64.powi(fraction_digits as i32);
    }

    // Parse binary exponent (decimal number after 'p')
    let exp_str = exp_str.trim();
    if exp_str.is_empty() && mantissa_str.contains(|c| c == 'p' || c == 'P') {
        return None; // 'p' present but no exponent
    }

    let exponent: i32 = if !exp_str.is_empty() {
        exp_str.parse().ok()?
    } else {
        0
    };

    // Combine: mantissa * 2^exponent
    let result = mantissa * 2.0f64.powi(exponent);

    Some(result)
}
