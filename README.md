# luars

[![CI](https://github.com/CppCXY/lua-rs/workflows/CI/badge.svg)](https://github.com/CppCXY/lua-rs/actions)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> ⚠️ **Project Notice**: This is an experimental **Lua 5.5** interpreter implementation crafted primarily through AI-assisted programming. It explores the intersection of modern AI coding capabilities, the Rust programming language, and low-level VM architecture.

A robust Lua 5.5 interpreter implementation in Rust. This project aims to strictly adhere to Lua 5.5 semantics while leveraging Rust's safety and performance characteristics.

## 🌟 Key Highlights

- **Lua 5.5 Semantics**: Implements the latest Lua 5.5 language specification.
- **100% Test Pass Rate**: Currently passing **302 out of 302** tests in the test suite.
- **Pure Rust**: Core VM and standard libraries implemented entirely in Rust.

## 🚀 Performance

**Overall**: Comparable to native Lua (30-100% speed) across various workloads.

[![Benchmarks](https://github.com/CppCXY/lua-rs/actions/workflows/benchmarks.yml/badge.svg)](https://github.com/CppCXY/lua-rs/actions/workflows/benchmarks.yml)

## ✨ Features

### Core Language Features
- **Complete Lua 5.5 Syntax**: Full support for all Lua 5.5 language constructs
  - Operators: arithmetic, bitwise, logical, relational, concatenation, and length
  - Control flow: `if/elseif/else`, `while`, `repeat/until`, numeric/generic `for`, `goto`/labels
  - Functions: closures, variadic arguments (`...`), multiple return values
  - Tables: comprehensive table constructor syntax with list/record/general forms
  
- **Advanced Features**:
  - **Metatables & Metamethods**: Full metamethod support including `__gc`, `__close`, `__index`, `__newindex`, arithmetic/bitwise/comparison operators
  - **Coroutines**: Complete coroutine API with `create`, `resume`, `yield`, `status`, `wrap`
  - **Upvalues**: Proper upvalue management with open/closed states and to-be-closed variables
  - **Weak Tables**: Full weak reference support with weak keys (`k`), weak values (`v`), and ephemeron tables (`kv`)
  - **Finalizers**: `__gc` metamethod with proper resurrection semantics

- **Garbage Collection**:
  - Tri-color incremental mark-and-sweep GC
  - Generational mode support
  - Full finalizer execution (`__gc` metamethod)
  - Weak table cleanup with proper ephemeron semantics
  - Configurable GC parameters (pause, stepmul, minor multiplier)

### Standard Library Coverage
- ✅ **basic**: `print`, `type`, `tonumber`, `tostring`, `pairs`, `ipairs`, `next`, `rawget`, `rawset`, `rawlen`, `rawequal`, `select`, `getmetatable`, `setmetatable`, `pcall`, `xpcall`, `error`, `assert`, `collectgarbage`, `load`, `dofile`, `loadfile`
- ✅ **string**: Pattern matching, `find`, `match`, `gmatch`, `gsub`, `format`, `pack`, `unpack`, `dump`, `len`, `sub`, `byte`, `char`, `rep`, `reverse`, `upper`, `lower`
- ✅ **table**: `insert`, `remove`, `move`, `concat`, `sort`, `pack`, `unpack`
- ✅ **math**: Complete math library including `random`, `randomseed`, `abs`, `ceil`, `floor`, `min`, `max`, `sqrt`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `deg`, `rad`, `exp`, `log`, `modf`, `fmod`, `tointeger`, `type`, `ult`, bitwise operations
- ✅ **io**: File I/O operations, `open`, `close`, `read`, `write`, `lines`, `flush`, `seek`, `setvbuf`, `input`, `output`, `popen`, `tmpfile`
- ✅ **os**: `clock`, `date`, `difftime`, `execute`, `exit`, `getenv`, `remove`, `rename`, `setlocale`, `time`, `tmpname`
- ✅ **coroutine**: `create`, `resume`, `yield`, `status`, `running`, `wrap`, `isyieldable`, `close`
- ✅ **utf8**: `char`, `codes`, `codepoint`, `len`, `offset`, `charpattern`
- ⚠️ **debug**: Partial implementation (missing some introspection features)
- ✅ **package**: Module loading system with `require`, `searchers`, `preload`, `loaded`, `path`, `cpath`

## 📦 Building & Running

### Prerequisites
- Rust (latest stable)

### Build

```bash
cargo build --release
```

### Run Tests
```bash
cargo test
```

### Run Benchmarks
```bash
# Windows
.\run_benchmarks.ps1

# Linux/macOS
./run_benchmarks.sh
```

### Usage
```bash
# Execute a script
./target/release/lua script.lua

# Interactive mode
./target/release/lua -i

# Inspect bytecode
./target/release/bytecode_dump script.lua
```

## 🤝 Contributing

Contributions are welcome! Please feel free to open issues for bugs, performance observations, or semantics that deviate from Lua 5.5.

## 📜 License

MIT License - See [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- **Lua 5.5**: For the language design and reference manual.