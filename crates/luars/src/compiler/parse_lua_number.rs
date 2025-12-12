use emmylua_parser::LuaSyntaxToken;

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

pub fn int_token_value(token: &LuaSyntaxToken) -> Result<NumberResult, String> {
    let text = token.text();
    let repr = if text.starts_with("0x") || text.starts_with("0X") {
        IntegerRepr::Hex
    } else if text.starts_with("0b") || text.starts_with("0B") {
        IntegerRepr::Bin
    } else {
        IntegerRepr::Normal
    };

    // 检查是否有无符号后缀并去除后缀
    let mut is_luajit_unsigned = false;
    let mut suffix_count = 0;
    for c in text.chars().rev() {
        if c == 'u' || c == 'U' {
            is_luajit_unsigned = true;
            suffix_count += 1;
        } else if c == 'l' || c == 'L' {
            suffix_count += 1;
        } else {
            break;
        }
    }

    let text = &text[..text.len() - suffix_count];

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
                } else if let Ok(f) = text.parse::<f64>() {
                    return Ok(NumberResult::Float(f));
                }
                
                Err(format!(
                    "malformed number",
                ))
            } else {
                Err(format!(
                    "Failed to parse integer literal '{}': {}",
                    token.text(),
                    e
                ))
            }
        }
    }
}
