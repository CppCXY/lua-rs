// Benchmark: NaN-Boxing vs Enum-based LuaValue
// 
// Run with: cargo test --release --lib nanbox_bench -- --nocapture

#[cfg(test)]
mod nanbox_bench {
    use crate::lua_value::nanbox::LuaValue as NanBoxValue;
    use crate::lua_value::LuaValue as EnumValue;
    use std::time::Instant;

    const ITERATIONS: usize = 10_000_000;

    #[test]
    fn bench_size() {
        println!("\n=== Size Comparison ===");
        println!("NanBox: {} bytes", std::mem::size_of::<NanBoxValue>());
        println!("Enum:   {} bytes", std::mem::size_of::<EnumValue>());
    }

    #[test]
    fn bench_integer_creation() {
        println!("\n=== Integer Creation ===");
        
        // NanBox
        let start = Instant::now();
        let mut sum = 0i64;
        for i in 0..ITERATIONS {
            let v = NanBoxValue::integer(i as i64);
            sum += v.as_integer().unwrap();
        }
        let nanbox_time = start.elapsed();
        println!("NanBox: {:?} (sum: {})", nanbox_time, sum);
        
        // Enum
        let start = Instant::now();
        let mut sum = 0i64;
        for i in 0..ITERATIONS {
            let v = EnumValue::integer(i as i64);
            if let EnumValue::Integer(val) = v {
                sum += val;
            }
        }
        let enum_time = start.elapsed();
        println!("Enum:   {:?} (sum: {})", enum_time, sum);
        println!("Speedup: {:.2}x", enum_time.as_secs_f64() / nanbox_time.as_secs_f64());
    }

    #[test]
    fn bench_type_check() {
        println!("\n=== Type Check (is_integer) ===");
        
        let nanbox_vals: Vec<_> = (0..1000).map(|i| NanBoxValue::integer(i)).collect();
        let enum_vals: Vec<_> = (0..1000).map(|i| EnumValue::integer(i)).collect();
        
        // NanBox
        let start = Instant::now();
        let mut count = 0;
        for _ in 0..10000 {
            for v in &nanbox_vals {
                if v.is_integer() {
                    count += 1;
                }
            }
        }
        let nanbox_time = start.elapsed();
        println!("NanBox: {:?} (count: {})", nanbox_time, count);
        
        // Enum
        let start = Instant::now();
        let mut count = 0;
        for _ in 0..10000 {
            for v in &enum_vals {
                if matches!(v, EnumValue::Integer(_)) {
                    count += 1;
                }
            }
        }
        let enum_time = start.elapsed();
        println!("Enum:   {:?} (count: {})", enum_time, count);
        println!("Speedup: {:.2}x", enum_time.as_secs_f64() / nanbox_time.as_secs_f64());
    }

    #[test]
    fn bench_arithmetic() {
        println!("\n=== Integer Addition ===");
        
        // NanBox
        let start = Instant::now();
        let mut result = NanBoxValue::integer(0);
        for i in 0..ITERATIONS {
            let v = NanBoxValue::integer(i as i64 % 100);
            result = result.add(&v).unwrap();
        }
        let nanbox_time = start.elapsed();
        println!("NanBox: {:?} (result: {:?})", nanbox_time, result.as_integer());
        
        // Enum (manual)
        let start = Instant::now();
        let mut result = 0i64;
        for i in 0..ITERATIONS {
            let v = EnumValue::integer(i as i64 % 100);
            if let EnumValue::Integer(val) = v {
                result += val;
            }
        }
        let enum_time = start.elapsed();
        println!("Enum:   {:?} (result: {})", enum_time, result);
        println!("Speedup: {:.2}x", enum_time.as_secs_f64() / nanbox_time.as_secs_f64());
    }

    #[test]
    fn bench_for_loop_simulation() {
        println!("\n=== For Loop Simulation (1M iterations) ===");
        
        // NanBox: simulate ForLoop register updates
        let start = Instant::now();
        let mut idx = NanBoxValue::integer(0);
        let limit = NanBoxValue::integer(1_000_000);
        let step = NanBoxValue::integer(1);
        let mut _loop_var = NanBoxValue::integer(0);
        
        while idx.as_integer().unwrap() <= limit.as_integer().unwrap() {
            let new_idx = idx.as_integer().unwrap() + step.as_integer().unwrap();
            idx = NanBoxValue::integer(new_idx);
            _loop_var = idx;
        }
        let nanbox_time = start.elapsed();
        println!("NanBox: {:?}", nanbox_time);
        
        // NanBox with raw bits (ultra-fast)
        let start = Instant::now();
        let mut idx = NanBoxValue::integer(0);
        let _limit = NanBoxValue::integer(1_000_000);
        let step_val = 1i64;
        
        loop {
            let current = idx.as_integer().unwrap();
            if current > 1_000_000 {
                break;
            }
            let new_idx = current + step_val;
            idx = NanBoxValue::integer(new_idx);
        }
        let nanbox_raw_time = start.elapsed();
        println!("NanBox (raw bits): {:?}", nanbox_raw_time);
        
        // Enum
        let start = Instant::now();
        let mut idx = EnumValue::integer(0);
        let limit = EnumValue::integer(1_000_000);
        let step = EnumValue::integer(1);
        let mut _loop_var = EnumValue::integer(0);
        
        loop {
            let (i, l, s) = match (&idx, &limit, &step) {
                (EnumValue::Integer(i), EnumValue::Integer(l), EnumValue::Integer(s)) => (*i, *l, *s),
                _ => break,
            };
            if i > l { break; }
            let new_idx = i + s;
            idx = EnumValue::integer(new_idx);
            _loop_var = idx.clone();
        }
        let enum_time = start.elapsed();
        println!("Enum:   {:?}", enum_time);
        println!("Speedup over enum: {:.2}x", enum_time.as_secs_f64() / nanbox_time.as_secs_f64());
        println!("Speedup (raw):     {:.2}x", enum_time.as_secs_f64() / nanbox_raw_time.as_secs_f64());
    }
}
