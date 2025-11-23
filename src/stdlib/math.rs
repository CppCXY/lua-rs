// Math library
// Implements: abs, acos, asin, atan, ceil, cos, deg, exp, floor, fmod,
// log, max, min, modf, rad, random, randomseed, sin, sqrt, tan, tointeger,
// type, ult, pi, huge, maxinteger, mininteger

use crate::lib_registry::{LibraryModule, get_arg, get_args, require_arg};
use crate::lua_value::{LuaValue, LuaValueKind, MultiValue};
use crate::lua_vm::{LuaError, LuaResult, LuaVM};

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

fn get_number(vm: &LuaVM, idx: usize, func_name: &str) -> LuaResult<f64> {
    require_arg(vm, idx, func_name)?.as_number().ok_or_else(|| {
        LuaError::RuntimeError(format!(
            "bad argument #{} to '{}' (number expected)",
            idx + 1,
            func_name
        ))
    })
}

fn math_abs(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.abs")?;
    Ok(MultiValue::single(LuaValue::float(x.abs())))
}

fn math_acos(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.acos")?;
    Ok(MultiValue::single(LuaValue::float(x.acos())))
}

fn math_asin(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.asin")?;
    Ok(MultiValue::single(LuaValue::float(x.asin())))
}

fn math_atan(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let y = get_number(vm, 0, "math.atan")?;
    let x = get_arg(vm, 1).and_then(|v| v.as_number()).unwrap_or(1.0);
    Ok(MultiValue::single(LuaValue::float(y.atan2(x))))
}

fn math_ceil(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.ceil")?;
    Ok(MultiValue::single(LuaValue::float(x.ceil())))
}

fn math_cos(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.cos")?;
    Ok(MultiValue::single(LuaValue::float(x.cos())))
}

fn math_deg(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.deg")?;
    Ok(MultiValue::single(LuaValue::float(x.to_degrees())))
}

fn math_exp(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.exp")?;
    Ok(MultiValue::single(LuaValue::float(x.exp())))
}

fn math_floor(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.floor")?;
    Ok(MultiValue::single(LuaValue::integer(x.floor() as i64)))
}

fn math_fmod(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.fmod")?;
    let y = get_number(vm, 1, "math.fmod")?;
    Ok(MultiValue::single(LuaValue::float(x % y)))
}

fn math_log(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.log")?;
    let base = get_arg(vm, 1).and_then(|v| v.as_number());

    let result = if let Some(b) = base { x.log(b) } else { x.ln() };

    Ok(MultiValue::single(LuaValue::float(result)))
}

fn math_max(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    if args.is_empty() {
        return Err(LuaError::RuntimeError(
            "bad argument to 'math.max' (value expected)".to_string(),
        ));
    }

    // Keep the original value and its index to preserve type (integer vs float)
    let mut max_idx = 0;
    let mut max_val = args[0].as_number().ok_or_else(|| {
        LuaError::RuntimeError("bad argument to 'math.max' (number expected)".to_string())
    })?;

    for (i, arg) in args.iter().enumerate().skip(1) {
        let val = arg.as_number().ok_or_else(|| {
            LuaError::RuntimeError("bad argument to 'math.max' (number expected)".to_string())
        })?;
        if val > max_val {
            max_val = val;
            max_idx = i;
        }
    }

    // Return the original value (preserves integer/float type)
    Ok(MultiValue::single(args[max_idx]))
}

fn math_min(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    if args.is_empty() {
        return Err(LuaError::RuntimeError(
            "bad argument to 'math.min' (value expected)".to_string(),
        ));
    }

    // Keep the original value and its index to preserve type (integer vs float)
    let mut min_idx = 0;
    let mut min_val = args[0].as_number().ok_or_else(|| {
        LuaError::RuntimeError("bad argument to 'math.min' (number expected)".to_string())
    })?;

    for (i, arg) in args.iter().enumerate().skip(1) {
        let val = arg.as_number().ok_or_else(|| {
            LuaError::RuntimeError("bad argument to 'math.min' (number expected)".to_string())
        })?;
        if val < min_val {
            min_val = val;
            min_idx = i;
        }
    }

    // Return the original value (preserves integer/float type)
    Ok(MultiValue::single(args[min_idx]))
}

fn math_modf(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.modf")?;
    let int_part = x.trunc();
    let frac_part = x - int_part;

    Ok(MultiValue::multiple(vec![
        LuaValue::float(int_part),
        LuaValue::float(frac_part),
    ]))
}

fn math_rad(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.rad")?;
    Ok(MultiValue::single(LuaValue::float(x.to_radians())))
}

fn math_random(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hash, Hasher};

    let argc = crate::lib_registry::arg_count(vm);

    // Simple pseudo-random using hash
    let mut hasher = RandomState::new().build_hasher();
    std::time::SystemTime::now().hash(&mut hasher);
    let hash = hasher.finish();
    let random = (hash % 1000000) as f64 / 1000000.0;

    match argc {
        0 => Ok(MultiValue::single(LuaValue::float(random))),
        1 => {
            let m = get_number(vm, 0, "math.random")? as i64;
            let result = (random * m as f64).floor() as i64 + 1;
            Ok(MultiValue::single(LuaValue::integer(result)))
        }
        _ => {
            let m = get_number(vm, 0, "math.random")? as i64;
            let n = get_number(vm, 1, "math.random")? as i64;
            let range = (n - m + 1) as f64;
            let result = m + (random * range).floor() as i64;
            Ok(MultiValue::single(LuaValue::integer(result)))
        }
    }
}

fn math_randomseed(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Seed is ignored in our simple implementation
    let _x = get_number(vm, 0, "math.randomseed")?;
    Ok(MultiValue::empty())
}

fn math_sin(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.sin")?;
    Ok(MultiValue::single(LuaValue::float(x.sin())))
}

fn math_sqrt(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.sqrt")?;
    Ok(MultiValue::single(LuaValue::float(x.sqrt())))
}

fn math_tan(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let x = get_number(vm, 0, "math.tan")?;
    Ok(MultiValue::single(LuaValue::float(x.tan())))
}

fn math_tointeger(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let val = require_arg(vm, 0, "math.tointeger")?;

    let result = if let Some(i) = val.as_integer() {
        LuaValue::integer(i)
    } else if let Some(f) = val.as_number() {
        if f.fract() == 0.0 && f.is_finite() {
            LuaValue::integer(f as i64)
        } else {
            LuaValue::nil()
        }
    } else if let Some(s) = val.as_lua_string() {
        // Try to parse string as integer
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
    };

    Ok(MultiValue::single(result))
}

fn math_type(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let val = require_arg(vm, 0, "math.type")?;

    let type_str = match val.kind() {
        LuaValueKind::Integer => "integer",
        LuaValueKind::Float => "float",
        _ => return Ok(MultiValue::single(LuaValue::nil())),
    };

    let result = vm.create_string(type_str);
    Ok(MultiValue::single(result))
}

fn math_ult(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let m = require_arg(vm, 0, "math.ult")?
        .as_integer()
        .ok_or_else(|| {
            LuaError::RuntimeError("bad argument #1 to 'math.ult' (integer expected)".to_string())
        })?;

    let n = require_arg(vm, 1, "math.ult")?
        .as_integer()
        .ok_or_else(|| {
            LuaError::RuntimeError("bad argument #2 to 'math.ult' (integer expected)".to_string())
        })?;
    // Unsigned less than
    let result = (m as u64) < (n as u64);
    Ok(MultiValue::single(LuaValue::boolean(result)))
}
