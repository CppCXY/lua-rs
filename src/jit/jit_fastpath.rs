// Fast-path implementations for common Lua patterns
// These are hand-optimized Rust implementations that bypass the interpreter

use crate::value::LuaValue;

/// Fast path for integer accumulation loop: sum = 0; for i=start,end do sum = sum + i end
pub fn fast_integer_accum_loop(start: i64, end: i64) -> i64 {
    // Direct implementation without any overhead
    let mut sum: i64 = 0;
    for i in start..=end {
        sum = sum.wrapping_add(i);
    }
    sum
}

/// Fast path for integer range sum with step: for i=start,end,step do sum = sum + i end
pub fn fast_integer_range_sum(start: i64, end: i64, step: i64, initial: i64) -> i64 {
    let mut sum = initial;
    if step > 0 {
        let mut i = start;
        while i <= end {
            sum = sum.wrapping_add(i);
            i = i.wrapping_add(step);
        }
    } else if step < 0 {
        let mut i = start;
        while i >= end {
            sum = sum.wrapping_add(i);
            i = i.wrapping_add(step);
        }
    }
    sum
}

/// Fast path for simple arithmetic expression: a + b + c + ...
pub fn fast_integer_add_chain(values: &[i64]) -> i64 {
    values.iter().fold(0i64, |acc, &v| acc.wrapping_add(v))
}

/// Fast path for integer multiplication chain: a * b * c * ...
pub fn fast_integer_mul_chain(values: &[i64]) -> i64 {
    values.iter().fold(1i64, |acc, &v| acc.wrapping_mul(v))
}

/// Check if value is integer (for guard checks)
#[inline(always)]
pub fn is_integer(value: &LuaValue) -> bool {
    matches!(value, LuaValue::Integer(_))
}

/// Extract integer value (unsafe, caller must ensure it's integer)
#[inline(always)]
pub fn extract_integer_unchecked(value: &LuaValue) -> i64 {
    match value {
        LuaValue::Integer(i) => *i,
        _ => unreachable!("Called extract_integer_unchecked on non-integer"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_integer_accum_loop() {
        // sum of 1..100 = 5050
        assert_eq!(fast_integer_accum_loop(1, 100), 5050);
        
        // sum of 1..10000 = 50005000
        assert_eq!(fast_integer_accum_loop(1, 10000), 50005000);
    }

    #[test]
    fn test_fast_integer_range_sum() {
        // sum of 1,3,5,7,9 with initial 0
        assert_eq!(fast_integer_range_sum(1, 10, 2, 0), 1 + 3 + 5 + 7 + 9);
    }

    #[test]
    fn test_fast_add_chain() {
        assert_eq!(fast_integer_add_chain(&[1, 2, 3, 4, 5]), 15);
    }

    #[test]
    fn test_fast_mul_chain() {
        assert_eq!(fast_integer_mul_chain(&[2, 3, 4]), 24);
    }
}
