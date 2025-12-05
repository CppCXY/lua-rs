// Math library
// Implements: abs, acos, asin, atan, ceil, cos, deg, exp, floor, fmod,
// log, max, min, modf, rad, random, randomseed, sin, sqrt, tan, tointeger,
// type, ult, pi, huge, maxinteger, mininteger

use crate::lib_registry::{LibraryModule, get_arg, require_arg};
use crate::lua_value::{LuaValue, LuaValueKind, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

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

/// OPTIMIZED: Get number without allocating error message on success path
#[inline(always)]
fn get_number_fast(vm: &LuaVM, idx: usize) -> Option<f64> {
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;

    if idx < top {
        let value = vm.register_stack[base_ptr + idx];
        value.as_number()
    } else {
        None
    }
}

fn get_number(vm: &mut LuaVM, idx: usize, func_name: &str) -> LuaResult<f64> {
    if let Some(n) = get_number_fast(vm, idx) {
        Ok(n)
    } else {
        Err(vm.error(format!(
            "bad argument #{} to '{}' (number expected)",
            idx, func_name
        )))
    }
}

fn math_abs(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;

    if top <= 1 {
        return Err(vm.error("bad argument #1 to 'abs' (number expected)".to_string()));
    }

    let value = vm.register_stack[base_ptr + 1];

    // Fast path: preserve integer type
    if let Some(i) = value.as_integer() {
        return Ok(MultiValue::single(LuaValue::integer(i.abs())));
    }

    if let Some(f) = value.as_float() {
        return Ok(MultiValue::single(LuaValue::float(f.abs())));
    }

    Err(vm.error("bad argument #1 to 'abs' (number expected)".to_string()))
}

fn math_acos(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.acos")?;
    Ok(MultiValue::single(LuaValue::float(x.acos())))
}

fn math_asin(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.asin")?;
    Ok(MultiValue::single(LuaValue::float(x.asin())))
}

fn math_atan(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let y = get_number(vm, 1, "math.atan")?;
    let x = get_arg(vm, 2).and_then(|v| v.as_number()).unwrap_or(1.0);
    Ok(MultiValue::single(LuaValue::float(y.atan2(x))))
}

fn math_ceil(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;

    if top <= 1 {
        return Err(vm.error("bad argument #1 to 'ceil' (number expected)".to_string()));
    }

    let value = vm.register_stack[base_ptr + 1];

    // Fast path: integers are already ceil'd
    if let Some(i) = value.as_integer() {
        return Ok(MultiValue::single(LuaValue::integer(i)));
    }

    if let Some(f) = value.as_float() {
        return Ok(MultiValue::single(LuaValue::integer(f.ceil() as i64)));
    }

    Err(vm.error("bad argument #1 to 'ceil' (number expected)".to_string()))
}

fn math_cos(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.cos")?;
    Ok(MultiValue::single(LuaValue::float(x.cos())))
}

fn math_deg(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.deg")?;
    Ok(MultiValue::single(LuaValue::float(x.to_degrees())))
}

fn math_exp(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.exp")?;
    Ok(MultiValue::single(LuaValue::float(x.exp())))
}

fn math_floor(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;

    if top <= 1 {
        return Err(vm.error("bad argument #1 to 'floor' (number expected)".to_string()));
    }

    let value = vm.register_stack[base_ptr + 1];

    // Fast path: integers are already floor'd
    if let Some(i) = value.as_integer() {
        return Ok(MultiValue::single(LuaValue::integer(i)));
    }

    if let Some(f) = value.as_float() {
        return Ok(MultiValue::single(LuaValue::integer(f.floor() as i64)));
    }

    Err(vm.error("bad argument #1 to 'floor' (number expected)".to_string()))
}

fn math_fmod(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.fmod")?;
    let y = get_number(vm, 2, "math.fmod")?;
    Ok(MultiValue::single(LuaValue::float(x % y)))
}

fn math_log(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.log")?;
    let base = get_arg(vm, 2).and_then(|v| v.as_number());

    let result = if let Some(b) = base { x.log(b) } else { x.ln() };

    Ok(MultiValue::single(LuaValue::float(result)))
}

/// OPTIMIZED: Direct stack access without get_arg overhead
fn math_max(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;
    let argc = top.saturating_sub(1);

    if argc == 0 {
        return Err(vm.error("bad argument to 'math.max' (value expected)".to_string()));
    }

    // Get first argument directly
    let first = vm.register_stack[base_ptr + 1];
    let mut max_val = first
        .as_number()
        .ok_or_else(|| vm.error("bad argument to 'math.max' (number expected)".to_string()))?;
    let mut max_arg = first;

    // Compare with rest - direct stack access
    for i in 2..=argc {
        let arg = vm.register_stack[base_ptr + i];
        let val = arg
            .as_number()
            .ok_or_else(|| vm.error("bad argument to 'math.max' (number expected)".to_string()))?;
        if val > max_val {
            max_val = val;
            max_arg = arg;
        }
    }

    Ok(MultiValue::single(max_arg))
}

/// OPTIMIZED: Direct stack access without get_arg overhead
fn math_min(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;
    let argc = top.saturating_sub(1);

    if argc == 0 {
        return Err(vm.error("bad argument to 'math.min' (value expected)".to_string()));
    }

    // Get first argument directly
    let first = vm.register_stack[base_ptr + 1];
    let mut min_val = first
        .as_number()
        .ok_or_else(|| vm.error("bad argument to 'math.min' (number expected)".to_string()))?;
    let mut min_arg = first;

    // Compare with rest - direct stack access
    for i in 2..=argc {
        let arg = vm.register_stack[base_ptr + i];
        let val = arg
            .as_number()
            .ok_or_else(|| vm.error("bad argument to 'math.min' (number expected)".to_string()))?;
        if val < min_val {
            min_val = val;
            min_arg = arg;
        }
    }

    Ok(MultiValue::single(min_arg))
}

fn math_modf(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.modf")?;
    let int_part = x.trunc();
    let frac_part = x - int_part;

    Ok(MultiValue::multiple(vec![
        LuaValue::float(int_part),
        LuaValue::float(frac_part),
    ]))
}

fn math_rad(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.rad")?;
    Ok(MultiValue::single(LuaValue::float(x.to_radians())))
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

fn math_random(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let frame = vm.current_frame();
    let top = frame.top as usize;
    let argc = top.saturating_sub(1);

    // Generate random u64 and convert to [0, 1) float
    let rand_u64 = xorshift64();
    let random = (rand_u64 >> 11) as f64 / (1u64 << 53) as f64;

    match argc {
        0 => Ok(MultiValue::single(LuaValue::float(random))),
        1 => {
            let m = get_number(vm, 1, "math.random")? as i64;
            if m < 1 {
                return Err(vm.error("bad argument #1 to 'random' (interval is empty)".to_string()));
            }
            let result = (random * m as f64).floor() as i64 + 1;
            Ok(MultiValue::single(LuaValue::integer(result)))
        }
        _ => {
            let m = get_number(vm, 1, "math.random")? as i64;
            let n = get_number(vm, 2, "math.random")? as i64;
            if m > n {
                return Err(vm.error("bad argument #1 to 'random' (interval is empty)".to_string()));
            }
            let range = (n - m + 1) as f64;
            let result = m + (random * range).floor() as i64;
            Ok(MultiValue::single(LuaValue::integer(result)))
        }
    }
}

fn math_randomseed(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.randomseed")? as u64;
    // Seed the random state
    RANDOM_STATE.with(|state| {
        // Ensure seed is non-zero for xorshift
        let seed = if x == 0 { 0x853c49e6748fea9b_u64 } else { x };
        state.set(seed);
    });
    Ok(MultiValue::empty())
}

fn math_sin(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.sin")?;
    Ok(MultiValue::single(LuaValue::float(x.sin())))
}

fn math_sqrt(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.sqrt")?;
    Ok(MultiValue::single(LuaValue::float(x.sqrt())))
}

fn math_tan(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 1, "math.tan")?;
    Ok(MultiValue::single(LuaValue::float(x.tan())))
}

fn math_tointeger(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let val = require_arg(vm, 1, "math.tointeger")?;

    let result = if let Some(i) = val.as_integer() {
        LuaValue::integer(i)
    } else if let Some(f) = val.as_number() {
        if f.fract() == 0.0 && f.is_finite() {
            LuaValue::integer(f as i64)
        } else {
            LuaValue::nil()
        }
    } else if let Some(s_id) = val.as_string_id() {
        // Try to parse string as integer
        if let Some(s) = vm.object_pool.get_string(s_id) {
            let s_str = s.as_str().trim();
            if let Ok(i) = s_str.parse::<i64>() {
                LuaValue::integer(i)
            } else if let Ok(f) = s_str.parse::<f64>() {
                // String is a float, check if it's a whole number
                if f.fract() == 0.0 && f.is_finite() {
                    LuaValue::integer(f as i64)
                } else {
                    LuaValue::nil()
                }
            } else {
                LuaValue::nil()
            }
        } else {
            LuaValue::nil()
        }
    } else {
        LuaValue::nil()
    };

    Ok(MultiValue::single(result))
}

fn math_type(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let val = require_arg(vm, 1, "math.type")?;

    let type_str = match val.kind() {
        LuaValueKind::Integer => "integer",
        LuaValueKind::Float => "float",
        _ => return Ok(MultiValue::single(LuaValue::nil())),
    };

    let result = vm.create_string(type_str);
    Ok(MultiValue::single(result))
}

fn math_ult(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let m_value = require_arg(vm, 1, "math.ult")?;
    let Some(m) = m_value.as_integer() else {
        return Err(vm.error("bad argument #1 to 'math.ult' (integer expected)".to_string()));
    };
    let n_value = require_arg(vm, 2, "math.ult")?;
    let Some(n) = n_value.as_integer() else {
        return Err(vm.error("bad argument #2 to 'math.ult' (integer expected)".to_string()));
    };
    // Unsigned less than
    let result = (m as u64) < (n as u64);
    Ok(MultiValue::single(LuaValue::boolean(result)))
}
