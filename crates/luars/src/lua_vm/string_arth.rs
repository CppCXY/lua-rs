// ============================================================
// String arithmetic metamethods (Lua 5.5 string-to-number coercion)
// These are set as __add, __sub, etc. on the string metatable.
// Matches lstrlib.c: arith() + tonum()
// ============================================================

use crate::{
    LuaResult, LuaState, LuaValue,
    lua_vm::{TmKind, execute},
    stdlib::basic::parse_number::parse_lua_number,
};

/// Try to convert a LuaValue to a number (integer or float).
/// Returns the numeric value, or None if conversion fails.
/// Matches C Lua's `tonum()` in lstrlib.c — uses lua_stringtonumber
/// which handles decimals, hex integers, hex floats, signs, whitespace.
pub fn string_arith_tonum(v: &LuaValue) -> Option<LuaValue> {
    if v.is_integer() || v.is_float() {
        return Some(*v);
    }
    if v.is_string() {
        let result = parse_lua_number(v.as_str().unwrap_or(""));
        if !result.is_nil() {
            return Some(result);
        }
    }
    None
}

/// Perform a binary arithmetic operation, converting strings to numbers.
/// Matches C Lua's arith() + trymt() in lstrlib.c:
/// - If both operands convert to numbers, do the arithmetic
/// - Otherwise, if the second operand is also a string, error
/// - Otherwise, try the second operand's metamethod for this operation
/// - If no metamethod found, error
pub fn string_arith_bin(
    l: &mut LuaState,
    op_name: &str,
    tm_kind: TmKind,
    op: fn(LuaValue, LuaValue) -> Option<LuaValue>,
) -> LuaResult<usize> {
    let v1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("attempt to perform arithmetic on a nil value".to_string()))?;
    let v2 = l
        .get_arg(2)
        .ok_or_else(|| l.error("attempt to perform arithmetic on a nil value".to_string()))?;

    let n1 = string_arith_tonum(&v1);
    let n2 = string_arith_tonum(&v2);

    if let (Some(a), Some(b)) = (n1, n2)
        && let Some(result) = op(a, b)
    {
        l.push_value(result)?;
        return Ok(1);
    }

    // Conversion failed — implement trymt() from C Lua:
    // If the second operand is a string, both are strings and both failed → error.
    // Otherwise, try the second operand's metamethod.
    if !v2.is_string()
        && let Some(mt) = execute::get_metatable(l, &v2)
    {
        let tm_key = l.vm_mut().const_strings.get_tm_value(tm_kind);
        if let Some(mm) = mt.as_table().and_then(|t| t.raw_get(&tm_key)) {
            // Call the other operand's metamethod with original args
            let results = l.call_function(mm, vec![v1, v2])?;
            if let Some(r) = results.into_iter().next() {
                l.push_value(r)?;
            } else {
                l.push_value(LuaValue::nil())?;
            }
            return Ok(1);
        }
    }

    let t1 = v1.type_name();
    let t2 = v2.type_name();
    Err(l.error(format!("attempt to {} a '{}' with a '{}'", op_name, t1, t2)))
}

pub fn arith_add(a: LuaValue, b: LuaValue) -> Option<LuaValue> {
    match (a.as_integer(), b.as_integer()) {
        (Some(x), Some(y)) => Some(LuaValue::integer(x.wrapping_add(y))),
        _ => {
            let fa = a.as_number().or_else(|| a.as_integer().map(|i| i as f64))?;
            let fb = b.as_number().or_else(|| b.as_integer().map(|i| i as f64))?;
            Some(LuaValue::float(fa + fb))
        }
    }
}

pub fn arith_sub(a: LuaValue, b: LuaValue) -> Option<LuaValue> {
    match (a.as_integer(), b.as_integer()) {
        (Some(x), Some(y)) => Some(LuaValue::integer(x.wrapping_sub(y))),
        _ => {
            let fa = a.as_number().or_else(|| a.as_integer().map(|i| i as f64))?;
            let fb = b.as_number().or_else(|| b.as_integer().map(|i| i as f64))?;
            Some(LuaValue::float(fa - fb))
        }
    }
}

pub fn arith_mul(a: LuaValue, b: LuaValue) -> Option<LuaValue> {
    match (a.as_integer(), b.as_integer()) {
        (Some(x), Some(y)) => Some(LuaValue::integer(x.wrapping_mul(y))),
        _ => {
            let fa = a.as_number().or_else(|| a.as_integer().map(|i| i as f64))?;
            let fb = b.as_number().or_else(|| b.as_integer().map(|i| i as f64))?;
            Some(LuaValue::float(fa * fb))
        }
    }
}

pub fn arith_mod(a: LuaValue, b: LuaValue) -> Option<LuaValue> {
    match (a.as_integer(), b.as_integer()) {
        (Some(x), Some(y)) => {
            if y == 0 {
                return Some(LuaValue::float(f64::NAN));
            }
            // Lua mod: a - floor(a/b)*b
            let r = x.wrapping_rem(y);
            if r != 0 && (r ^ y) < 0 {
                Some(LuaValue::integer(r.wrapping_add(y)))
            } else {
                Some(LuaValue::integer(r))
            }
        }
        _ => {
            let fa = a.as_number().or_else(|| a.as_integer().map(|i| i as f64))?;
            let fb = b.as_number().or_else(|| b.as_integer().map(|i| i as f64))?;
            let r = fa % fb;
            // Lua float mod semantics
            if r != 0.0 && r.is_sign_negative() != fb.is_sign_negative() {
                Some(LuaValue::float(r + fb))
            } else {
                Some(LuaValue::float(r))
            }
        }
    }
}

pub fn arith_pow(a: LuaValue, b: LuaValue) -> Option<LuaValue> {
    let fa = a.as_number().or_else(|| a.as_integer().map(|i| i as f64))?;
    let fb = b.as_number().or_else(|| b.as_integer().map(|i| i as f64))?;
    Some(LuaValue::float(fa.powf(fb)))
}

pub fn arith_div(a: LuaValue, b: LuaValue) -> Option<LuaValue> {
    // Division always returns float in Lua
    let fa = a.as_number().or_else(|| a.as_integer().map(|i| i as f64))?;
    let fb = b.as_number().or_else(|| b.as_integer().map(|i| i as f64))?;
    Some(LuaValue::float(fa / fb))
}

pub fn arith_idiv(a: LuaValue, b: LuaValue) -> Option<LuaValue> {
    match (a.as_integer(), b.as_integer()) {
        (Some(x), Some(y)) => {
            if y == 0 {
                return Some(LuaValue::float(f64::NAN));
            }
            // Lua floor division
            let d = x.wrapping_div(y);
            if (x ^ y) < 0 && d * y != x {
                Some(LuaValue::integer(d - 1))
            } else {
                Some(LuaValue::integer(d))
            }
        }
        _ => {
            let fa = a.as_number().or_else(|| a.as_integer().map(|i| i as f64))?;
            let fb = b.as_number().or_else(|| b.as_integer().map(|i| i as f64))?;
            Some(LuaValue::float((fa / fb).floor()))
        }
    }
}

pub fn string_arith_add(l: &mut LuaState) -> LuaResult<usize> {
    string_arith_bin(l, "add", TmKind::Add, arith_add)
}

pub fn string_arith_sub(l: &mut LuaState) -> LuaResult<usize> {
    string_arith_bin(l, "sub", TmKind::Sub, arith_sub)
}

pub fn string_arith_mul(l: &mut LuaState) -> LuaResult<usize> {
    string_arith_bin(l, "mul", TmKind::Mul, arith_mul)
}

pub fn string_arith_mod(l: &mut LuaState) -> LuaResult<usize> {
    string_arith_bin(l, "mod", TmKind::Mod, arith_mod)
}

pub fn string_arith_pow(l: &mut LuaState) -> LuaResult<usize> {
    string_arith_bin(l, "pow", TmKind::Pow, arith_pow)
}

pub fn string_arith_div(l: &mut LuaState) -> LuaResult<usize> {
    string_arith_bin(l, "div", TmKind::Div, arith_div)
}

pub fn string_arith_idiv(l: &mut LuaState) -> LuaResult<usize> {
    string_arith_bin(l, "idiv", TmKind::IDiv, arith_idiv)
}

pub fn string_arith_unm(l: &mut LuaState) -> LuaResult<usize> {
    let v1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("attempt to perform arithmetic on a nil value".to_string()))?;

    if let Some(n) = string_arith_tonum(&v1) {
        if let Some(i) = n.as_integer() {
            l.push_value(LuaValue::integer(i.wrapping_neg()))?;
        } else if let Some(f) = n.as_number() {
            l.push_value(LuaValue::float(-f))?;
        }
        return Ok(1);
    }

    Err(l.error(format!(
        "attempt to perform arithmetic on a '{}' value",
        v1.type_name()
    )))
}
