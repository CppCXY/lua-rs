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
            // TODO support this
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
