// Basic library (_G global functions)
// Implements: print, type, assert, error, tonumber, tostring,
// select, ipairs, pairs, next, pcall, xpcall, getmetatable, setmetatable,
// rawget, rawset, rawlen, rawequal, collectgarbage, dofile, loadfile, load
pub mod parse_number;
mod require;

use std::rc::Rc;

use crate::gc::{code_param, decode_param};
use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, LuaValueKind};
use crate::lua_vm::{LuaError, LuaResult, LuaState, get_metatable};
use crate::stdlib::basic::parse_number::parse_lua_number;
use crate::{GcKind, GcState, MAJORMINOR, MINORMAJOR, MINORMUL, PAUSE, STEPMUL, STEPSIZE};
use require::lua_require;

pub fn create_basic_lib() -> LibraryModule {
    crate::lib_module!("_G", {
        "print" => lua_print,
        "type" => lua_type,
        "assert" => lua_assert,
        "error" => lua_error,
        "tonumber" => lua_tonumber,
        "tostring" => lua_tostring,
        "select" => lua_select,
        "ipairs" => lua_ipairs,
        "pairs" => lua_pairs,
        "next" => lua_next,
        "pcall" => lua_pcall,
        "xpcall" => lua_xpcall,
        "getmetatable" => lua_getmetatable,
        "setmetatable" => lua_setmetatable,
        "rawget" => lua_rawget,
        "rawset" => lua_rawset,
        "rawlen" => lua_rawlen,
        "rawequal" => lua_rawequal,
        "collectgarbage" => lua_collectgarbage,
        "require" => lua_require,
        "load" => lua_load,
        "loadfile" => lua_loadfile,
        "dofile" => lua_dofile,
        "warn" => lua_warn,
    })
    .with_value("_VERSION", |vm| {
        vm.create_string_owned(format!("{}", vm.version))
    })
}

/// print(...) - Print values to stdout
fn lua_print(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();
    let mut output = String::new();
    for (index, arg) in args.iter().enumerate() {
        let s = l.to_string(arg)?;
        output.push_str(&s);
        if index < args.len() - 1 {
            output.push('\t');
        }
    }
    println!("{}", output);
    Ok(0)
}

/// type(v) - Return the type of a value as a string
fn lua_type(l: &mut LuaState) -> LuaResult<usize> {
    let value = match l.get_arg(1) {
        Some(v) => v,
        None => {
            return Err(l.error("bad argument #1 to 'type' (value expected)".to_string()));
        }
    };

    let cs = &l.vm_mut().const_strings;
    let result = match value.kind() {
        LuaValueKind::Nil => cs.str_nil,
        LuaValueKind::Boolean => cs.str_boolean,
        LuaValueKind::Integer | LuaValueKind::Float => cs.str_number,
        LuaValueKind::String => cs.str_string,
        LuaValueKind::Binary => cs.str_string, // Binary is also a string type
        LuaValueKind::Table => cs.str_table,
        LuaValueKind::Function
        | LuaValueKind::CFunction
        | LuaValueKind::CClosure
        | LuaValueKind::RClosure => cs.str_function,
        LuaValueKind::Userdata => cs.str_userdata,
        LuaValueKind::Thread => cs.str_thread,
    };

    l.push_value(result)?;
    Ok(1)
}

/// assert(v [, message]) - Raise error if v is false or nil
fn lua_assert(l: &mut LuaState) -> LuaResult<usize> {
    let arg_count = l.arg_count();

    // assert() without arguments: error "value expected"
    if arg_count == 0 {
        return Err(l.error("bad argument #1 to 'assert' (value expected)".to_string()));
    }

    // Get first argument without consuming it
    let condition = l.get_arg(1).unwrap_or_default();

    if !condition.is_truthy() {
        // Check if second argument is present and what type
        let msg_arg = l.get_arg(2);

        if let Some(msg) = msg_arg {
            if msg.is_string() {
                // String message: add source:line prefix like error() does
                let message = l.to_string(&msg)?;
                let where_prefix = lua_where(l, 1);
                let formatted = format!("{}{}", where_prefix, message);
                let err_str = l.create_string(&formatted)?;
                l.error_object = err_str;
                l.error_msg = formatted;
                return Err(LuaError::RuntimeError);
            } else {
                // Non-string: raise as error object (like error(obj, 0))
                let message = l.to_string(&msg)?;
                return Err(l.error_with_object(message, msg));
            }
        }

        // No second argument: default "assertion failed!" with source prefix
        let where_prefix = lua_where(l, 1);
        let formatted = format!("{}assertion failed!", where_prefix);
        let err_str = l.create_string(&formatted)?;
        l.error_object = err_str;
        l.error_msg = formatted;
        return Err(LuaError::RuntimeError);
    }

    // Return all arguments - they are already on stack
    // Just return the count
    Ok(arg_count)
}

/// Helper: compute "source:line: " prefix at the given call level (like luaL_where)
/// Counts ALL frames (C and Lua) for the level, but only returns info for Lua frames.
fn lua_where(l: &LuaState, level: usize) -> String {
    let depth = l.call_depth();
    let mut lvl = level;
    // Start from the frame BELOW the current one (skip the current C frame itself)
    if depth >= 2 {
        let mut i = depth - 2;
        loop {
            // Count ALL frames (C and Lua)
            lvl -= 1;
            if lvl == 0 {
                let ci = l.get_call_info(i);
                // Only extract info from Lua frames
                if ci.is_lua()
                    && let Some(func_obj) = ci.func.as_lua_function()
                {
                    let chunk = func_obj.chunk();
                    let source = chunk.source_name.as_deref().unwrap_or("[string]");
                    let line = if ci.pc > 0 && (ci.pc as usize - 1) < chunk.line_info.len() {
                        chunk.line_info[ci.pc as usize - 1] as usize
                    } else if !chunk.line_info.is_empty() {
                        chunk.line_info[0] as usize
                    } else {
                        0
                    };
                    return if line > 0 {
                        format!("{}:{}: ", source, line)
                    } else {
                        format!("{}: ", source)
                    };
                }
                // C frame at target level: no line info available
                break;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
    }
    String::new()
}

/// error(message) - Raise an error
fn lua_error(l: &mut LuaState) -> LuaResult<usize> {
    let arg = l.get_arg(1).unwrap_or_default();

    // error() with nil or no argument: Lua 5.5 raises nil as the error object
    // The error message becomes "<no error object>" when formatted
    if arg.is_nil() {
        return Err(l.error_with_object("<no error object>".to_string(), LuaValue::nil()));
    }

    let level = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(1);

    if arg.is_string() && level > 0 {
        // Add position info to string error message (like luaL_where)
        let message = l.to_string(&arg)?;
        let where_prefix = lua_where(l, level as usize);

        let formatted_msg = format!("{}{}", where_prefix, message);
        let err_str = l.create_string(&formatted_msg)?;
        l.error_object = err_str;
        // Set error_msg without adding another source prefix (we added it manually)
        l.error_msg = formatted_msg;
        Err(LuaError::RuntimeError)
    } else {
        // Non-string error object or level 0: raise as-is
        // Preserve the original error value
        let message = l.to_string(&arg)?;
        Err(l.error_with_object(message, arg))
    }
}

/// tonumber(e [, base]) - Convert to number
fn lua_tonumber(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("tonumber() requires argument 1".to_string()))?;
    let has_base = l.get_arg(2).is_some();
    let base = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(10);

    if has_base && !(2..=36).contains(&base) {
        return Err(l.error("bad argument #2 to 'tonumber' (base out of range)".to_string()));
    }

    let result = match value.kind() {
        LuaValueKind::Integer if !has_base => value,
        LuaValueKind::Float if !has_base => value,
        LuaValueKind::String => {
            if let Some(s) = value.as_str() {
                let s_str = s.trim();
                if has_base {
                    // Explicit base: parse as base-N integer.
                    // Leading/trailing spaces are allowed; 0x prefix is NOT.
                    // Handle optional leading sign.
                    let (neg, digits) = if let Some(rest) = s_str.strip_prefix('-') {
                        (true, rest.trim_start())
                    } else if let Some(rest) = s_str.strip_prefix('+') {
                        (false, rest.trim_start())
                    } else {
                        (false, s_str)
                    };
                    // Reject empty, strings with embedded whitespace, or null bytes
                    if digits.is_empty() || digits.contains('\0') {
                        LuaValue::nil()
                    } else {
                        // u64 to handle full unsigned range, then cast to i64 (wrapping)
                        let mut result: u64 = 0;
                        let mut valid = true;
                        let mut has_any = false;
                        for ch in digits.chars() {
                            if let Some(d) = ch.to_digit(base as u32) {
                                result = result.wrapping_mul(base as u64).wrapping_add(d as u64);
                                has_any = true;
                            } else {
                                valid = false;
                                break;
                            }
                        }
                        if valid && has_any {
                            let i = result as i64;
                            LuaValue::integer(if neg { i.wrapping_neg() } else { i })
                        } else {
                            LuaValue::nil()
                        }
                    }
                } else {
                    parse_lua_number(s_str)
                }
            } else {
                LuaValue::nil()
            }
        }
        _ => {
            if has_base {
                return Err(l.error(
                    "bad argument #1 to 'tonumber' (string expected, got number)".to_string(),
                ));
            }
            LuaValue::nil()
        }
    };

    l.push_value(result)?;
    Ok(1)
}

/// Format a float value matching Lua 5.5's tostringbuffFloat behavior:
/// First try %.15g (max digits preserving tostring(tonumber(x)) == x),
/// then %.17g if roundtrip fails, and append ".0" if result looks integer-like.
pub(crate) fn lua_float_to_string(n: f64) -> String {
    if n.is_nan() {
        return if n.is_sign_negative() { "-nan" } else { "-nan" }.to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "inf" } else { "-inf" }.to_string();
    }

    // First try: format with roughly 15 significant digits (%.15g equivalent)
    let s = format_g(n, 15);

    // Check if it roundtrips
    let mut result = if s.parse::<f64>().ok() == Some(n) {
        s
    } else {
        // Second try: format with 17 significant digits (%.17g equivalent)
        format_g(n, 17)
    };

    // If result looks like an integer (no '.', 'e', 'E', 'n', 'i'), add ".0"
    if !result.contains('.')
        && !result.contains('e')
        && !result.contains('E')
        && !result.contains('n')
        && !result.contains('i')
    {
        result.push_str(".0");
    }

    result
}

/// Format a float with %.<prec>g semantics (C-style %g formatting)
fn format_g(n: f64, prec: usize) -> String {
    if n == 0.0 {
        return if n.is_sign_negative() { "-0" } else { "0" }.to_string();
    }

    let abs_n = n.abs();
    // Determine the base-10 exponent
    let exp = abs_n.log10().floor() as i32;

    let formatted = if exp >= -4 && exp < prec as i32 {
        // Fixed-point notation: precision = prec - (exp + 1) decimal places
        let decimal_places = (prec as i32 - exp - 1).max(0) as usize;
        format!("{:.prec$}", n, prec = decimal_places)
    } else {
        // Scientific notation: precision = prec - 1 decimal places
        format!("{:.prec$e}", n, prec = prec - 1)
    };

    // Strip trailing zeros after decimal point (matching %g behavior)
    strip_trailing_zeros(&formatted)
}

/// Strip trailing zeros from a formatted number string (matching C's %g behavior)
fn strip_trailing_zeros(s: &str) -> String {
    if let Some(e_pos) = s.find('e').or_else(|| s.find('E')) {
        // Scientific notation: strip zeros between decimal and 'e'
        let (mantissa, exponent) = s.split_at(e_pos);
        let stripped = strip_decimal_zeros(mantissa);
        format!("{}{}", stripped, exponent)
    } else if s.contains('.') {
        strip_decimal_zeros(s)
    } else {
        s.to_string()
    }
}

/// Strip trailing zeros after decimal point, remove point if no digits follow
fn strip_decimal_zeros(s: &str) -> String {
    let trimmed = s.trim_end_matches('0');
    trimmed.strip_suffix('.').unwrap_or(trimmed).to_string()
}

/// tostring(v) - Convert to string
fn lua_tostring(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("tostring() requires argument 1".to_string()))?;

    // Fast path: already a string, return it directly
    if value.is_string() {
        l.push_value(value)?;
        return Ok(1);
    }

    // Fast path: raw integer type — use itoa
    if value.is_integer() {
        let n = value.as_integer_strict().unwrap();
        let mut buf = itoa::Buffer::new();
        let s = buf.format(n);
        let result_value = l.create_string(s)?;
        l.push_value(result_value)?;
        return Ok(1);
    }

    // Fast path: raw float type — use Lua-compatible formatting
    if value.is_float() {
        let n = value.as_number().unwrap();
        let s = lua_float_to_string(n);
        let result_value = l.create_string(&s)?;
        l.push_value(result_value)?;
        return Ok(1);
    }

    // Fast path: nil / bool — pre-interned strings
    if value.is_nil() {
        let result_value = l.vm_mut().const_strings.str_nil;
        l.push_value(result_value)?;
        return Ok(1);
    }
    if let Some(b) = value.as_boolean() {
        let cs = &l.vm_mut().const_strings;
        let result_value = if b { cs.str_true } else { cs.str_false };
        l.push_value(result_value)?;
        return Ok(1);
    }

    // General path: metamethods, functions, tables, etc.
    let result = l.to_string(&value)?;
    let result_value = l.create_string_owned(result)?;
    l.push_value(result_value)?;
    Ok(1)
}

/// select(index, ...) - Return subset of arguments
/// select(index, ...) - Return subset of arguments
/// OPTIMIZED: Use ensure_stack_capacity + push_value_unchecked, cache base
fn lua_select(l: &mut LuaState) -> LuaResult<usize> {
    let index_arg = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'select' (value expected)".to_string()))?;

    // Total args after index
    let total_args = l.arg_count();
    let vararg_count = if total_args > 0 { total_args - 1 } else { 0 };

    // FAST PATH: Check for "#"
    if let Some(s) = index_arg.as_str() {
        if s == "#" {
            let result = LuaValue::integer(vararg_count as i64);
            l.push_value(result)?;
            return Ok(1);
        }
        return Err(l.error("bad argument #1 to 'select' (number expected)".to_string()));
    }

    let index = index_arg
        .as_integer()
        .ok_or_else(|| l.error("bad argument #1 to 'select' (number expected)".to_string()))?;

    if index == 0 {
        return Err(l.error("bad argument #1 to 'select' (index out of range)".to_string()));
    }

    // Calculate start position (1-based to 0-based)
    let start_idx = if index > 0 {
        (index - 1) as usize
    } else {
        let abs_idx = (-index) as usize;
        if abs_idx > vararg_count {
            return Err(l.error("bad argument #1 to 'select' (index out of range)".to_string()));
        }
        vararg_count - abs_idx
    };

    if start_idx >= vararg_count {
        return Ok(0);
    }

    let result_count = vararg_count - start_idx;

    // Ensure stack has room for all results at once, then use unchecked push
    l.ensure_stack_capacity(result_count)?;

    // Cache base to avoid repeated frame lookups
    let frame = &l.call_stack[l.call_depth() - 1];
    let base = frame.base;
    let top = frame.top;

    // first_arg_idx is 1-based: arg 2 + start_idx → stack offset = base + 1 + start_idx
    let stack_start = base + 1 + start_idx;

    for i in 0..result_count {
        let stack_idx = stack_start + i;
        let val = if stack_idx < top && stack_idx < l.stack.len() {
            l.stack[stack_idx]
        } else {
            LuaValue::nil()
        };
        unsafe {
            l.push_value_unchecked(val);
        }
    }

    Ok(result_count)
}

/// ipairs(t) - Return iterator for array part of table
fn lua_ipairs(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'ipairs' (value expected)".to_string()))?;

    if !table_val.is_table() {
        return Err(l.error("bad argument #1 to 'ipairs' (table expected)".to_string()));
    }

    // Return iterator function, table, and 0 (3 values)
    let iter_func = LuaValue::cfunction(ipairs_next);
    l.push_value(iter_func)?;
    l.push_value(table_val)?;
    l.push_value(LuaValue::integer(0))?;
    Ok(3)
}

/// Iterator function for ipairs - Optimized for performance
#[inline]
fn ipairs_next(l: &mut LuaState) -> LuaResult<usize> {
    // SAFETY: ipairs always provides (table, index) as args 1 and 2.
    let (table_val, index_val) = unsafe { (l.get_arg_unchecked(1), l.get_arg_unchecked(2)) };

    if let Some(table) = table_val.as_table()
        && let Some(index) = index_val.as_integer()
    {
        let next_index = index.wrapping_add(1);
        if let Some(value) = table.raw_geti(next_index) {
            unsafe {
                l.push_value_unchecked(LuaValue::integer(next_index));
                l.push_value_unchecked(value);
            }
            return Ok(2);
        } else {
            unsafe {
                l.push_value_unchecked(LuaValue::nil());
            }
            return Ok(1);
        }
    }

    Err(l.error("ipairs iterator: invalid table or index".to_string()))
}

/// pairs(t) - Return iterator for all key-value pairs
/// Supports both tables and userdata. For userdata, calls `lua_next()` on the trait.
fn lua_pairs(l: &mut LuaState) -> LuaResult<usize> {
    let val = l.get_arg(1).ok_or_else(|| {
        l.error("bad argument #1 to 'pairs' (table or userdata expected)".to_string())
    })?;

    if val.is_table() {
        // Return next, table, nil (3 values)
        let next_func = LuaValue::cfunction(lua_next);
        l.push_value(next_func)?;
        l.push_value(val)?;
        l.push_value(LuaValue::nil())?;
        Ok(3)
    } else if val.is_userdata() {
        // Return userdata_next, userdata, nil (3 values)
        let next_func = LuaValue::cfunction(lua_userdata_next);
        l.push_value(next_func)?;
        l.push_value(val)?;
        l.push_value(LuaValue::nil())?;
        Ok(3)
    } else {
        Err(l.error("bad argument #1 to 'pairs' (table or userdata expected)".to_string()))
    }
}

/// Iterator function for userdata pairs().
/// Delegates to `UserDataTrait::lua_next(control)` — a stateless Rust iterator.
/// Returns (next_control, value) or nil when exhausted.
fn lua_userdata_next(l: &mut LuaState) -> LuaResult<usize> {
    let ud_val = unsafe { l.get_arg_unchecked(1) };
    let key_val = l.get_arg(2).unwrap_or_default();

    let ud = ud_val.as_userdata_mut().ok_or_else(|| {
        l.error("bad argument #1 to userdata iterator (userdata expected)".to_string())
    })?;

    // Convert the Lua control variable to UdValue for the trait call
    let control = crate::lua_value::userdata_trait::lua_value_to_udvalue(&key_val);

    match ud.get_trait().lua_next(&control) {
        Some((next_control, value)) => {
            let k = crate::lua_value::userdata_trait::udvalue_to_lua_value(l, next_control)?;
            let v = crate::lua_value::userdata_trait::udvalue_to_lua_value(l, value)?;
            l.push_value(k)?;
            l.push_value(v)?;
            Ok(2)
        }
        None => {
            // Iteration exhausted
            unsafe {
                l.push_value_unchecked(LuaValue::nil());
            }
            Ok(1)
        }
    }
}

/// next(table [, index]) - Return next key-value pair
/// Port of Lua 5.5's luaB_next using luaH_next
fn lua_next(l: &mut LuaState) -> LuaResult<usize> {
    // arg 1 is the table (required), arg 2 is the key (optional, defaults to nil)
    let table_val = unsafe { l.get_arg_unchecked(1) };
    let index_val = l.get_arg(2).unwrap_or_default();

    let result = {
        let table = table_val
            .as_table()
            .ok_or_else(|| l.error("bad argument #1 to 'next' (table expected)".to_string()))?;
        table
            .next(&index_val)
            .map_err(|_| l.error("invalid key to 'next'".to_string()))?
    };

    if let Some((k, v)) = result {
        unsafe {
            l.push_value_unchecked(k);
            l.push_value_unchecked(v);
        }
        Ok(2)
    } else {
        unsafe {
            l.push_value_unchecked(LuaValue::nil());
        }
        Ok(1)
    }
}

/// pcall(f [, arg1, ...]) - Protected call
fn lua_pcall(l: &mut LuaState) -> LuaResult<usize> {
    // Arguments are already on stack from the call:
    // stack: [pcall_func, target_func, arg1, arg2, ...]
    // We need: [target_func, arg1, arg2, ...] and call it

    let arg_count = l.arg_count();
    if arg_count < 1 {
        return Err(l.error("bad argument #1 to 'pcall' (value expected)".to_string()));
    }

    // Get current frame info
    let base = l
        .current_frame()
        .map(|f| f.base)
        .ok_or(LuaError::RuntimeError)?;

    // func is at base+0, args are at base+1..base+arg_count-1
    // We want to call func with arg_count-1 arguments
    let func_idx = base;
    let call_arg_count = arg_count - 1;

    // Call using stack-based API (no Vec allocation!)
    let (success, result_count) = l.pcall_stack_based(func_idx, call_arg_count)?;

    // Results at stack[func_idx..func_idx+result_count], top = func_idx + result_count.
    // Need to return [bool, result1, result2, ...] — shift results right by 1.
    // Push a nil to ensure stack capacity for the extra boolean slot.
    l.push_value(LuaValue::nil())?;

    // In-place shift: move results right by 1, insert boolean at func_idx.
    // Zero allocation, single O(n) copy.
    {
        let stack = l.stack_mut();
        for i in (0..result_count).rev() {
            stack[func_idx + 1 + i] = stack[func_idx + i];
        }
        stack[func_idx] = LuaValue::boolean(success);
    }

    Ok(result_count + 1)
}

/// xpcall(f, msgh [, arg1, ...]) - Protected call with error handler
fn lua_xpcall(l: &mut LuaState) -> LuaResult<usize> {
    // xpcall(f, msgh, arg1, arg2, ...)
    // Stack layout from call:
    //   xpcall's C frame has: base+0=f, base+1=msgh, base+2..=args
    let arg_count = l.arg_count();
    if arg_count < 2 {
        return Err(l.error("bad argument #2 to 'xpcall' (value expected)".to_string()));
    }

    let base = l
        .current_frame()
        .map(|f| f.base)
        .ok_or(LuaError::RuntimeError)?;
    let xpcall_func_pos = l
        .current_frame()
        .map(|f| f.base - f.func_offset)
        .ok_or(LuaError::RuntimeError)?;

    // Rearrange stack for xpcall_stack_based:
    // We want [handler, f, arg1, arg2, ...] starting at xpcall_func_pos.
    //   xpcall_func_pos = handler (was xpcall function itself, overwrite)
    //   xpcall_func_pos+1 = f (the function to protect)
    //   xpcall_func_pos+2.. = args
    //
    // Currently: xpcall_func_pos=xpcall, base+0=f, base+1=msgh, base+2..=args
    // We need: xpcall_func_pos=msgh, xpcall_func_pos+1=f, xpcall_func_pos+2..=args
    let msgh = l.stack_get(base + 1).unwrap_or_default();
    let f = l.stack_get(base).unwrap_or_default();

    // Store handler at xpcall_func_pos
    l.stack_set(xpcall_func_pos, msgh)?;

    // Store function at xpcall_func_pos+1
    l.stack_set(xpcall_func_pos + 1, f)?;

    // Shift args to xpcall_func_pos+2..
    let call_arg_count = arg_count - 2;
    for i in 0..call_arg_count {
        let val = l.stack_get(base + 2 + i).unwrap_or_default();
        l.stack_set(xpcall_func_pos + 2 + i, val)?;
    }
    let func_idx = xpcall_func_pos + 1;
    let handler_idx = xpcall_func_pos;
    l.set_top(func_idx + 1 + call_arg_count)?;

    // Mark current (xpcall's) C frame with CIST_XPCALL
    // so finish_c_frame knows to apply the error handler on error recovery.
    {
        use crate::lua_vm::call_info::call_status::CIST_XPCALL;
        let frame_idx = l.call_depth() - 1;
        let ci = l.get_call_info_mut(frame_idx);
        ci.call_status |= CIST_XPCALL;
    }

    // Call using xpcall_stack_based which calls handler BEFORE unwinding frames
    let (success, result_count) = l.xpcall_stack_based(func_idx, call_arg_count, handler_idx)?;

    if success {
        // Results at func_idx..func_idx+result_count — shift right by 1 for boolean
        l.push_value(LuaValue::nil())?; // extend stack by 1
        {
            let stack = l.stack_mut();
            for i in (0..result_count).rev() {
                stack[func_idx + 1 + i] = stack[func_idx + i];
            }
            stack[func_idx] = LuaValue::boolean(true);
        }
        Ok(result_count + 1)
    } else {
        // Error — handler already called by xpcall_stack_based, result is at func_idx
        let transformed_error = l.stack_get(func_idx).unwrap_or_default();
        l.push_value(LuaValue::boolean(false))?;
        l.push_value(transformed_error)?;
        Ok(2)
    }
}

/// getmetatable(object) - Get metatable
fn lua_getmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'getmetatable' (value expected)".to_string()))?;

    let mt = get_metatable(l, &value);
    match mt {
        Some(mt_val) => {
            // Check for __metatable field - if present, return that instead
            if let Some(table) = mt_val.as_table() {
                let key = l.create_string("__metatable")?;
                if let Some(mm) = table.raw_get(&key)
                    && !mm.is_nil()
                {
                    l.push_value(mm)?;
                    return Ok(1);
                }
            }
            l.push_value(mt_val)?;
        }
        None => {
            l.push_value(LuaValue::nil())?;
        }
    }
    Ok(1)
}

/// setmetatable(table, metatable) - Set metatable
fn lua_setmetatable(l: &mut LuaState) -> LuaResult<usize> {
    let table = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'setmetatable' (value expected)".to_string()))?;
    let metatable = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'setmetatable' (value expected)".to_string()))?;

    if let Some(table_ref) = table.as_table_mut() {
        // Check for __metatable protection on existing metatable
        if let Some(existing_mt) = table_ref.get_metatable()
            && let Some(mt_table) = existing_mt.as_table()
        {
            let key = l.create_string("__metatable")?;
            let has_protection = mt_table.raw_get(&key).is_some_and(|v| !v.is_nil());
            if has_protection {
                return Err(l.error("cannot change a protected metatable".to_string()));
            }
        }

        match metatable.kind() {
            LuaValueKind::Nil => {
                table_ref.set_metatable(None);
            }
            LuaValueKind::Table => {
                table_ref.set_metatable(Some(metatable));
            }
            _ => {
                return Err(
                    l.error("setmetatable() second argument must be a table or nil".to_string())
                );
            }
        }

        // GC write barrier: if the table is BLACK and the new metatable is WHITE,
        // the GC must be notified. barrier_back turns the table back to GRAY
        // so it gets re-traversed and the metatable gets properly marked.
        // Without this, the metatable can be swept (freed) while the table still
        // references it → dangling pointer → heap corruption.
        if let Some(gc_ptr) = table.as_gc_ptr() {
            l.gc_barrier_back(gc_ptr);
        }
    }

    // Lua 5.5: luaC_checkfinalizer - register object if __gc is present
    l.vm_mut().gc.check_finalizer(&table);
    // Return the original table
    l.push_value(table)?;
    Ok(1)
}

/// rawget(table, index) - Get without metamethods
fn lua_rawget(l: &mut LuaState) -> LuaResult<usize> {
    let table = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rawget' (value expected)".to_string()))?;
    let key = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'rawget' (value expected)".to_string()))?;

    let value = table
        .as_table()
        .map(|table_ref| table_ref.raw_get(&key).unwrap_or(LuaValue::nil()));

    if let Some(v) = value {
        l.push_value(v)?;
        return Ok(1);
    }
    Err(l.error("bad argument #1 to 'rawget' (table expected)".to_string()))
}

/// rawset(table, index, value) - Set without metamethods
fn lua_rawset(l: &mut LuaState) -> LuaResult<usize> {
    let table = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rawset' (value expected)".to_string()))?;
    let key = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'rawset' (value expected)".to_string()))?;
    let value = l
        .get_arg(3)
        .ok_or_else(|| l.error("bad argument #3 to 'rawset' (value expected)".to_string()))?;

    if table.is_table() {
        // Check for NaN key
        if key.is_float()
            && let Some(f) = key.as_number()
            && f.is_nan()
        {
            return Err(l.error("table index is NaN".to_string()));
        }
        l.raw_set(&table, key, value);
        l.push_value(table)?;
        return Ok(1);
    }
    Err(l.error("bad argument #1 to 'rawset' (table expected)".to_string()))
}

/// rawlen(v) - Length without metamethods
fn lua_rawlen(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rawlen' (value expected)".to_string()))?;

    let len = match value.kind() {
        LuaValueKind::Table => {
            if let Some(table) = value.as_table() {
                table.len() as i64
            } else {
                return Err(
                    l.error("bad argument #1 to 'rawlen' (table or string expected)".to_string())
                );
            }
        }
        LuaValueKind::String => {
            if let Some(s) = value.as_str() {
                s.len() as i64
            } else {
                return Err(
                    l.error("bad argument #1 to 'rawlen' (table or string expected)".to_string())
                );
            }
        }
        _ => {
            return Err(
                l.error("bad argument #1 to 'rawlen' (table or string expected)".to_string())
            );
        }
    };

    l.push_value(LuaValue::integer(len))?;
    Ok(1)
}

/// rawequal(v1, v2) - Equality without metamethods
fn lua_rawequal(l: &mut LuaState) -> LuaResult<usize> {
    let v1 = l.get_arg(1).unwrap_or_default();
    let v2 = l.get_arg(2).unwrap_or_default();

    let result = v1 == v2;
    l.push_value(LuaValue::boolean(result))?;
    Ok(1)
}

/// collectgarbage([opt [, arg, arg2]]) - Garbage collector control
/// Lua 5.5 version with full parameter support including the new 'param' option
fn lua_collectgarbage(l: &mut LuaState) -> LuaResult<usize> {
    // Check if GC is internally stopped (like Lua 5.5's gcstp & (GCSTPGC | GCSTPCLS))
    // GCSTPGC means GC is currently running (prevents reentrancy)
    // From lapi.c line 1174: if (g->gcstp & (GCSTPGC | GCSTPCLS)) return -1;
    if l.vm_mut().gc.gc_stopem {
        // Return nil (false) to indicate GC is currently running
        // In Lua 5.5, lua_gc returns -1, which is not returned to Lua code
        // The Lua manual says collectgarbage returns false if it cannot run
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    }

    let arg1 = l.get_arg(1);

    let opt = match &arg1 {
        Some(v) if v.is_nil() => "collect".to_string(),
        Some(v) => match v.as_str() {
            Some(s) => s.to_string(),
            None => return Err(crate::stdlib::debug::arg_typeerror(l, 1, "string", v)),
        },
        None => "collect".to_string(),
    };

    match opt.as_str() {
        "collect" => {
            l.collect_garbage()?;
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "count" => {
            let gc = &l.vm_mut().gc;
            let real_bytes = gc.total_bytes - gc.gc_debt; // gettotalbytes
            let kb = real_bytes.max(0) as f64 / 1024.0;
            l.push_value(LuaValue::number(kb))?;
            Ok(1)
        }
        "stop" => {
            // LUA_GCSTOP: Stop collector (like Lua's gcstp = GCSTPUSR)
            l.vm_mut().gc.gc_stopped = true;
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "restart" => {
            // LUA_GCRESTART: Restart collector
            // From lapi.c: luaE_setdebt(g, 0); g->gcstp = 0;
            // Exactly like Lua 5.5: debt=0 will trigger GC on next check
            l.vm_mut().gc.gc_stopped = false;
            l.vm_mut().gc.set_debt(0);
            l.push_value(LuaValue::integer(0))?;
            Ok(1)
        }
        "step" => {
            // LUA_GCSTEP: Single step with optional size argument (in bytes)
            //
            // From lapi.c (Lua 5.5): lines 1202-1214
            // ```c
            // case LUA_GCSTEP: {
            //   lu_byte oldstp = g->gcstp;
            //   l_mem n = cast(l_mem, va_arg(argp, size_t));
            //   int work = 0;
            //   g->gcstp = 0;
            //   if (n <= 0)
            //     n = g->GCdebt;
            //   luaE_setdebt(g, g->GCdebt - n);
            //   luaC_condGC(L, (void)0, work = 1);
            //   if (work && g->gcstate == GCSpause)
            //     res = 1;
            //   g->gcstp = oldstp;
            //   break;
            // }
            // ```
            let arg2 = l.get_arg(2);
            let n_arg = arg2.and_then(|v| v.as_integer()).unwrap_or(0);

            // lu_byte oldstp = g->gcstp;
            let old_stopped = l.vm_mut().gc.gc_stopped;

            // l_mem n = cast(l_mem, va_arg(argp, size_t));
            // if (n <= 0) n = g->GCdebt;
            let gc = &l.vm_mut().gc;
            let n = if n_arg <= 0 {
                gc.gc_debt
            } else {
                n_arg as isize
            };

            // int work = 0;
            let mut work = false;

            // g->gcstp = 0;
            l.vm_mut().gc.gc_stopped = false;

            // luaE_setdebt(g, g->GCdebt - n);
            // Use saturating subtraction to avoid overflow
            let old_debt = l.vm_mut().gc.gc_debt;
            l.vm_mut().gc.set_debt(old_debt.saturating_sub(n));

            // luaC_condGC(L, (void)0, work = 1);
            // Expands to: if (G(L)->GCdebt <= 0) { luaC_step(L); work = 1; }
            if l.check_gc()? {
                work = true;
            }

            // g->gcstp = oldstp;
            l.vm_mut().gc.gc_stopped = old_stopped;

            // if (work && g->gcstate == GCSpause) res = 1;
            let completed = work && matches!(l.vm_mut().gc.gc_state, GcState::Pause);
            l.push_value(LuaValue::boolean(completed))?;

            Ok(1)
        }
        "isrunning" => {
            // LUA_GCISRUNNING: Check if collector is running
            // GC is running if not stopped by user
            let is_running = !l.vm_mut().gc.gc_stopped;
            l.push_value(LuaValue::boolean(is_running))?;
            Ok(1)
        }
        "generational" => {
            // LUA_GCGEN: Switch to generational mode (like luaC_changemode)
            let old_mode = match l.vm_mut().gc.gc_kind {
                GcKind::Inc => "incremental",
                GcKind::GenMinor => "generational",
                GcKind::GenMajor => "generational",
            };

            let vm_ptr = l.vm_ptr();
            let vm = unsafe { &mut *vm_ptr };
            vm.gc.change_mode(l, GcKind::GenMinor);

            let mode_value = l.create_string(old_mode)?;
            l.push_value(mode_value)?;
            Ok(1)
        }
        "incremental" => {
            // LUA_GCINC: Switch to incremental mode (like luaC_changemode)
            let old_mode = match l.vm_mut().gc.gc_kind {
                GcKind::Inc => "incremental",
                GcKind::GenMinor => "generational",
                GcKind::GenMajor => "generational",
            };

            let vm_ptr = l.vm_ptr();
            let vm = unsafe { &mut *vm_ptr };
            vm.gc.change_mode(l, GcKind::Inc);

            let mode_value = l.create_string(old_mode)?;
            l.push_value(mode_value)?;
            Ok(1)
        }
        "param" => {
            // LUA_GCPARAM: Get/set GC parameters (NEW in Lua 5.5!)
            let arg2 = l.get_arg(2);
            let arg3 = l.get_arg(3);

            // Get parameter name string
            let param_name = if let Some(v) = arg2 {
                v.as_str().map(|s| s.to_string())
            } else {
                None
            };

            if param_name.is_none() {
                return Err(l.error("collectgarbage 'param': parameter name expected".to_string()));
            }

            let param_name = param_name.unwrap();

            // Map parameter name to index
            let param_idx = match param_name.as_str() {
                "minormul" => Some(MINORMUL),     // 0: LUA_GCPMINORMUL
                "majorminor" => Some(MAJORMINOR), // 1: LUA_GCPMAJORMINOR
                "minormajor" => Some(MINORMAJOR), // 2: LUA_GCPMINORMAJOR
                "pause" => Some(PAUSE),           // 3: LUA_GCPPAUSE
                "stepmul" => Some(STEPMUL),       // 4: LUA_GCPSTEPMUL
                "stepsize" => Some(STEPSIZE),     // 5: LUA_GCPSTEPSIZE
                _ => None,
            };

            if param_idx.is_none() {
                return Err(l.error(format!(
                    "collectgarbage 'param': invalid parameter name '{}'",
                    param_name
                )));
            }

            let param_idx = param_idx.unwrap();

            // Get old value and potentially set new value
            let old_value = {
                let vm = l.vm_mut();
                // Decode the compressed parameter to get actual percentage
                let old = decode_param(vm.gc.gc_params[param_idx]);

                // Set new value if provided
                if let Some(new_val) = arg3
                    && let Some(new_int) = new_val.as_integer()
                {
                    // Encode the new value using Lua 5.5's compressed format
                    vm.gc.gc_params[param_idx] = code_param(new_int as u32);
                }

                old
            };

            // Return old value
            l.push_value(LuaValue::integer(old_value as i64))?;
            Ok(1)
        }
        _ => Err(l.error(format!("collectgarbage: invalid option '{}'", opt))),
    }
}

/// load(chunk [, chunkname [, mode [, env]]]) - Load a chunk
fn lua_load(l: &mut LuaState) -> LuaResult<usize> {
    use crate::lua_value::chunk_serializer;

    let chunk_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'load' (value expected)".to_string()))?;

    // Save all arguments before potentially calling reader function
    // because calling the reader will modify the stack
    let chunkname_arg = l.get_arg(2).and_then(|v| v.as_str().map(|s| s.to_string()));
    let mode_arg = l.get_arg(3).and_then(|v| v.as_str().map(|s| s.to_string()));
    let env_arg = l.get_arg(4);

    // Check if chunk is callable (function, cfunction, or table with __call metamethod)
    let is_reader = chunk_val.is_function() || chunk_val.is_table();

    // Get the chunk string or binary data
    let (code_bytes, is_binary) = if is_reader {
        // chunk is a reader function - call it repeatedly to get source
        let mut accumulated = Vec::new();
        let mut is_binary = false;
        let mut first_chunk = true;

        loop {
            // Call the reader function
            l.push_value(chunk_val)?;

            let func_idx = l.get_top() - 1;
            let call_result = l.pcall_stack_based(func_idx, 0);

            let result = match call_result {
                Ok((true, result_count)) => {
                    if result_count > 0 {
                        l.stack_get(func_idx).unwrap_or_default()
                    } else {
                        LuaValue::nil()
                    }
                }
                Ok((false, _)) => {
                    // Error occurred in reader function
                    let error_val = l.stack_get(func_idx).unwrap_or_default();
                    l.set_top(func_idx)?;

                    // Return nil + error message
                    l.push_value(LuaValue::nil())?;
                    let err_msg =
                        l.create_string(&format!("error in reader function: {}", error_val))?;
                    l.push_value(err_msg)?;
                    return Ok(2);
                }
                Err(e) => {
                    // Fatal error, propagate it
                    return Err(e);
                }
            };

            // nil or empty string means end of input
            if result.is_nil() {
                l.set_top(func_idx)?;
                break;
            }

            // Get raw bytes - support both string and binary types
            // IMPORTANT: Use as_binary() first to avoid UTF-8 conversion issues
            // when reading binary bytecode one byte at a time
            let bytes_opt = if let Some(b) = result.as_binary() {
                Some(b)
            } else {
                result.as_str().map(|s| s.as_bytes())
            };

            if let Some(bytes) = bytes_opt {
                if bytes.is_empty() {
                    l.set_top(func_idx)?;
                    break;
                }

                // Check if first byte is binary marker (0x1B for Lua bytecode)
                if first_chunk && !bytes.is_empty() && bytes[0] == 0x1B {
                    is_binary = true;
                }

                // IMPORTANT: Copy the bytes BEFORE calling set_top
                // because set_top may allow GC to run
                accumulated.extend_from_slice(bytes);
                first_chunk = false;

                // Clean up stack
                l.set_top(func_idx)?;
            } else {
                // Reader function returned non-string value (not nil)
                // Return nil + error message like Lua does
                l.set_top(func_idx)?;
                l.push_value(LuaValue::nil())?;
                let err_msg = l.create_string("reader function must return a string")?;
                l.push_value(err_msg)?;
                return Ok(2);
            }
        }

        (accumulated, is_binary)
    } else if let Some(b) = chunk_val.as_binary() {
        let is_binary = b.first() == Some(&0x1B);
        (b.to_vec(), is_binary)
    } else if let Some(s) = chunk_val.as_str() {
        // Check if this is binary bytecode by looking at first byte
        let is_binary = s.as_bytes().first() == Some(&0x1B);
        (s.as_bytes().to_vec(), is_binary)
    } else {
        return Err(l.error("bad argument #1 to 'load' (function or string expected)".to_string()));
    };

    // Optional chunk name for error messages
    // If not provided and chunk is text, use the source code itself (or a prefix)
    let chunkname = if let Some(name) = chunkname_arg {
        name
    } else if !is_binary {
        // For text chunks, use the source code as chunk name (like Lua 5.5)
        // This allows the source to be preserved in the bytecode
        match String::from_utf8(code_bytes.clone()) {
            Ok(s) => s,
            Err(_) => "=(load)".to_string(),
        }
    } else {
        "=(load)".to_string()
    };

    // Optional mode ("b", "t", or "bt")
    let mode = mode_arg.unwrap_or_else(|| "bt".to_string());

    // Validate mode string - must contain only 'b' and/or 't'
    if mode.is_empty() || mode.chars().any(|c| c != 'b' && c != 't') {
        return Err(crate::stdlib::debug::argerror(l, 3, "invalid mode"));
    }

    // Check if mode allows this chunk type
    if is_binary {
        // Binary chunk - mode must allow binary ("b" or "bt")
        if !mode.contains('b') {
            l.push_value(LuaValue::nil())?;
            let err_msg = l.create_string("attempt to load a binary chunk (mode is 'text')")?;
            l.push_value(err_msg)?;
            return Ok(2);
        }
    } else {
        // Text chunk - mode must allow text ("t" or "bt")
        if !mode.contains('t') {
            l.push_value(LuaValue::nil())?;
            let err_msg = l.create_string("attempt to load a text chunk (mode is 'binary')")?;
            l.push_value(err_msg)?;
            return Ok(2);
        }
    }

    // Optional environment table
    let env = env_arg;

    let chunk_result = if is_binary {
        // Deserialize binary bytecode with VM to directly create strings
        let vm = l.vm_mut();
        match chunk_serializer::deserialize_chunk_with_strings_vm(&code_bytes, vm) {
            Ok(chunk) => Ok(chunk),
            Err(e) => Err(format!("binary load error: {}", e)),
        }
    } else {
        // Compile text code using VM's string pool with chunk name
        // Source code should be valid UTF-8
        let code_str = match String::from_utf8(code_bytes.clone()) {
            Ok(s) => s,
            Err(_) => return Err(l.error("source is not valid UTF-8".to_string())),
        };
        let vm = l.vm_mut();
        vm.compile_with_name(&code_str, &chunkname).map_err(|e| {
            // Get the actual error message from VM

            vm.get_error_message(e)
        })
    };

    match chunk_result {
        Ok(chunk) => {
            // Create upvalues for the function
            // According to Lua documentation:
            // - If chunk has upvalues, the first one should be _ENV (global environment)
            // - Other upvalues are initialized to nil
            let upvalue_count = chunk.upvalue_count;
            let mut upvalues = Vec::with_capacity(upvalue_count);

            for i in 0..upvalue_count {
                if i == 0 {
                    // First upvalue is _ENV
                    let env_upvalue_id = if let Some(env) = env {
                        l.create_upvalue_closed(env)?
                    } else {
                        let global = l.vm_mut().global;
                        l.create_upvalue_closed(global)?
                    };
                    upvalues.push(env_upvalue_id);
                } else {
                    // Other upvalues are initialized to nil
                    let nil_upvalue = l.create_upvalue_closed(LuaValue::nil())?;
                    upvalues.push(nil_upvalue);
                }
            }

            let func = l.create_function(
                Rc::new(chunk),
                crate::lua_value::UpvalueStore::from_vec(upvalues),
            )?;
            l.push_value(func)?;
            Ok(1)
        }
        Err(e) => {
            // Return nil and error message
            let err_msg = l.create_string(&e)?;
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            Ok(2)
        }
    }
}

/// loadfile([filename [, mode [, env]]]) - Load a file as a chunk
fn lua_loadfile(l: &mut LuaState) -> LuaResult<usize> {
    use crate::lua_value::chunk_serializer;

    let filename = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'loadfile' (value expected)".to_string()))?;

    let filename_str = if let Some(s) = filename.as_str() {
        s.to_string()
    } else {
        return Err(l.error("bad argument #1 to 'loadfile' (string expected)".to_string()));
    };

    // Optional mode ("b", "t", or "bt")
    let mode = l
        .get_arg(2)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "bt".to_string());

    // Optional environment table
    let env_arg = l.get_arg(3);

    // Load from specified file as bytes (to handle both text and binary)
    let file_bytes = match std::fs::read(&filename_str) {
        Ok(b) => b,
        Err(e) => {
            let err_msg = l.create_string(&format!("cannot open {}: {}", filename_str, e))?;
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            return Ok(2);
        }
    };

    // Determine content after skipping shebang/BOM for binary detection
    // For text files, the tokenizer handles shebang natively
    let mut skip_offset = 0;

    // Skip initial comment line (shebang) if present
    if file_bytes.first() == Some(&b'#') {
        if let Some(pos) = file_bytes.iter().position(|&b| b == b'\n') {
            skip_offset = pos + 1;
        } else {
            skip_offset = file_bytes.len();
        }
    }

    // Skip UTF-8 BOM if present (after potential shebang skip)
    if file_bytes[skip_offset..].starts_with(&[0xEF, 0xBB, 0xBF]) {
        skip_offset += 3;
    }

    // Check if it's a binary chunk (starts with 0x1B after shebang/BOM)
    let is_binary = file_bytes.get(skip_offset) == Some(&0x1B);

    // Validate mode
    if !mode.is_empty() && mode.chars().all(|c| c == 'b' || c == 't') {
        if is_binary && !mode.contains('b') {
            let err_msg = l.create_string("attempt to load a binary chunk (mode is 'text')")?;
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            return Ok(2);
        }
        if !is_binary && !mode.contains('t') {
            let err_msg = l.create_string("attempt to load a text chunk (mode is 'binary')")?;
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            return Ok(2);
        }
    }

    let chunkname = format!("@{}", filename_str);

    let chunk_result = if is_binary {
        // Deserialize binary bytecode (skip shebang/BOM)
        let vm = l.vm_mut();
        chunk_serializer::deserialize_chunk_with_strings_vm(&file_bytes[skip_offset..], vm)
            .map_err(|e| format!("binary load error: {}", e))
    } else {
        // For text, strip BOM but keep shebang (tokenizer handles it)
        let text_start = if file_bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        };
        // Source files must be valid UTF-8
        let code_str = match String::from_utf8(file_bytes[text_start..].to_vec()) {
            Ok(s) => s,
            Err(_) => {
                // loadfile returns nil + error message on failure
                let err_msg = l.create_string("source file is not valid UTF-8")?;
                l.push_value(LuaValue::nil())?;
                l.push_value(err_msg)?;
                return Ok(2);
            }
        };
        let vm = l.vm_mut();
        vm.compile_with_name(&code_str, &chunkname)
            .map_err(|e| vm.get_error_message(e))
    };

    match chunk_result {
        Ok(chunk) => {
            let upvalue_count = chunk.upvalue_count;
            let mut upvalues = Vec::with_capacity(upvalue_count);

            for i in 0..upvalue_count {
                if i == 0 {
                    let env_upvalue_id = if let Some(env) = env_arg {
                        l.create_upvalue_closed(env)?
                    } else {
                        let global = l.vm_mut().global;
                        l.create_upvalue_closed(global)?
                    };
                    upvalues.push(env_upvalue_id);
                } else {
                    let nil_upvalue = l.create_upvalue_closed(LuaValue::nil())?;
                    upvalues.push(nil_upvalue);
                }
            }

            let func = l.create_function(
                std::rc::Rc::new(chunk),
                crate::lua_value::UpvalueStore::from_vec(upvalues),
            )?;
            l.push_value(func)?;
            Ok(1)
        }
        Err(e) => {
            let err_msg = l.create_string(&e.to_string())?;
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            Ok(2)
        }
    }
}

/// dofile([filename]) - Execute a file
fn lua_dofile(l: &mut LuaState) -> LuaResult<usize> {
    let arg1 = l.get_arg(1);

    // Get filename (nil/none means stdin, which we don't support yet)
    let filename_str = if let Some(v) = arg1 {
        if v.is_nil() {
            return Err(l.error("dofile: reading from stdin not yet implemented".to_string()));
        }
        if let Some(s) = v.as_str() {
            s.to_string()
        } else {
            return Err(l.error("bad argument #1 to 'dofile' (string expected)".to_string()));
        }
    } else {
        return Err(l.error("dofile: reading from stdin not yet implemented".to_string()));
    };

    // Load from file as bytes
    let file_bytes = match std::fs::read(&filename_str) {
        Ok(b) => b,
        Err(e) => {
            return Err(l.error(format!("cannot open {}: {}", filename_str, e)));
        }
    };

    // Determine content after skipping shebang/BOM for binary detection
    let mut skip_offset = 0;
    if file_bytes.first() == Some(&b'#') {
        if let Some(pos) = file_bytes.iter().position(|&b| b == b'\n') {
            skip_offset = pos + 1;
        } else {
            skip_offset = file_bytes.len();
        }
    }
    if file_bytes[skip_offset..].starts_with(&[0xEF, 0xBB, 0xBF]) {
        skip_offset += 3;
    }

    let is_binary = file_bytes.get(skip_offset) == Some(&0x1B);

    // Compile/load the code
    let chunkname = format!("@{}", filename_str);
    let chunk = if is_binary {
        use crate::lua_value::chunk_serializer;
        let vm = l.vm_mut();
        chunk_serializer::deserialize_chunk_with_strings_vm(&file_bytes[skip_offset..], vm)
            .map_err(|e| l.error(format!("binary load error: {}", e)))?
    } else {
        let text_start = if file_bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        };
        let code_str = String::from_utf8(file_bytes[text_start..].to_vec())
            .map_err(|_| l.error("source file is not valid UTF-8".to_string()))?;
        l.vm_mut()
            .compile_with_name(&code_str, &chunkname)
            .map_err(|e| l.error(format!("error loading {}: {}", filename_str, e)))?
    };

    let global = l.vm_mut().global;
    // Create function with _ENV upvalue (global table)
    let env_upvalue = l.create_upvalue_closed(global)?;
    let upvalues = vec![env_upvalue];
    let func = l.create_function(
        std::rc::Rc::new(chunk),
        crate::lua_value::UpvalueStore::from_vec(upvalues),
    )?;

    // Use call_stack_based which supports yields (equivalent to lua_callk in C Lua).
    // Push the function onto the stack at the current top.
    let func_idx = l.get_top();
    l.push_value(func)?;
    let num_results = l.call_stack_based(func_idx, 0)?;

    Ok(num_results)
}

/// warn(msg1, ...) - Emit a warning (Lua 5.5 semantics)
///
/// Control messages: single argument starting with '@':
///   @on    - enable warnings (stderr mode)
///   @off   - disable warnings
///   @store - store warnings in _WARN global
///   @normal - restore normal stderr output
///   other  - ignored
///
/// Regular messages: concatenate all arguments; output according to state.
/// Warnings are OFF by default.
fn lua_warn(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();

    // At least one argument required, all must be strings
    if args.is_empty() {
        return Err(
            l.error("bad argument #1 to 'warn' (string expected, got no value)".to_string())
        );
    }
    let mut parts: Vec<String> = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        if let Some(s) = arg.as_str() {
            parts.push(s.to_string());
        } else {
            return Err(l.error(format!(
                "bad argument #{} to 'warn' (string expected, got {})",
                i + 1,
                arg.type_name()
            )));
        }
    }

    // Get current warn mode from registry ("off", "on", "store")
    let registry = l.vm_mut().registry;
    let mode_key = l.create_string("_WARN_MODE")?;
    let current_mode = l
        .raw_get(&registry, &mode_key)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "off".to_string());

    // Check for control message: single argument starting with '@'
    if parts.len() == 1 && parts[0].starts_with('@') {
        let control = &parts[0][1..];
        match control {
            "on" => {
                let mode_val = l.create_string("on")?;
                l.raw_set(&registry, mode_key, mode_val);
            }
            "off" => {
                let mode_val = l.create_string("off")?;
                l.raw_set(&registry, mode_key, mode_val);
            }
            "store" => {
                let mode_val = l.create_string("store")?;
                l.raw_set(&registry, mode_key, mode_val);
            }
            "normal" => {
                let mode_val = l.create_string("on")?;
                l.raw_set(&registry, mode_key, mode_val);
            }
            _ => {
                // Unknown control message, ignored
            }
        }
        return Ok(0);
    }

    // Regular message: concatenate all parts (no separator)
    let message: String = parts.concat();

    match current_mode.as_str() {
        "on" => {
            eprintln!("Lua warning: {}", message);
        }
        "store" => {
            // Store in _WARN global
            let warn_val = l.create_string(&message)?;
            l.vm_mut().set_global("_WARN", warn_val)?;
        }
        _ => {
            // "off" - do nothing
        }
    }

    Ok(0)
}
