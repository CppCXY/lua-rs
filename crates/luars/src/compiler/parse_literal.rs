use crate::compiler::parser::LuaTokenKind;

enum IntegerRepr {
    Normal,
    Hex,
    Bin,
}

pub enum NumberResult {
    Int(i64),
    Uint(u64),
    Float(f64),
}

pub fn parse_int_token_value(num_text: &str) -> Result<NumberResult, String> {
    let repr = if num_text.starts_with("0x") || num_text.starts_with("0X") {
        IntegerRepr::Hex
    } else if num_text.starts_with("0b") || num_text.starts_with("0B") {
        IntegerRepr::Bin
    } else {
        IntegerRepr::Normal
    };

    // 检查是否有无符号后缀并去除后缀
    let mut is_luajit_unsigned = false;
    let mut suffix_count = 0;
    for c in num_text.chars().rev() {
        if c == 'u' || c == 'U' {
            is_luajit_unsigned = true;
            suffix_count += 1;
        } else if c == 'l' || c == 'L' {
            suffix_count += 1;
        } else {
            break;
        }
    }

    let text = &num_text[..num_text.len() - suffix_count];

    // 首先尝试解析为有符号整数
    let signed_value = match repr {
        IntegerRepr::Hex => {
            let text = &text[2..];
            i64::from_str_radix(text, 16)
        }
        IntegerRepr::Bin => {
            let text = &text[2..];
            i64::from_str_radix(text, 2)
        }
        IntegerRepr::Normal => text.parse::<i64>(),
    };

    match signed_value {
        Ok(value) => Ok(NumberResult::Int(value)),
        Err(e) => {
            // 按照Lua的行为：如果整数溢出，尝试解析为浮点数
            if matches!(
                *e.kind(),
                std::num::IntErrorKind::NegOverflow | std::num::IntErrorKind::PosOverflow
            ) {
                // 如果是luajit无符号整数，尝试解析为u64
                if is_luajit_unsigned {
                    let unsigned_value = match repr {
                        IntegerRepr::Hex => {
                            let text = &text[2..];
                            u64::from_str_radix(text, 16)
                        }
                        IntegerRepr::Bin => {
                            let text = &text[2..];
                            u64::from_str_radix(text, 2)
                        }
                        IntegerRepr::Normal => text.parse::<u64>(),
                    };

                    if let Ok(value) = unsigned_value {
                        return Ok(NumberResult::Uint(value));
                    }
                } else {
                    // Lua 5.4行为：对于十六进制/二进制整数溢出，解析为u64然后reinterpret为i64
                    // 例如：0xFFFFFFFFFFFFFFFF = -1
                    if matches!(repr, IntegerRepr::Hex | IntegerRepr::Bin) {
                        let unsigned_value = match repr {
                            IntegerRepr::Hex => {
                                let text = &text[2..];
                                u64::from_str_radix(text, 16)
                            }
                            IntegerRepr::Bin => {
                                let text = &text[2..];
                                u64::from_str_radix(text, 2)
                            }
                            _ => unreachable!(),
                        };

                        if let Ok(value) = unsigned_value {
                            // Reinterpret u64 as i64 (补码转换)
                            return Ok(NumberResult::Int(value as i64));
                        } else {
                            // 超过64位，转换为浮点数
                            // 例如：0x13121110090807060504030201
                            let hex_str = match repr {
                                IntegerRepr::Hex => &text[2..],
                                IntegerRepr::Bin => &text[2..],
                                _ => unreachable!(),
                            };

                            // 手动将十六进制转为浮点数
                            let base = if matches!(repr, IntegerRepr::Hex) {
                                16.0
                            } else {
                                2.0
                            };
                            let mut result = 0.0f64;
                            for c in hex_str.chars() {
                                if let Some(digit) = c.to_digit(base as u32) {
                                    result = result * base + (digit as f64);
                                }
                            }
                            return Ok(NumberResult::Float(result));
                        }
                    } else if let Ok(f) = text.parse::<f64>() {
                        // 十进制整数溢出，解析为浮点数
                        return Ok(NumberResult::Float(f));
                    }
                }

                Err(format!("malformed number",))
            } else {
                Err(format!(
                    "Failed to parse integer literal '{}': {}",
                    num_text, e
                ))
            }
        }
    }
}

pub fn parse_float_token_value(num_text: &str) -> Result<f64, String> {
    let hex = num_text.starts_with("0x") || num_text.starts_with("0X");

    // This section handles the parsing of hexadecimal floating-point numbers.
    // Hexadecimal floating-point literals are of the form 0x1.8p3, where:
    // - "0x1.8" is the significand (integer and fractional parts in hexadecimal)
    // - "p3" is the exponent (in decimal, base 2 exponent)
    let value = if hex {
        let hex_float_text = &num_text[2..];
        let exponent_position = hex_float_text
            .find('p')
            .or_else(|| hex_float_text.find('P'));
        let (float_part, exponent_part) = if let Some(pos) = exponent_position {
            (&hex_float_text[..pos], &hex_float_text[(pos + 1)..])
        } else {
            (hex_float_text, "")
        };

        let (integer_part, fraction_value) = if let Some(dot_pos) = float_part.find('.') {
            let (int_part, frac_part) = float_part.split_at(dot_pos);
            let int_value = if !int_part.is_empty() {
                i64::from_str_radix(int_part, 16).unwrap_or(0)
            } else {
                0
            };
            let frac_part = &frac_part[1..];
            let frac_value = if !frac_part.is_empty() {
                let frac_part_value = i64::from_str_radix(frac_part, 16).unwrap_or(0);
                frac_part_value as f64 * 16f64.powi(-(frac_part.len() as i32))
            } else {
                0.0
            };
            (int_value, frac_value)
        } else {
            (i64::from_str_radix(float_part, 16).unwrap_or(0), 0.0)
        };

        let mut value = integer_part as f64 + fraction_value;
        if !exponent_part.is_empty()
            && let Ok(exp) = exponent_part.parse::<i32>()
        {
            value *= 2f64.powi(exp);
        }
        value
    } else {
        let (float_part, exponent_part) =
            if let Some(pos) = num_text.find('e').or_else(|| num_text.find('E')) {
                (&num_text[..pos], &num_text[(pos + 1)..])
            } else {
                (num_text, "")
            };

        let mut value = float_part
            .parse::<f64>()
            .map_err(|e| format!("Failed to parse float literal '{}': {}", num_text, e))?;

        if !exponent_part.is_empty()
            && let Ok(exp) = exponent_part.parse::<i32>()
        {
            value *= 10f64.powi(exp);
        }
        value
    };

    Ok(value)
}

pub fn parse_string_token_value(text: &str, kind: LuaTokenKind) -> Result<String, String> {
    match kind {
        LuaTokenKind::TkString => normal_string_value(text),
        LuaTokenKind::TkLongString => long_string_value(text),
        _ => unreachable!(),
    }
}

fn long_string_value(text: &str) -> Result<String, String> {
    if text.len() < 4 {
        return Err(format!("String too short"));
    }

    let mut equal_num = 0;
    let mut i = 0;
    let mut chars = text.char_indices();

    // check first char
    if let Some((_, first_char)) = chars.next() {
        if first_char != '[' {
            return Err(format!(
                "Invalid long string start, expected '[', found '{}'",
                first_char
            ));
        }
    } else {
        return Err(format!("Invalid long string start"));
    }

    for (idx, c) in chars.by_ref() {
        // calc eq num
        if c == '=' {
            equal_num += 1;
        } else if c == '[' {
            i = idx + 1;
            break;
        } else {
            return Err(format!(
                "Invalid long string start, expected '[', found '{}'",
                c
            ));
        }
    }

    // check string len is enough
    if text.len() < i + equal_num + 2 {
        return Err(format!("Long string too short"));
    }

    // lua special rule for long string
    if let Some((_, first_content_char)) = chars.next() {
        if first_content_char == '\r' {
            if let Some((_, next_char)) = chars.next() {
                if next_char == '\n' {
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if first_content_char == '\n' {
            i += 1;
        }
    }

    let content = &text[i..(text.len() - equal_num - 2)];

    Ok(content.to_string())
}

fn normal_string_value(text: &str) -> Result<String, String> {
    if text.len() < 2 {
        return Ok(String::new());
    }

    let mut result = String::with_capacity(text.len() - 2);
    let mut chars = text.chars().peekable();
    let delimiter = chars.next().unwrap();

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next_char) = chars.next() {
                    match next_char {
                        'a' => result.push('\u{0007}'), // Bell
                        'b' => result.push('\u{0008}'), // Backspace
                        'f' => result.push('\u{000C}'), // Formfeed
                        'n' => result.push('\n'),       // Newline
                        'r' => result.push('\r'),       // Carriage return
                        't' => result.push('\t'),       // Horizontal tab
                        'v' => result.push('\u{000B}'), // Vertical tab
                        'x' => {
                            // Hexadecimal escape sequence
                            let hex = chars.by_ref().take(2).collect::<String>();
                            if hex.len() == 2 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                                if let Ok(value) = u8::from_str_radix(&hex, 16) {
                                    result.push(value as char);
                                }
                            } else {
                                return Err(format!(
                                    "Invalid hexadecimal escape sequence '\\x{}'",
                                    hex
                                ));
                            }
                        }
                        'u' => {
                            // Unicode escape sequence
                            if let Some('{') = chars.next() {
                                let unicode_hex =
                                    chars.by_ref().take_while(|c| *c != '}').collect::<String>();
                                if let Ok(code_point) = u32::from_str_radix(&unicode_hex, 16) {
                                    if let Some(unicode_char) = std::char::from_u32(code_point) {
                                        result.push(unicode_char);
                                    } else {
                                        return Err(format!(
                                            "Invalid unicode escape sequence '\\u{{{}}}'",
                                            unicode_hex
                                        ));
                                    }
                                }
                            }
                        }
                        '0'..='9' => {
                            // Decimal escape sequence
                            let mut dec = String::new();
                            dec.push(next_char);
                            for _ in 0..2 {
                                if let Some(digit) = chars.peek() {
                                    if digit.is_ascii_digit() {
                                        dec.push(*digit);
                                    } else {
                                        break;
                                    }
                                    chars.next();
                                }
                            }
                            if let Ok(value) = dec.parse::<u8>() {
                                result.push(value as char);
                            }
                        }
                        '\\' | '\'' | '\"' => result.push(next_char),
                        'z' => {
                            // Skip whitespace
                            while let Some(c) = chars.peek() {
                                if !c.is_whitespace() {
                                    break;
                                }
                                chars.next();
                            }
                        }
                        '\r' | '\n' => {
                            result.push(next_char);
                        }
                        _ => {
                            return Err(format!(
                                "Invalid escape sequence '\\{}'",
                                next_char
                            ));
                        }
                    }
                }
            }
            _ => {
                if c == delimiter {
                    break;
                }
                result.push(c);
            }
        }
    }

    Ok(result)
}
