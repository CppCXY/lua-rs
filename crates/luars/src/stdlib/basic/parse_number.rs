use crate::LuaValue;

pub fn parse_lua_number(s: &str) -> LuaValue {
    let s = s.trim();
    if s.is_empty() || s.contains('\0') {
        return LuaValue::nil();
    }

    // Handle sign
    let (sign, rest) = if let Some(rest) = s.strip_prefix('-') {
        (-1i64, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (1i64, rest)
    } else {
        (1i64, s)
    };

    // No whitespace allowed between sign and digits
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

        // Plain hex integer — parse with wrapping (C Lua wraps on overflow)
        let mut result: u64 = 0;
        let mut has_digits = false;
        for c in hex_part.chars() {
            if c == '_' {
                continue; // allow underscores (Lua 5.5 doesn't, but skip for safety)
            }
            if let Some(d) = c.to_digit(16) {
                result = result.wrapping_mul(16).wrapping_add(d as u64);
                has_digits = true;
            } else {
                // Invalid character
                return LuaValue::nil();
            }
        }
        if has_digits {
            let i = result as i64;
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
    // Reject inf/nan which Rust's parse accepts but Lua doesn't
    if let Ok(f) = s.parse::<f64>()
        && (f.is_finite() || s.contains('.') || has_exponent)
    {
        // Only accept inf/nan if they came from a valid numeric expression
        // (e.g., overflow), not from the literal strings "inf"/"nan"
        let lower = s.to_lowercase();
        let stripped = lower.trim_start_matches(['+', '-']);
        if stripped.starts_with("inf") || stripped.starts_with("nan") {
            return LuaValue::nil();
        }
        return LuaValue::float(f);
    }

    LuaValue::nil()
}

/// Parse hexadecimal float format (e.g., "0x1.8p+1" = 3.0)
/// Format: [integer_part][.fractional_part][p|P[+|-]exponent]
/// Returns Some(f64) on success, None on parse error
///
/// Matches C Lua's `lua_strx2number`: accumulates mantissa as an integer
/// (up to significant digits), tracks the binary exponent separately, so
/// very long digit strings don't cause overflow.
fn parse_hex_float(s: &str) -> Option<f64> {
    // Split by 'p' or 'P' to separate mantissa and exponent
    let (mantissa_str, exp_str) = if let Some(pos) = s.to_lowercase().find('p') {
        (&s[..pos], &s[pos + 1..])
    } else {
        // No exponent, treat whole string as mantissa with exponent 0
        (s, "0")
    };

    // Parse mantissa (hex digits with optional decimal point).
    // Track significand as u64 and binary exponent separately.
    // Leading zeros are not counted as significant digits.
    let mut mantissa: u64 = 0;
    let mut found_dot = false;
    let mut has_digit = false;
    let mut sig_digits = 0; // number of significant hex digits consumed (non-leading-zero)
    let mut exp_adjust: i64 = 0; // binary exponent adjustment
    const MAX_SIG: usize = 15; // 15 hex digits = 60 bits, fits in u64

    for ch in mantissa_str.chars() {
        if ch == '.' {
            if found_dot {
                return None; // Multiple decimal points
            }
            found_dot = true;
        } else if let Some(digit) = ch.to_digit(16) {
            has_digit = true;
            if digit != 0 && sig_digits == 0 {
                // First non-zero digit: start tracking significant digits
                mantissa = digit as u64;
                sig_digits = 1;
                if found_dot {
                    exp_adjust -= 4;
                }
            } else if sig_digits > 0 && sig_digits < MAX_SIG {
                // Accumulating significant digits
                mantissa = mantissa * 16 + digit as u64;
                sig_digits += 1;
                if found_dot {
                    exp_adjust -= 4;
                }
            } else if sig_digits >= MAX_SIG {
                // Beyond precision: skip digit, adjust exponent for integer-part digits
                if !found_dot {
                    exp_adjust += 4;
                }
                // Fractional digits beyond precision are dropped.
            } else {
                // sig_digits == 0 && digit == 0 → leading zero
                // Still need to track exponent for fractional leading zeros
                if found_dot {
                    exp_adjust -= 4;
                }
            }
        } else if !ch.is_whitespace() {
            return None; // Invalid character
        }
    }

    if !has_digit {
        return None;
    }

    // Parse binary exponent (decimal number after 'p')
    let exp_str = exp_str.trim();
    if exp_str.is_empty() && s.to_lowercase().contains('p') {
        return None; // 'p' present but no exponent
    }
    let exp: i64 = if exp_str == "0" || exp_str.is_empty() {
        0
    } else {
        exp_str.parse::<i64>().ok()?
    };

    let total_exp = exp + exp_adjust;
    // Use ldexp-style multiplication; for very large/small exponents, f64 handles
    // overflow to inf and underflow to 0 naturally.
    let result = ldexp(mantissa as f64, total_exp);
    Some(result)
}

/// Multiply a float by 2^exp (ldexp). Handles large exponents that exceed
/// the range of a single multiplication by splitting into steps.
fn ldexp(mut x: f64, mut exp: i64) -> f64 {
    if x == 0.0 || exp == 0 {
        return x;
    }
    // Apply exponent in steps of at most 1023 (max finite 2^n for f64)
    while exp > 1023 {
        x *= 2.0f64.powi(1023);
        exp -= 1023;
        if x.is_infinite() {
            return x;
        }
    }
    while exp < -1074 {
        x *= 2.0f64.powi(-1074);
        exp += 1074;
        if x == 0.0 {
            return x;
        }
    }
    x * (2.0f64).powi(exp as i32)
}
