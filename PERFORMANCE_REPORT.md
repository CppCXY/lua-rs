# Lua-RS Performance Report

> **Last Updated**: November 30, 2025  
> **Test Environment**: Windows 11, AMD Ryzen 7 5800X, Rust 1.89.0
> **Lua-RS Version**: main 
> **Native Lua Version**: Lua 5.4.6

## Executive Summary

Lua-RS has achieved **production-ready correctness** with **302/302 tests passing (100%)**. The interpreter delivers **40-100%+ of native Lua 5.4 performance** across most operations, with excellent performance in arithmetic and control flow operations.

### Key Performance Highlights

🏆 **Excellent Performance (>90% of native)**:
- **Integer addition**: **~220 M ops/sec** - Near native performance
- **Float multiplication**: **~210 M ops/sec** - Near native performance
- **Local variable access**: **~220 M ops/sec** - Extremely fast
- **Nested loops**: **~210 M ops/sec** - Excellent optimization
- **String length**: **~150 M ops/sec** - Faster than native!
- **Table access**: **~115 M ops/sec** - Solid performance
- **String equality**: **~82 M ops/sec** - Fast comparison

🎯 **Good Performance (>50% of native)**:
- **While loop**: ~125 M ops/sec
- **If-else control**: ~93 M ops/sec
- **Upvalue access**: ~95 M ops/sec
- **Table insertion**: ~50 M ops/sec
- **Simple function call**: ~24 M calls/sec
- **Bitwise operations**: ~80 M ops/sec
- **Integer division**: ~190 M ops/sec

📊 **Areas for Optimization**:
- **ipairs/pairs iteration**: ~13-15 K iters/sec (vs ~120 K for numeric for)
- **Vararg to table**: ~0.06 M ops/sec (GC overhead)
- **Object creation**: ~40-160 K ops/sec (allocation overhead)

---

## Latest Comprehensive Benchmark Results (November 30, 2025)

### Core Operations (10M iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| Integer addition | **219 M ops/sec** | Near native |
| Float multiplication | **200 M ops/sec** | Near native |
| Mixed operations | **111 M ops/sec** | Good |
| Local var access | **219 M ops/sec** | Excellent |
| Global var access | **43 M ops/sec** | 5x slower than local |
| Upvalue access | **96 M ops/sec** | Good |

### Control Flow (10M iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| If-else | **93 M ops/sec** | Good |
| While loop | **121 M ops/sec** | Excellent |
| Repeat-until | **110 M ops/sec** | Good |
| Nested loops | **218 M ops/sec** | Excellent |
| Numeric for | **122 K iters/sec** | Fast |

### Functions & Closures (1M iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| Simple function call | **22 M calls/sec** | Good |
| Recursive fib(25) | **0.010s** | Acceptable |
| Vararg function | **1.5 M calls/sec** | OK |
| Closure creation | **6.8 M ops/sec** | Good |
| Upvalue read/write | **22 M ops/sec** | Excellent |
| Nested closures | **18 M ops/sec** | Good |

### Multiple Returns (1M iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| Single return | **34 M ops/sec** | Excellent |
| Triple return | **15 M ops/sec** | Good |
| 10 returns | **4.8 M ops/sec** | OK |
| select('#') | **4.4 M ops/sec** | OK |
| table.pack | **4 M ops/sec** | OK |
| table.unpack | **8.9 M ops/sec** | Good |

### Tables (1M iterations unless noted)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| Table insertion | **51 M inserts/sec** | Excellent |
| Table access | **117 M accesses/sec** | Excellent |
| Hash table (100k) | **0.022s** | Fast |
| # operator | **44 M ops/sec** | Excellent |
| table.insert (end) | **25.7 M ops/sec** | Excellent |
| table.insert (mid) | **8.8 M ops/sec** | Good |
| table.remove | **16.3 M ops/sec** | Good |
| table.concat (1k) | **26 K ops/sec** | OK |
| table.sort (random) | **6.6 K ops/sec** | OK |

### Iterators (100K iterations × 1000 items)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| Numeric for | **122 K iters/sec** | Fast (baseline) |
| ipairs | **14.8 K iters/sec** | 8x slower than for |
| pairs (array) | **12.7 K iters/sec** | Iterator overhead |
| pairs (hash) | **14 K iters/sec** | Similar |
| next() | **14.9 K iters/sec** | Similar |
| Custom iterator | **11.2 K iters/sec** | Overhead |

### Strings (100K iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| Concatenation | **2.7 M ops/sec** | Good |
| String length | **185 M ops/sec** | Excellent |
| string.upper | **8.5 M ops/sec** | Good |
| string.lower | **7.9 M ops/sec** | Good |
| string.sub | **7.1 M ops/sec** | Good |
| string.find | **5.1 M ops/sec** | Good |
| string.format | **3.4 M ops/sec** | Good |
| string.match | **1.5 M ops/sec** | OK |
| string.gsub | **1.1 M ops/sec** | OK |
| String equality | **82 M ops/sec** | Excellent |

### Math Library (5M iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| Integer mul/add/mod | **103 M ops/sec** | Excellent |
| Float mul/add/div | **77 M ops/sec** | Good |
| math.sqrt | **22 M ops/sec** | Good |
| math.sin | **20 M ops/sec** | Good |
| math.floor/ceil | **11 M ops/sec** | OK |
| math.abs | **20 M ops/sec** | Good |
| math.random | **11 M ops/sec** | Good |
| Bitwise ops | **82 M ops/sec** | Excellent |
| Integer division | **170 M ops/sec** | Excellent |
| Power (^2) | **43 M ops/sec** | Good |

### Metatables & OOP (500K/100K iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| __index (function) | **6 M ops/sec** | Good |
| __index (table) | **19 M ops/sec** | Good |
| __newindex | **7.2 M ops/sec** | Good |
| __call | **13 M ops/sec** | Good |
| __len | **7.3 M ops/sec** | Good |
| rawget | **15.4 M ops/sec** | Good |
| Object creation | **41 K ops/sec** | Allocation overhead |
| Method call | **4.5 M calls/sec** | Good |
| Property access | **56 M ops/sec** | Excellent |

### Coroutines (100K iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| Create/resume/yield | **27 K cycles/sec** | OK |
| Repeated yield | **5.6 M yields/sec** | Good |
| coroutine.wrap | **22 K ops/sec** | OK |
| coroutine.status | **13 M ops/sec** | Excellent |

### Error Handling (100K iterations)
| Operation | Performance | Notes |
|-----------|-------------|-------|
| pcall (success) | **4.3 M ops/sec** | Good |
| pcall (error) | **3.6 M ops/sec** | Good |
| xpcall (error) | **1.8 M ops/sec** | OK |
| Direct call | **41 M ops/sec** | Baseline |
| assert (success) | **16 M ops/sec** | Good |

---

## Running Benchmarks

### Run All Benchmarks
```bash
# Using PowerShell script (compares with native Lua)
.\run_benchmarks.ps1

# Run with lua-rs only
.\target\release\lua.exe .\benchmarks\run_all.lua
```

### Individual Benchmarks
```bash
.\target\release\lua.exe .\benchmarks\bench_arithmetic.lua
.\target\release\lua.exe .\benchmarks\bench_tables.lua
.\target\release\lua.exe .\benchmarks\bench_strings.lua
# ... etc
```

### Benchmark Files (16 total)
- **Core**: bench_arithmetic, bench_control_flow, bench_locals
- **Functions**: bench_functions, bench_closures, bench_multiret
- **Tables**: bench_tables, bench_table_lib, bench_iterators
- **Strings**: bench_strings, bench_string_lib
- **Math**: bench_math
- **Advanced**: bench_metatables, bench_oop, bench_coroutines, bench_errors

---

## Performance History

### November 30, 2025 - Comprehensive Benchmarks & Optimizations
- Added 11 new benchmark files (16 total)
- Fixed floating-point for loop bug
- Optimized `call_function_internal` - reduced code by ~300 lines
- All 302 tests passing
- Total benchmark runtime: ~120 seconds

### November 29, 2025 - While Loop Optimization
- Optimized while/repeat loop bytecode generation
- While loop at **85% of native**
- Nested loops at **97% of native**

### November 24, 2025 - CallFrame Optimization  
- Implemented code pointer caching in CallFrame
- Eliminated HashMap lookups in hot paths
- Major improvements across all benchmarks

---

## Architecture Notes

### Performance Characteristics
- **Local variables are ~5x faster** than global variables
- **Numeric for is ~8-9x faster** than ipairs/pairs
- **Property access** is very fast (~56 M ops/sec)
- **Function calls** are efficient (~22 M calls/sec)
- **Bitwise operations** are very fast (~82 M ops/sec)

### Known Performance Bottlenecks
1. **ipairs/pairs iteration**: Iterator protocol overhead
2. **Object creation**: Allocation and setmetatable overhead
3. **Vararg to table**: Extra allocation and copying
4. **Complex pattern matching**: Regex-like overhead

### Optimization Opportunities
1. Iterator fast-path for ipairs/pairs
2. Object pooling for common patterns
3. Inlining for small functions
4. Better GC tuning for allocation-heavy code
