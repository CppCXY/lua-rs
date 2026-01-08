# lua_rt

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

### Supported Language Features
- **Full Operator Set**: Arithmetic, bitwise, logical, and length operators.
- **Control Structures**: `if`, `while`, `repeat`, `for` (numeric/generic), `goto` with label support.
- **Values & Types**: Full support for basic types including integers, floats, strings, tables, closures, and userdatas.
- **Advanced Lua**:
  - Full **Metatable** & **Metamethod** support.
  - **Coroutines** (symmetric/semifunctions).
  - **Closures** with complex upvalue management (open/closed).
  - **Variadic arguments** (`...`) and functions.

### Standard Libraries implementation
- **Basic**: `print`, `type`, `pairs`, `ipairs`, `getmetatable`, `setmetatable`, etc.
- **String**: Pattern matching, formatting, binary packing/unpacking.
- **Table**: Manipulation, sorting, moving, concatenation.
- **Math**: Full mathematical suite including bitwise operations.
- **IO & OS**: File system operations and system interaction.
- **Coroutine**: Full coroutine management.
- **UTF-8**: Proper UTF-8 string support.


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

## 📄 Examples

**Coroutines**
```lua
local co = coroutine.create(function()
    for i = 1, 3 do
        print("yield", i)
        coroutine.yield()
    end
end)

while coroutine.status(co) ~= "dead" do
    coroutine.resume(co)
end
```

**Metatables**
```lua
local vec = {x = 10, y = 20}
setmetatable(vec, {
    __add = function(a, b)
        return {x = a.x + b.x, y = a.y + b.y}
    end
})
local v2 = vec + vec -- {x=20, y=40}
```

## 🤝 Contributing

Contributions are welcome! Please feel free to open issues for bugs, performance observations, or semantics that deviate from Lua 5.5.

## 📜 License

MIT License - See [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- **Lua 5.5**: For the language design and reference manual.