// Math library
// Implements: abs, acos, asin, atan, ceil, cos, deg, exp, floor, fmod,
// log, max, min, modf, rad, random, randomseed, sin, sqrt, tan, tointeger,
// type, ult, pi, huge, maxinteger, mininteger

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, LuaValueKind};
use crate::lua_vm::LuaError;
use crate::lua_vm::LuaResult;
use crate::lua_vm::LuaState;

/// Check that argument at position `n` is a number, with proper error message.
fn checknumber(l: &mut LuaState, n: usize, fname: &str) -> Result<f64, LuaError> {
    let Some(v) = l.get_arg(n) else {
        return Err(l.error(format!(
            "bad argument #{} to '{}' (number expected)",
            n, fname
        )));
    };
    if let Some(f) = v.as_number() {
        return Ok(f);
    }
    if let Some(s) = v.as_str()
        && let Ok(f) = s.trim().parse::<f64>()
    {
        return Ok(f);
    }
    let t = crate::stdlib::debug::objtypename(l, &v);
    Err(l.error(format!(
        "bad argument #{} to '{}' (number expected, got {})",
        n, fname, t
    )))
}

pub fn create_math_lib() -> LibraryModule {
    let mut module = crate::lib_module!("math", {
        "abs" => math_abs,
        "acos" => math_acos,
        "asin" => math_asin,
        "atan" => math_atan,
        "ceil" => math_ceil,
        "cos" => math_cos,
        "deg" => math_deg,
        "exp" => math_exp,
        "floor" => math_floor,
        "fmod" => math_fmod,
        "frexp" => math_frexp,
        "ldexp" => math_ldexp,
        "log" => math_log,
        "max" => math_max,
        "min" => math_min,
        "modf" => math_modf,
        "rad" => math_rad,
        "random" => math_random,
        "randomseed" => math_randomseed,
        "sin" => math_sin,
        "sqrt" => math_sqrt,
        "tan" => math_tan,
        "tointeger" => math_tointeger,
        "type" => math_type,
        "ult" => math_ult,
    });

    // Add constants using with_value
    module = module.with_value("pi", |_vm| Ok(LuaValue::float(std::f64::consts::PI)));
    module = module.with_value("huge", |_vm| Ok(LuaValue::float(f64::INFINITY)));
    module = module.with_value("maxinteger", |_vm| Ok(LuaValue::integer(i64::MAX)));
    module = module.with_value("mininteger", |_vm| Ok(LuaValue::integer(i64::MIN)));

    module
}

fn math_abs(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'abs' (number expected)".to_string()))?;

    // Fast path: preserve integer type
    if let Some(i) = value.as_integer() {
        l.push_value(LuaValue::integer(i.wrapping_abs()))?;
        return Ok(1);
    }

    if let Some(f) = value.as_float() {
        l.push_value(LuaValue::float(f.abs()))?;
        return Ok(1);
    }

    Err(l.error("bad argument #1 to 'abs' (number expected)".to_string()))
}

fn math_acos(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'acos' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'acos' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.acos()))?;
    Ok(1)
}

fn math_asin(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'asin' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'asin' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.asin()))?;
    Ok(1)
}

fn math_atan(l: &mut LuaState) -> LuaResult<usize> {
    let y = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'atan' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'atan' (number expected)".to_string()))?;
    let x = l.get_arg(2).and_then(|v| v.as_number()).unwrap_or(1.0);
    l.push_value(LuaValue::float(y.atan2(x)))?;
    Ok(1)
}

fn math_ceil(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'ceil' (number expected)".to_string()))?;

    // Fast path: integers are already ceil'd
    if let Some(i) = value.as_integer() {
        l.push_value(LuaValue::integer(i))?;
        return Ok(1);
    }

    if let Some(f) = value.as_float() {
        let ceiled = f.ceil();
        // Return integer if result fits, otherwise keep as float
        if ceiled >= (i64::MIN as f64) && ceiled < -(i64::MIN as f64) {
            l.push_value(LuaValue::integer(ceiled as i64))?;
        } else {
            l.push_value(LuaValue::float(ceiled))?;
        }
        return Ok(1);
    }

    Err(l.error("bad argument #1 to 'ceil' (number expected)".to_string()))
}

fn math_cos(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'cos' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'cos' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.cos()))?;
    Ok(1)
}

fn math_deg(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'deg' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'deg' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.to_degrees()))?;
    Ok(1)
}

fn math_exp(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'exp' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'exp' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.exp()))?;
    Ok(1)
}

fn math_floor(l: &mut LuaState) -> LuaResult<usize> {
    let value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'floor' (number expected)".to_string()))?;

    // Fast path: integers are already floor'd
    if let Some(i) = value.as_integer() {
        l.push_value(LuaValue::integer(i))?;
        return Ok(1);
    }

    if let Some(f) = value.as_float() {
        let floored = f.floor();
        // Return integer if result fits, otherwise keep as float
        if floored >= (i64::MIN as f64) && floored < -(i64::MIN as f64) {
            l.push_value(LuaValue::integer(floored as i64))?;
        } else {
            l.push_value(LuaValue::float(floored))?;
        }
        return Ok(1);
    }

    Err(l.error("bad argument #1 to 'floor' (number expected)".to_string()))
}

fn math_fmod(l: &mut LuaState) -> LuaResult<usize> {
    let arg1 = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'fmod' (number expected)".to_string()))?;
    let arg2 = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'fmod' (number expected)".to_string()))?;

    // If both arguments are raw integers (not float-that-happens-to-be-integral), do integer fmod
    if arg1.is_integer() && arg2.is_integer() {
        let a = arg1.as_integer_strict().unwrap();
        let b = arg2.as_integer_strict().unwrap();
        if b == 0 {
            return Err(l.error("bad argument #2 to 'fmod' (zero)".to_string()));
        }
        // C Lua uses luaV_mod for integers: a % b with sign adjustment
        // But math.fmod for integers just uses C's fmod semantics (truncated division remainder)
        // In C Lua, math_fmod calls luaV_modf which for integers does:
        //   if b == -1: result = 0 (avoid overflow of minint % -1)
        //   else: result = a % b (C remainder, truncated toward zero)
        let result = if b == -1 { 0 } else { a % b };
        l.push_value(LuaValue::integer(result))?;
        return Ok(1);
    }

    let x = arg1
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'fmod' (number expected)".to_string()))?;
    let y = arg2
        .as_number()
        .ok_or_else(|| l.error("bad argument #2 to 'fmod' (number expected)".to_string()))?;
    if y == 0.0 {
        return Err(l.error("bad argument #2 to 'fmod' (zero)".to_string()));
    }
    l.push_value(LuaValue::float(x % y))?;
    Ok(1)
}

fn math_log(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'log' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'log' (number expected)".to_string()))?;
    let base = l.get_arg(2).and_then(|v| v.as_number());

    let result = if let Some(b) = base { x.log(b) } else { x.ln() };

    l.push_value(LuaValue::float(result))?;
    Ok(1)
}

/// Compare two numeric LuaValues properly without losing precision.
/// Returns true if a < b.
fn lua_num_lt(a: &LuaValue, b: &LuaValue) -> bool {
    match (a.as_integer_strict(), b.as_integer_strict()) {
        (Some(ai), Some(bi)) => ai < bi,
        (Some(ai), None) => {
            let bf = b.as_number().unwrap_or(f64::NAN);
            // int < float: compare carefully
            lua_int_lt_float(ai, bf)
        }
        (None, Some(bi)) => {
            let af = a.as_number().unwrap_or(f64::NAN);
            // float < int
            lua_float_lt_int(af, bi)
        }
        (None, None) => {
            let af = a.as_number().unwrap_or(f64::NAN);
            let bf = b.as_number().unwrap_or(f64::NAN);
            af < bf
        }
    }
}

/// int < float comparison (matching C Lua's LTintfloat)
fn lua_int_lt_float(a: i64, b: f64) -> bool {
    if b.is_nan() {
        return false;
    }
    // If b is within i64 range, cast to i64 and compare; else compare as float
    if b >= -(i64::MIN as f64) {
        true // b >= 2^63, any i64 < b
    } else if b < (i64::MIN as f64) {
        false // b < -2^63, no i64 < b
    } else {
        let bi = b as i64;
        if (bi as f64) == b {
            a < bi // b is exactly representable
        } else {
            (a as f64) < b // compare as floats
        }
    }
}

/// float < int comparison (matching C Lua's LTfloatint)
fn lua_float_lt_int(a: f64, b: i64) -> bool {
    if a.is_nan() {
        return false;
    }
    if a < (i64::MIN as f64) {
        true // a < -2^63, a < any i64
    } else if a >= -(i64::MIN as f64) {
        false // a >= 2^63, no i64 > a
    } else {
        let ai = a as i64;
        if (ai as f64) == a {
            ai < b // a is exactly representable
        } else {
            a < (b as f64)
        }
    }
}

fn math_max(l: &mut LuaState) -> LuaResult<usize> {
    let argc = l.arg_count();

    if argc == 0 {
        return Err(l.error("bad argument to 'max' (value expected)".to_string()));
    }

    // Get first argument
    let first = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'max' (number expected)".to_string()))?;
    // Validate it's a number
    let _ = first
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'max' (number expected)".to_string()))?;
    let mut max_arg = first;

    // Compare with rest
    for i in 2..=argc {
        let arg = l
            .get_arg(i)
            .ok_or_else(|| l.error(format!("bad argument #{} to 'max' (number expected)", i)))?;
        let _ = arg
            .as_number()
            .ok_or_else(|| l.error(format!("bad argument #{} to 'max' (number expected)", i)))?;
        if lua_num_lt(&max_arg, &arg) {
            max_arg = arg;
        }
    }

    l.push_value(max_arg)?;
    Ok(1)
}

fn math_min(l: &mut LuaState) -> LuaResult<usize> {
    let argc = l.arg_count();

    if argc == 0 {
        return Err(l.error("bad argument to 'min' (value expected)".to_string()));
    }

    // Get first argument
    let first = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'min' (number expected)".to_string()))?;
    let _ = first
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'min' (number expected)".to_string()))?;
    let mut min_arg = first;

    // Compare with rest
    for i in 2..=argc {
        let arg = l
            .get_arg(i)
            .ok_or_else(|| l.error(format!("bad argument #{} to 'min' (number expected)", i)))?;
        let _ = arg
            .as_number()
            .ok_or_else(|| l.error(format!("bad argument #{} to 'min' (number expected)", i)))?;
        if lua_num_lt(&arg, &min_arg) {
            min_arg = arg;
        }
    }

    l.push_value(min_arg)?;
    Ok(1)
}

fn math_modf(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'modf' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'modf' (number expected)".to_string()))?;
    let int_part = x.trunc();
    let frac_part = if x.is_infinite() { 0.0 } else { x - int_part };

    // Return integer part as integer if it fits
    if !x.is_nan()
        && !x.is_infinite()
        && int_part >= i64::MIN as f64
        && int_part < -(i64::MIN as f64)
    {
        l.push_value(LuaValue::integer(int_part as i64))?;
    } else {
        l.push_value(LuaValue::float(int_part))?;
    }
    l.push_value(LuaValue::float(frac_part))?;
    Ok(2)
}

fn math_rad(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rad' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'rad' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.to_radians()))?;
    Ok(1)
}

fn math_random(l: &mut LuaState) -> LuaResult<usize> {
    let argc = l.arg_count();

    if argc > 2 {
        return Err(l.error("wrong number of arguments to 'random'".to_string()));
    }

    match argc {
        0 => {
            // No arguments: return float in [0, 1)
            let r = l.vm_mut().rng.next_float();
            l.push_value(LuaValue::float(r))?;
            Ok(1)
        }
        1 => {
            let rv = l.vm_mut().rng.next_rand();
            let up = l
                .get_arg(1)
                .ok_or_else(|| {
                    l.error("bad argument #1 to 'random' (number expected)".to_string())
                })?
                .as_integer()
                .ok_or_else(|| {
                    l.error("bad argument #1 to 'random' (number expected, got float)".to_string())
                })?;
            if up == 0 {
                // random(0): return raw random integer
                l.push_value(LuaValue::integer(rv as i64))?;
                return Ok(1);
            }
            // random(n): return random integer in [1, n]
            if up < 1 {
                return Err(l.error("bad argument #1 to 'random' (interval is empty)".to_string()));
            }
            let result = project(rv, 1, up as u64)?;
            l.push_value(LuaValue::integer(result))?;
            Ok(1)
        }
        _ => {
            let rv = l.vm_mut().rng.next_rand();
            let low = l
                .get_arg(1)
                .ok_or_else(|| {
                    l.error("bad argument #1 to 'random' (number expected)".to_string())
                })?
                .as_integer()
                .ok_or_else(|| {
                    l.error("bad argument #1 to 'random' (number expected, got float)".to_string())
                })?;
            let up = l
                .get_arg(2)
                .ok_or_else(|| {
                    l.error("bad argument #2 to 'random' (number expected)".to_string())
                })?
                .as_integer()
                .ok_or_else(|| {
                    l.error("bad argument #2 to 'random' (number expected, got float)".to_string())
                })?;
            if low > up {
                return Err(l.error("bad argument #2 to 'random' (interval is empty)".to_string()));
            }
            let result = project(rv, low as u64, up as u64)?;
            l.push_value(LuaValue::integer(result))?;
            Ok(1)
        }
    }
}

/// Project a random u64 into [low, up] range (matching C Lua's project)
fn project(rv: u64, low: u64, up: u64) -> LuaResult<i64> {
    // range = up - low + 1 (could overflow for full range)
    let range = up.wrapping_sub(low).wrapping_add(1);
    if range == 0 {
        // Full u64 range
        Ok(low.wrapping_add(rv) as i64)
    } else {
        // Compute rv % range using rejection sampling to avoid bias
        // Simple version: just use modulo (C Lua does this too for the common case)
        Ok(low.wrapping_add(rv % range) as i64)
    }
}

fn math_randomseed(l: &mut LuaState) -> LuaResult<usize> {
    use crate::lua_vm::LuaRng;
    let argc = l.arg_count();

    let (n1, n2) = if argc == 0 || (argc >= 1 && l.get_arg(1).is_none_or(|v| v.is_nil())) {
        // No argument or nil: use time-based seed
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        (time as i64, 0i64)
    } else {
        let seed1 = l.get_arg(1).and_then(|v| v.as_integer()).ok_or_else(|| {
            l.error("bad argument #1 to 'randomseed' (number expected)".to_string())
        })?;
        let seed2 = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(0);
        (seed1, seed2)
    };

    l.vm_mut().rng = LuaRng::from_seed(n1, n2);

    // Return two seed values
    l.push_value(LuaValue::integer(n1))?;
    l.push_value(LuaValue::integer(n2))?;
    Ok(2)
}

fn math_sin(l: &mut LuaState) -> LuaResult<usize> {
    let x = checknumber(l, 1, "sin")?;
    l.push_value(LuaValue::float(x.sin()))?;
    Ok(1)
}

fn math_sqrt(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'sqrt' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'sqrt' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.sqrt()))?;
    Ok(1)
}

fn math_tan(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'tan' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'tan' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.tan()))?;
    Ok(1)
}

fn math_tointeger(l: &mut LuaState) -> LuaResult<usize> {
    let val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'tointeger' (value expected)".to_string()))?;

    let result = if let Some(i) = val.as_integer() {
        LuaValue::integer(i)
    } else if let Some(f) = val.as_number() {
        float_to_integer(f)
    } else if let Some(s) = val.as_str() {
        // Try to parse string as integer
        let s_str = s.trim();
        if let Ok(i) = s_str.parse::<i64>() {
            LuaValue::integer(i)
        } else if let Ok(f) = s_str.parse::<f64>() {
            // String is a float, check if it's a whole number
            float_to_integer(f)
        } else {
            LuaValue::nil()
        }
    } else {
        LuaValue::nil()
    };

    l.push_value(result)?;
    Ok(1)
}

/// Convert a float to integer if it's an exact integer within i64 range
/// Returns nil if the float is not an exact integer or out of range
fn float_to_integer(f: f64) -> LuaValue {
    // Check for non-finite values
    if !f.is_finite() {
        return LuaValue::nil();
    }
    // Check if it's a whole number
    if f.fract() != 0.0 {
        return LuaValue::nil();
    }
    // Check if it's within i64 range BEFORE conversion
    // i64::MIN = -9223372036854775808 (can be exactly represented in f64)
    // i64::MAX = 9223372036854775807 (cannot be exactly represented in f64)
    // The closest f64 values are:
    // - i64::MIN as f64 = -9223372036854775808.0 (exact)
    // - i64::MAX as f64 = 9223372036854776000.0 (rounded up!)
    //
    // So we check: f >= i64::MIN as f64 AND f < (i64::MAX as f64 + 1.0)
    // But since i64::MAX can't be exactly represented, we use a different check:
    // f must be in the range where f as i64 doesn't overflow
    const MIN_F: f64 = i64::MIN as f64; // -9223372036854775808.0
    // i64::MAX + 1 = 9223372036854775808 which is exactly representable as f64
    const MAX_PLUS_ONE: f64 = 9223372036854775808.0; // 2^63

    if (MIN_F..MAX_PLUS_ONE).contains(&f) {
        let i = f as i64;
        // Verify the conversion is exact
        if i as f64 == f {
            LuaValue::integer(i)
        } else {
            LuaValue::nil()
        }
    } else {
        LuaValue::nil()
    }
}

fn math_type(l: &mut LuaState) -> LuaResult<usize> {
    let val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'type' (value expected)".to_string()))?;

    let type_str = match val.kind() {
        LuaValueKind::Integer => "integer",
        LuaValueKind::Float => "float",
        _ => {
            l.push_value(LuaValue::nil())?;
            return Ok(1);
        }
    };

    let result = l.create_string(type_str)?;
    l.push_value(result)?;
    Ok(1)
}

fn math_ult(l: &mut LuaState) -> LuaResult<usize> {
    let m_value = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'ult' (integer expected)".to_string()))?;
    let Some(m) = m_value.as_integer() else {
        return Err(l.error("bad argument #1 to 'ult' (integer expected)".to_string()));
    };
    let n_value = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'ult' (integer expected)".to_string()))?;
    let Some(n) = n_value.as_integer() else {
        return Err(l.error("bad argument #2 to 'ult' (integer expected)".to_string()));
    };
    // Unsigned less than
    let result = (m as u64) < (n as u64);
    l.push_value(LuaValue::boolean(result))?;
    Ok(1)
}

/// math.frexp(x) -> m, e such that x = m * 2^e, 0.5 <= |m| < 1
fn math_frexp(l: &mut LuaState) -> LuaResult<usize> {
    let x = l
        .get_arg(1)
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .ok_or_else(|| l.error("bad argument #1 to 'frexp' (number expected)".to_string()))?;

    if x == 0.0 {
        l.push_value(LuaValue::float(0.0))?;
        l.push_value(LuaValue::integer(0))?;
        return Ok(2);
    }
    if x.is_infinite() || x.is_nan() {
        l.push_value(LuaValue::float(x))?;
        l.push_value(LuaValue::integer(0))?;
        return Ok(2);
    }

    // frexp: extract mantissa and exponent
    let bits = x.to_bits();
    let sign = if (bits >> 63) != 0 { -1.0 } else { 1.0 };
    let abs_x = x.abs();
    let mut exp = ((bits >> 52) & 0x7FF) as i64 - 1022;
    let mut mantissa =
        sign * f64::from_bits((abs_x.to_bits() & 0x000FFFFFFFFFFFFF) | 0x3FE0000000000000);

    // Handle subnormals
    if ((bits >> 52) & 0x7FF) == 0 {
        // Subnormal: multiply by 2^53 to normalize, then adjust exponent
        let norm = abs_x * (2.0f64.powi(53));
        let norm_bits = norm.to_bits();
        exp = ((norm_bits >> 52) & 0x7FF) as i64 - 1022 - 53;
        mantissa = sign * f64::from_bits((norm_bits & 0x000FFFFFFFFFFFFF) | 0x3FE0000000000000);
    }

    l.push_value(LuaValue::float(mantissa))?;
    l.push_value(LuaValue::integer(exp))?;
    Ok(2)
}

/// math.ldexp(m, e) -> m * 2^e
fn math_ldexp(l: &mut LuaState) -> LuaResult<usize> {
    let m = l
        .get_arg(1)
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .ok_or_else(|| l.error("bad argument #1 to 'ldexp' (number expected)".to_string()))?;
    let e = l
        .get_arg(2)
        .and_then(|v| v.as_integer())
        .ok_or_else(|| l.error("bad argument #2 to 'ldexp' (number expected)".to_string()))?;

    // Use the ldexp helper from parse_number to handle large exponents
    let mut result = m;
    let mut exp = e;
    while exp > 1023 {
        result *= 2.0f64.powi(1023);
        exp -= 1023;
        if result.is_infinite() {
            break;
        }
    }
    while exp < -1074 {
        result *= 2.0f64.powi(-1074);
        exp += 1074;
        if result == 0.0 {
            break;
        }
    }
    if !result.is_infinite() && result != 0.0 {
        result *= 2.0f64.powi(exp as i32);
    }

    l.push_value(LuaValue::float(result))?;
    Ok(1)
}
