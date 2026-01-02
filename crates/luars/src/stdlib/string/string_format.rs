/// Optimized string.format implementation
/// Reduced from 400+ lines to ~200 lines with better performance
use crate::{LuaResult, LuaValue, lua_vm::LuaState};

/// string.format(formatstring, ...) - Format with various specifiers
pub fn string_format(l: &mut LuaState) -> LuaResult<usize> {
    // Get format string
    let format_str_value = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'format' (string expected)".to_string()))?;
    
    let format_str_id = format_str_value.as_string_id()
        .ok_or_else(|| l.error("bad argument #1 to 'format' (string expected)".to_string()))?;

    // Copy format string once to avoid borrow conflicts
    let format = {
        let vm = l.vm_mut();
        vm.object_pool.get_string(format_str_id)
            .map(|s| s.as_str().to_string())
            .ok_or_else(|| l.error("invalid string".to_string()))?
    };

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
        let fmt_char = chars.next()
            .ok_or_else(|| l.error("incomplete format".to_string()))?;

        // Get argument
        let arg = args.get(arg_index)
            .ok_or_else(|| l.error(format!("bad argument #{} to 'format' (no value)", arg_index + 1)))?;
        arg_index += 1;

        // Format based on type
        match fmt_char {
            'c' => format_char(&mut result, arg, l)?,
            'd' | 'i' => format_int(&mut result, arg, l)?,
            'o' => format_octal(&mut result, arg, &flags, l)?,
            'u' => format_uint(&mut result, arg, l)?,
            'x' => format_hex(&mut result, arg, &flags, false, l)?,
            'X' => format_hex(&mut result, arg, &flags, true, l)?,
            'e' => format_sci(&mut result, arg, &flags, false, l)?,
            'E' => format_sci(&mut result, arg, &flags, true, l)?,
            'f' => format_float(&mut result, arg, &flags, l)?,
            'g' => format_auto(&mut result, arg, false, l)?,
            'G' => format_auto(&mut result, arg, true, l)?,
            's' => format_string(&mut result, arg, l)?,
            'q' => format_quoted(&mut result, arg, l)?,
            _ => return Err(l.error(format!("invalid option '%{}' to 'format'", fmt_char))),
        }
    }

    let result_str = l.create_string_owned(result);
    l.push_value(result_str)?;
    Ok(1)
}

// Helper functions - all inline for performance

#[inline]
fn get_num(arg: &LuaValue, l: &LuaState) -> Result<f64, String> {
    arg.as_number()
        .or_else(|| arg.as_integer().map(|i| i as f64))
        .ok_or_else(|| "bad argument to 'format' (number expected)".to_string())
}

#[inline]
fn get_int(arg: &LuaValue, l: &LuaState) -> Result<i64, String> {
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
fn format_int(buf: &mut String, arg: &LuaValue, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    buf.push_str(&num.to_string());
    Ok(())
}

#[inline]
fn format_octal(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    let s = format!("{:o}", num);
    if flags.contains('#') && !s.starts_with('0') {
        buf.push('0');
    }
    buf.push_str(&s);
    Ok(())
}

#[inline]
fn format_uint(buf: &mut String, arg: &LuaValue, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    buf.push_str(&(num as u64).to_string());
    Ok(())
}

#[inline]
fn format_hex(buf: &mut String, arg: &LuaValue, flags: &str, upper: bool, l: &mut LuaState) -> LuaResult<()> {
    let num = get_int(arg, l).map_err(|e| l.error(e))?;
    
    if flags.contains('#') && num != 0 {
        buf.push_str(if upper { "0X" } else { "0x" });
    }
    
    if upper {
        buf.push_str(&format!("{:X}", num));
    } else {
        buf.push_str(&format!("{:x}", num));
    }
    Ok(())
}

#[inline]
fn format_sci(buf: &mut String, arg: &LuaValue, flags: &str, upper: bool, l: &mut LuaState) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;
    
    // Parse precision from flags (e.g., ".2")
    if let Some(dot_pos) = flags.find('.') {
        if let Ok(prec) = flags[dot_pos + 1..].trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<usize>() {
            if upper {
                buf.push_str(&format!("{:.prec$E}", num, prec = prec));
            } else {
                buf.push_str(&format!("{:.prec$e}", num, prec = prec));
            }
            return Ok(());
        }
    }
    
    if upper {
        buf.push_str(&format!("{:E}", num));
    } else {
        buf.push_str(&format!("{:e}", num));
    }
    Ok(())
}

#[inline]
fn format_float(buf: &mut String, arg: &LuaValue, flags: &str, l: &mut LuaState) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;
    
    // Parse precision from flags (e.g., ".2")
    if let Some(dot_pos) = flags.find('.') {
        if let Ok(prec) = flags[dot_pos + 1..].trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<usize>() {
            buf.push_str(&format!("{:.prec$}", num, prec = prec));
            return Ok(());
        }
    }
    
    buf.push_str(&num.to_string());
    Ok(())
}

#[inline]
fn format_auto(buf: &mut String, arg: &LuaValue, upper: bool, l: &mut LuaState) -> LuaResult<()> {
    let num = get_num(arg, l).map_err(|e| l.error(e))?;
    
    // Use scientific for very large/small numbers
    if num.abs() < 0.0001 || num.abs() >= 1e10 {
        if upper {
            buf.push_str(&format!("{:E}", num));
        } else {
            buf.push_str(&format!("{:e}", num));
        }
    } else {
        buf.push_str(&num.to_string());
    }
    Ok(())
}

#[inline]
fn format_string(buf: &mut String, arg: &LuaValue, l: &mut LuaState) -> LuaResult<()> {
    if let Some(str_id) = arg.as_string_id() {
        let s = l.vm_mut().object_pool.get_string(str_id)
            .map(|s| s.as_str().to_string())
            .ok_or_else(|| l.error("invalid string".to_string()))?;
        buf.push_str(&s);
    } else if let Some(n) = arg.as_integer() {
        buf.push_str(&n.to_string());
    } else if let Some(n) = arg.as_number() {
        buf.push_str(&n.to_string());
    } else {
        let s = l.vm_mut().value_to_string_raw(arg);
        buf.push_str(&s);
    }
    Ok(())
}

#[inline]
fn format_quoted(buf: &mut String, arg: &LuaValue, l: &mut LuaState) -> LuaResult<()> {
    let str_id = arg.as_string_id()
        .ok_or_else(|| l.error("bad argument to 'format' (string expected for %q)".to_string()))?;
    
    let s = l.vm_mut().object_pool.get_string(str_id)
        .map(|s| s.as_str().to_string())
        .ok_or_else(|| l.error("invalid string".to_string()))?;
    
    buf.push('"');
    for ch in s.chars() {
        match ch {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if c.is_control() => buf.push_str(&format!("\\{}", c as u8)),
            c => buf.push(c),
        }
    }
    buf.push('"');
    Ok(())
}
