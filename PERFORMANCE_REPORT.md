# Lua-RS Performance Report

> **Last Updated**: November 30, 2025  
> **Test Environment**: Windows 11, AMD Ryzen 7 5800X, Rust 1.89.0
> **Lua-RS Version**: main 
> **Native Lua Version**: Lua 5.4.6

## Executive Summary

Lua-RS has achieved **production-ready correctness** with **302/302 tests passing (100%)**. The interpreter delivers **40-100%+ of native Lua 5.4 performance** across most operations, with excellent performance in arithmetic and control flow operations.

### Key Performance Highlights

🏆 **Excellent Performance (>90% of native)**:
- **Integer addition**: **101%** of native (251.89 M/s vs 250.00 M/s) - **Faster than native!**
- **Float multiplication**: **99%** of native (248.50 M/s vs 250.00 M/s)
- **Table insertion**: **101%** of native (71.99 M/s vs 71.43 M/s) - **Faster than native!**
- **Nested loops**: **97%** of native (243.30 M/s vs 250.00 M/s)

🎯 **Good Performance (60-90% of native)**:
- **While loop**: **85%** (127.10 M/s vs 149.25 M/s)
- **If-else control**: **84%** (99.71 M/s vs 119.05 M/s)
- **Mixed operations**: **80%** (125.22 M/s vs 156.25 M/s)
- **Table access**: **77%** (128.46 M/s vs 166.67 M/s)
- **Hash table insertion**: **136%** (0.022s vs 0.030s) - **Faster than native!**
- **Repeat-until**: **61%** (114.36 M/s vs 188.68 M/s)
- **String concatenation**: **60%** (2748 K/s vs 4545 K/s)
- **Simple function call**: **59%** (32.77 M/s vs 55.56 M/s)

📊 **Acceptable Performance (30-60% of native)**:
- **Array creation & access**: **45%** (5.10 M/s vs 11.24 M/s)
- **Recursive fib(25)**: **40%** (0.010s vs 0.004s)
- **Vararg function**: **36%** (1.29 M/s vs 3.58 M/s)
- **ipairs iteration**: **31%** (6.785s vs 2.098s)
- **string.sub**: **33%** (8155 K/s vs 25000 K/s)
- **string.find**: **33%** (5553 K/s vs 16666 K/s)

🏆 **Faster than Native**:
- **string.gsub**: **146%** (0.104s vs 0.152s) - **46% faster!**
- **Hash table insertion**: **136%** (0.022s vs 0.030s) - **36% faster!**

---

## Latest Benchmark Results (November 30, 2025)

### Arithmetic Operations
| Operation | Lua-RS | Native Lua | % of Native | Status |
|-----------|--------|-----------|-------------|--------|
| Integer addition | **251.89 M/s** | 250.00 M/s | **101%** | Excellent 🏆 |
| Float multiplication | **248.50 M/s** | 250.00 M/s | **99%** | Excellent 🏆 |
| Mixed operations | **125.22 M/s** | 156.25 M/s | **80%** | Good |

### Function Calls
| Operation | Lua-RS | Native Lua | % of Native | Status |
|-----------|--------|-----------|-------------|--------|
| Simple function call | **32.77 M/s** | 55.56 M/s | **59%** | Good |
| Recursive fib(25) | **0.010s** | 0.004s | **40%** | Acceptable |
| Vararg function | **1.29 M/s** | 3.58 M/s | **36%** | Acceptable |

### Table Operations
| Operation | Lua-RS | Native Lua | % of Native | Status |
|-----------|--------|-----------|-------------|--------|
| Array creation & access | **5.10 M/s** | 11.24 M/s | **45%** | Acceptable |
| Table insertion | **71.99 M/s** | 71.43 M/s | **101%** | Excellent 🏆 |
| Table access | **128.46 M/s** | 166.67 M/s | **77%** | Good |
| Hash table insertion (100k) | **0.022s** | 0.030s | **136%** | Excellent 🏆 |
| ipairs iteration (100×1M) | **6.785s** | 2.098s | **31%** | Needs optimization |

### String Operations
| Operation | Lua-RS | Native Lua | % of Native | Status |
|-----------|--------|-----------|-------------|--------|
| String concatenation | **2748.53 K/s** | 4545.45 K/s | **60%** | Good |
| String length | **156.99 M/s** | 100.00 M/s | **157%** | Excellent 🏆 |
| string.sub | **8155.08 K/s** | 25000.00 K/s | **33%** | Acceptable |
| string.find | **5553.24 K/s** | 16666.67 K/s | **33%** | Acceptable |
| string.gsub (10k) | **0.104s** | 0.152s | **146%** | Excellent 🏆 |

### Control Flow
| Operation | Lua-RS | Native Lua | % of Native | Status |
|-----------|--------|-----------|-------------|--------|
| If-else | **99.71 M/s** | 119.05 M/s | **84%** | Good |
| While loop | **127.10 M/s** | 149.25 M/s | **85%** | Good |
| Repeat-until | **114.36 M/s** | 188.68 M/s | **61%** | Good |
| Nested loops (1000×1000) | **243.30 M/s** | 250.00 M/s | **97%** | Excellent 🏆 |

---

## Running Benchmarks

### Windows (PowerShell)
```powershell
.\run_benchmarks.ps1
```

### Linux/macOS (Bash)
```bash
chmod +x run_benchmarks.sh
./run_benchmarks.sh
```

### CI
Performance benchmarks run automatically on push to `main` or `refactor` branches. See the [Benchmarks workflow](https://github.com/CppCXY/lua-rs/actions/workflows/benchmarks.yml) for cross-platform results.

---

## Performance History

### November 30, 2025 - call_function_internal Optimization
- Eliminated duplicate dispatch loop in `call_function_internal`
- Now directly calls `luavm_execute` instead of copying 300+ lines of dispatch code
- Reduced code size, improved CPU cache efficiency
- Integer addition now **101% of native** (faster than native Lua!)
- Float multiplication now **99% of native**
- Table insertion now **101% of native** (faster than native Lua!)

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

### Why Some Operations are Faster Than Native
- **Integer addition/Table insertion**: Rust's optimizations for integer operations
- **string.gsub**: Rust's string handling is more efficient for pattern matching
- **Hash table insertion**: Optimized Lua-style open addressing hash table
- **String length**: Direct access to pre-computed length field

### Known Performance Bottlenecks
1. **ipairs iteration**: Iterator overhead compared to C implementation
2. **Vararg functions**: Extra allocation and copying overhead
3. **Recursive calls**: Frame allocation overhead
4. **Array creation**: GC allocation patterns

---

## Detailed Optimization History

See git history for detailed optimization phases (Phase 1-24).
