use crate::{LuaResult, lua_vm::LuaState};

/// string.format(formatstring, ...) - Format string (simplified)
pub fn string_format(l: &mut LuaState) -> LuaResult<usize> {
    let format_str_value = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'string.format' (string expected)".to_string())
    })?;
    let Some(string_id) = format_str_value.as_string_id() else {
        return Err(l.error("bad argument #1 to 'string.format' (string expected)".to_string()));
    };

    // Collect all arguments first to avoid borrow conflicts
    let args = l.get_args();

    // Copy the format string to avoid holding a borrow on vm throughout the loop
    let format = {
        let vm = l.vm_mut();
        let Some(format_str) = vm.object_pool.get_string(string_id) else {
            return Err(l.error("bad argument #1 to 'string.format' (string expected)".to_string()));
        };
        format_str.as_str().to_string()
    }; // vm borrow ends here

    let mut result = String::new();
    let mut arg_index = 1; // Index into args vec (0-based, args[0] is format string, args[1] is first arg)
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            if chars.peek().is_some() {
                // Parse format flags and width
                let mut flags = String::new();
                let mut has_format_char = false;

                // Collect format specifier (flags, width, precision)
                while let Some(&c) = chars.peek() {
                    if c == '-'
                        || c == '+'
                        || c == ' '
                        || c == '#'
                        || c == '0'
                        || c.is_numeric()
                        || c == '.'
                    {
                        flags.push(c);
                        chars.next();
                    } else {
                        has_format_char = true;
                        break;
                    }
                }

                if !has_format_char {
                    return Err(l.error("incomplete format string".to_string()));
                }

                let format_char = chars.next().unwrap();

                match format_char {
                    '%' => {
                        result.push('%');
                    }
                    'c' => {
                        // Character
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val.as_integer().ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (number expected)",
                                arg_index + 1
                            ))
                        })?;
                        if num >= 0 && num <= 255 {
                            result.push(num as u8 as char);
                        } else {
                            return Err(l.error(format!(
                                "bad argument #{} to 'format' (invalid value for '%%c')",
                                arg_index + 1
                            )));
                        }
                        arg_index += 1;
                    }
                    'd' | 'i' => {
                        // Integer
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{}", num));
                        arg_index += 1;
                    }
                    'o' => {
                        // Octal
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:o}", num));
                        arg_index += 1;
                    }
                    'u' => {
                        // Unsigned integer
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{}", num as u64));
                        arg_index += 1;
                    }
                    'x' => {
                        // Lowercase hexadecimal
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:x}", num));
                        arg_index += 1;
                    }
                    'X' => {
                        // Uppercase hexadecimal
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_integer()
                            .or_else(|| val.as_number().map(|n| n as i64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:X}", num));
                        arg_index += 1;
                    }
                    'e' => {
                        // Scientific notation (lowercase)
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:e}", num));
                        arg_index += 1;
                    }
                    'E' => {
                        // Scientific notation (uppercase)
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;
                        result.push_str(&format!("{:E}", num));
                        arg_index += 1;
                    }
                    'f' => {
                        // Floating point
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;

                        // Parse precision from flags (e.g., ".2")
                        if let Some(dot_pos) = flags.find('.') {
                            let precision_str = &flags[dot_pos + 1..];
                            if let Ok(precision) = precision_str.parse::<usize>() {
                                result.push_str(&format!(
                                    "{:.precision$}",
                                    num,
                                    precision = precision
                                ));
                            } else {
                                result.push_str(&format!("{}", num));
                            }
                        } else {
                            result.push_str(&format!("{}", num));
                        }
                        arg_index += 1;
                    }
                    'g' => {
                        // Automatic format (lowercase) - use shorter of %e or %f
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;

                        // Use scientific notation for very large or very small numbers
                        if num.abs() < 0.0001 || num.abs() >= 1e10 {
                            result.push_str(&format!("{:e}", num));
                        } else {
                            result.push_str(&format!("{}", num));
                        }
                        arg_index += 1;
                    }
                    'G' => {
                        // Automatic format (uppercase) - use shorter of %E or %f
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let num = val
                            .as_number()
                            .or_else(|| val.as_integer().map(|i| i as f64))
                            .ok_or_else(|| {
                                l.error(format!(
                                    "bad argument #{} to 'format' (number expected)",
                                    arg_index + 1
                                ))
                            })?;

                        // Use scientific notation for very large or very small numbers
                        if num.abs() < 0.0001 || num.abs() >= 1e10 {
                            result.push_str(&format!("{:E}", num));
                        } else {
                            result.push_str(&format!("{}", num));
                        }
                        arg_index += 1;
                    }
                    's' => {
                        // String
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;

                        // Need to get string from object pool
                        let s = if let Some(str_id) = val.as_string_id() {
                            let vm = l.vm_mut();
                            vm.object_pool
                                .get_string(str_id)
                                .map(|s| s.as_str().to_string())
                                .unwrap_or_else(|| "[invalid string]".to_string())
                        } else if let Some(n) = val.as_integer() {
                            n.to_string()
                        } else if let Some(n) = val.as_number() {
                            n.to_string()
                        } else {
                            // Use simple type_name for non-string values
                            let vm = l.vm_mut();
                            vm.value_to_string_raw(&val)
                        };

                        result.push_str(&s);
                        arg_index += 1;
                    }
                    'q' => {
                        // Quoted string
                        let val = args.get(arg_index).ok_or_else(|| {
                            l.error(format!(
                                "bad argument #{} to 'format' (no value)",
                                arg_index + 1
                            ))
                        })?;
                        let Some(str_id) = val.as_string_id() else {
                            return Err(l.error(format!(
                                "bad argument #{} to 'format' (string expected)",
                                arg_index + 1
                            )));
                        };

                        let s_str = {
                            let vm = l.vm_mut();
                            let Some(s) = vm.object_pool.get_string(str_id) else {
                                return Err(l.error(format!(
                                    "bad argument #{} to 'format' (string expected)",
                                    arg_index + 1
                                )));
                            };
                            s.as_str().to_string()
                        }; // vm borrow ends here

                        result.push('"');
                        for ch in s_str.chars() {
                            match ch {
                                '"' => result.push_str("\\\""),
                                '\\' => result.push_str("\\\\"),
                                '\n' => result.push_str("\\n"),
                                '\r' => result.push_str("\\r"),
                                '\t' => result.push_str("\\t"),
                                _ if ch.is_control() => result.push_str(&format!("\\{}", ch as u8)),
                                _ => result.push(ch),
                            }
                        }
                        result.push('"');
                        arg_index += 1;
                    }
                    _ => {
                        return Err(
                            l.error(format!("invalid option '%{}' to 'format'", format_char))
                        );
                    }
                }
            } else {
                return Err(l.error("incomplete format string".to_string()));
            }
        } else {
            result.push(ch);
        }
    }

    let result_str = l.create_string_owned(result);
    l.push_value(result_str)?;
    Ok(1)
}
