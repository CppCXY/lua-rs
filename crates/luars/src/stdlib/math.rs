// Math library
// Implements: abs, acos, asin, atan, ceil, cos, deg, exp, floor, fmod,
// log, max, min, modf, rad, random, randomseed, sin, sqrt, tan, tointeger,
// type, ult, pi, huge, maxinteger, mininteger

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, LuaValueKind};
use crate::lua_vm::LuaState;
use crate::lua_vm::LuaResult;

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
    module = module.with_value("pi", |_vm| LuaValue::float(std::f64::consts::PI));
    module = module.with_value("huge", |_vm| LuaValue::float(f64::INFINITY));
    module = module.with_value("maxinteger", |_vm| LuaValue::integer(i64::MAX));
    module = module.with_value("mininteger", |_vm| LuaValue::integer(i64::MIN));

    module
}

fn math_abs(l: &mut LuaState) -> LuaResult<usize> {
    let value = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'abs' (number expected)".to_string()))?;

    // Fast path: preserve integer type
    if let Some(i) = value.as_integer() {
        l.push_value(LuaValue::integer(i.abs()))?;
        return Ok(1);
    }

    if let Some(f) = value.as_float() {
        l.push_value(LuaValue::float(f.abs()))?;
        return Ok(1);
    }

    Err(l.error("bad argument #1 to 'abs' (number expected)".to_string()))
}

fn math_acos(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'acos' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'acos' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.acos()))?;
    Ok(1)
}

fn math_asin(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'asin' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'asin' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.asin()))?;
    Ok(1)
}

fn math_atan(l: &mut LuaState) -> LuaResult<usize> {
    let y = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'atan' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'atan' (number expected)".to_string()))?;
    let x = l.get_arg(2).and_then(|v| v.as_number()).unwrap_or(1.0);
    l.push_value(LuaValue::float(y.atan2(x)))?;
    Ok(1)
}

fn math_ceil(l: &mut LuaState) -> LuaResult<usize> {
    let value = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'ceil' (number expected)".to_string()))?;

    // Fast path: integers are already ceil'd
    if let Some(i) = value.as_integer() {
        l.push_value(LuaValue::integer(i))?;
        return Ok(1);
    }

    if let Some(f) = value.as_float() {
        l.push_value(LuaValue::integer(f.ceil() as i64))?;
        return Ok(1);
    }

    Err(l.error("bad argument #1 to 'ceil' (number expected)".to_string()))
}

fn math_cos(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'cos' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'cos' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.cos()))?;
    Ok(1)
}

fn math_deg(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'deg' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'deg' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.to_degrees()))?;
    Ok(1)
}

fn math_exp(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'exp' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'exp' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.exp()))?;
    Ok(1)
}

fn math_floor(l: &mut LuaState) -> LuaResult<usize> {
    let value = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'floor' (number expected)".to_string()))?;

    // Fast path: integers are already floor'd
    if let Some(i) = value.as_integer() {
        l.push_value(LuaValue::integer(i))?;
        return Ok(1);
    }

    if let Some(f) = value.as_float() {
        l.push_value(LuaValue::integer(f.floor() as i64))?;
        return Ok(1);
    }

    Err(l.error("bad argument #1 to 'floor' (number expected)".to_string()))
}

fn math_fmod(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'fmod' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'fmod' (number expected)".to_string()))?;
    let y = l.get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'fmod' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #2 to 'fmod' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x % y))?;
    Ok(1)
}

fn math_log(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'log' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'log' (number expected)".to_string()))?;
    let base = l.get_arg(2).and_then(|v| v.as_number());

    let result = if let Some(b) = base { x.log(b) } else { x.ln() };

    l.push_value(LuaValue::float(result))?;
    Ok(1)
}

fn math_max(l: &mut LuaState) -> LuaResult<usize> {
    let argc = l.arg_count();

    if argc == 0 {
        return Err(l.error("bad argument to 'max' (value expected)".to_string()));
    }

    // Get first argument
    let first = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'max' (number expected)".to_string()))?;
    let mut max_val = first.as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'max' (number expected)".to_string()))?;
    let mut max_arg = first;

    // Compare with rest
    for i in 2..=argc {
        let arg = l.get_arg(i)
            .ok_or_else(|| l.error(format!("bad argument #{} to 'max' (number expected)", i)))?;
        let val = arg.as_number()
            .ok_or_else(|| l.error(format!("bad argument #{} to 'max' (number expected)", i)))?;
        if val > max_val {
            max_val = val;
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
    let first = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'min' (number expected)".to_string()))?;
    let mut min_val = first.as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'min' (number expected)".to_string()))?;
    let mut min_arg = first;

    // Compare with rest
    for i in 2..=argc {
        let arg = l.get_arg(i)
            .ok_or_else(|| l.error(format!("bad argument #{} to 'min' (number expected)", i)))?;
        let val = arg.as_number()
            .ok_or_else(|| l.error(format!("bad argument #{} to 'min' (number expected)", i)))?;
        if val < min_val {
            min_val = val;
            min_arg = arg;
        }
    }

    l.push_value(min_arg)?;
    Ok(1)
}

fn math_modf(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'modf' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'modf' (number expected)".to_string()))?;
    let int_part = x.trunc();
    let frac_part = x - int_part;

    l.push_value(LuaValue::float(int_part))?;
    l.push_value(LuaValue::float(frac_part))?;
    Ok(2)
}

fn math_rad(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'rad' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'rad' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.to_radians()))?;
    Ok(1)
}

/// Thread-local random state using xorshift64 algorithm
/// Much faster than creating new RandomState each call
use std::cell::Cell;
thread_local! {
    static RANDOM_STATE: Cell<u64> = Cell::new(0x853c49e6748fea9b_u64);
}

/// Fast xorshift64 random number generator
#[inline(always)]
fn xorshift64() -> u64 {
    RANDOM_STATE.with(|state| {
        let mut x = state.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        state.set(x);
        x
    })
}

fn math_random(l: &mut LuaState) -> LuaResult<usize> {
    let argc = l.arg_count();

    // Generate random u64 and convert to [0, 1) float
    let rand_u64 = xorshift64();
    let random = (rand_u64 >> 11) as f64 / (1u64 << 53) as f64;

    match argc {
        0 => {
            l.push_value(LuaValue::float(random))?;
            Ok(1)
        }
        1 => {
            let m = l.get_arg(1)
                .ok_or_else(|| l.error("bad argument #1 to 'random' (number expected)".to_string()))?
                .as_number()
                .ok_or_else(|| l.error("bad argument #1 to 'random' (number expected)".to_string()))? as i64;
            if m < 1 {
                return Err(l.error("bad argument #1 to 'random' (interval is empty)".to_string()));
            }
            let result = (random * m as f64).floor() as i64 + 1;
            l.push_value(LuaValue::integer(result))?;
            Ok(1)
        }
        _ => {
            let m = l.get_arg(1)
                .ok_or_else(|| l.error("bad argument #1 to 'random' (number expected)".to_string()))?
                .as_number()
                .ok_or_else(|| l.error("bad argument #1 to 'random' (number expected)".to_string()))? as i64;
            let n = l.get_arg(2)
                .ok_or_else(|| l.error("bad argument #2 to 'random' (number expected)".to_string()))?
                .as_number()
                .ok_or_else(|| l.error("bad argument #2 to 'random' (number expected)".to_string()))? as i64;
            if m > n {
                return Err(l.error("bad argument #1 to 'random' (interval is empty)".to_string()));
            }
            let range = (n - m + 1) as f64;
            let result = m + (random * range).floor() as i64;
            l.push_value(LuaValue::integer(result))?;
            Ok(1)
        }
    }
}

fn math_randomseed(l: &mut LuaState) -> LuaResult<usize> {
    // Lua 5.4: math.randomseed() with no args uses time-based seed
    let seed = if let Some(arg) = l.get_arg(1) {
        if arg.is_nil() {
            // Use time-based seed
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x853c49e6748fea9b_u64)
        } else if let Some(n) = arg.as_number() {
            n as u64
        } else {
            return Err(
                l.error("bad argument #1 to 'randomseed' (number expected)".to_string())
            );
        }
    } else {
        // No argument - use time-based seed
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x853c49e6748fea9b_u64)
    };

    // Seed the random state
    RANDOM_STATE.with(|state| {
        // Ensure seed is non-zero for xorshift
        let s = if seed == 0 {
            0x853c49e6748fea9b_u64
        } else {
            seed
        };
        state.set(s);
    });

    // Lua 5.4 returns two values from randomseed
    l.push_value(LuaValue::integer((seed >> 32) as i64))?;
    l.push_value(LuaValue::integer((seed & 0xFFFFFFFF) as i64))?;
    Ok(2)
}

fn math_sin(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'sin' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'sin' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.sin()))?;
    Ok(1)
}

fn math_sqrt(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'sqrt' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'sqrt' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.sqrt()))?;
    Ok(1)
}

fn math_tan(l: &mut LuaState) -> LuaResult<usize> {
    let x = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'tan' (number expected)".to_string()))?
        .as_number()
        .ok_or_else(|| l.error("bad argument #1 to 'tan' (number expected)".to_string()))?;
    l.push_value(LuaValue::float(x.tan()))?;
    Ok(1)
}

fn math_tointeger(l: &mut LuaState) -> LuaResult<usize> {
    let val = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'tointeger' (value expected)".to_string()))?;

    let result = if let Some(i) = val.as_integer() {
        LuaValue::integer(i)
    } else if let Some(f) = val.as_number() {
        float_to_integer(f)
    } else if let Some(s_id) = val.as_string_id() {
        // Try to parse string as integer
        let vm = l.vm_mut();
        if let Some(s) = vm.object_pool.get_string(s_id) {
            let s_str = s.as_str().trim();
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

    if f >= MIN_F && f < MAX_PLUS_ONE {
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
    let val = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'type' (value expected)".to_string()))?;

    let type_str = match val.kind() {
        LuaValueKind::Integer => "integer",
        LuaValueKind::Float => "float",
        _ => {
            l.push_value(LuaValue::nil())?;
            return Ok(1);
        }
    };

    let result = l.create_string(type_str);
    l.push_value(result)?;
    Ok(1)
}

fn math_ult(l: &mut LuaState) -> LuaResult<usize> {
    let m_value = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'ult' (integer expected)".to_string()))?;
    let Some(m) = m_value.as_integer() else {
        return Err(l.error("bad argument #1 to 'ult' (integer expected)".to_string()));
    };
    let n_value = l.get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'ult' (integer expected)".to_string()))?;
    let Some(n) = n_value.as_integer() else {
        return Err(l.error("bad argument #2 to 'ult' (integer expected)".to_string()));
    };
    // Unsigned less than
    let result = (m as u64) < (n as u64);
    l.push_value(LuaValue::boolean(result))?;
    Ok(1)
}
