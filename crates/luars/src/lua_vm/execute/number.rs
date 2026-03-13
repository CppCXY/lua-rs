// ============================================================
// Integer-Float comparison helpers (Lua 5.5 semantics)
// These handle the tricky edge cases where converting to f64
// loses precision (e.g., i64::MAX as f64 rounds to 2^63).
// ============================================================

use crate::{
    LuaValue,
    lua_vm::execute::helper::{ttisfloat, ttisinteger},
};

/// Is integer i less than float f?  (i < f)
/// Handles NaN, infinities, and precision loss at i64 boundaries.
#[inline]
pub fn int_lt_float(i: i64, f: f64) -> bool {
    if f.is_nan() {
        return false;
    }
    // i64::MAX as f64 = 2^63 (rounds up), so f >= 2^63 means f > any i64
    if f >= (i64::MAX as f64) {
        return true;
    }
    // i64::MIN as f64 = -2^63 (exact), so f < -2^63 means f < any i64
    if f < (i64::MIN as f64) {
        return false;
    }
    // f is in castable range: truncate toward zero
    let fi = f as i64;
    if i < fi {
        true
    } else if i > fi {
        false
    } else {
        // i == fi: true iff f has a positive fractional part beyond fi
        f > (fi as f64)
    }
}

/// Is float f less than integer i?  (f < i)
#[inline]
pub fn float_lt_int(f: f64, i: i64) -> bool {
    if f.is_nan() {
        return false;
    }
    if f >= (i64::MAX as f64) {
        return false;
    }
    if f < (i64::MIN as f64) {
        return true;
    }
    let fi = f as i64;
    if fi < i {
        true
    } else if fi > i {
        false
    } else {
        // fi == i: true iff f has a negative fractional part (truncated away)
        f < (fi as f64)
    }
}

/// Is integer i less than or equal to float f?  (i <= f)
#[inline]
pub fn int_le_float(i: i64, f: f64) -> bool {
    // NaN: i <= NaN is always false
    if f.is_nan() {
        return false;
    }
    !float_lt_int(f, i)
}

/// Is float f less than or equal to integer i?  (f <= i)
#[inline]
pub fn float_le_int(f: f64, i: i64) -> bool {
    // NaN: NaN <= i is always false
    if f.is_nan() {
        return false;
    }
    !int_lt_float(i, f)
}

pub fn lt_num(a: &LuaValue, b: &LuaValue) -> bool {
    if ttisinteger(a) {
        let ai = a.ivalue();
        if ttisinteger(b) {
            let bi = b.ivalue();
            ai < bi
        } else if ttisfloat(b) {
            int_le_float(ai, b.fltvalue())
        } else {
            // unrecachable: caller should have ensured both are numbers
            false
        }
    } else {
        let af = a.fltvalue();
        if ttisfloat(b) {
            let bf = b.fltvalue();
            af < bf
        } else if ttisinteger(b) {
            float_le_int(af, b.ivalue())
        } else {
            // unrecachable: caller should have ensured both are numbers
            false
        }
    }
}

pub fn le_num(a: &LuaValue, b: &LuaValue) -> bool {
    if ttisinteger(a) {
        let ai = a.ivalue();
        if ttisinteger(b) {
            let bi = b.ivalue();
            ai <= bi
        } else if ttisfloat(b) {
            int_le_float(ai, b.fltvalue())
        } else {
            // unrecachable: caller should have ensured both are numbers
            false
        }
    } else {
        let af = a.fltvalue();
        if ttisfloat(b) {
            let bf = b.fltvalue();
            af <= bf
        } else if ttisinteger(b) {
            float_le_int(af, b.ivalue())
        } else {
            // unrecachable: caller should have ensured both are numbers
            false
        }
    }
}
