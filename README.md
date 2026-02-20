# luars

[![CI](https://github.com/CppCXY/lua-rs/workflows/CI/badge.svg)](https://github.com/CppCXY/lua-rs/actions)
[![Lua Test Suite](https://github.com/CppCXY/lua-rs/workflows/Lua%20Test%20Suite/badge.svg)](https://github.com/CppCXY/lua-rs/actions/workflows/lua_testes.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![crate](https://img.shields.io/crates/v/luars.svg?style=flat-square)](https://crates.io/crates/luars)

> ⚠️ **Project Notice**: This is an experimental **Lua 5.5** interpreter implementation crafted primarily through AI-assisted programming.

A Lua 5.5 interpreter written in pure Rust (~49,000 lines). Faithfully ported from the official C Lua source code architecture — register-based VM, incremental/generational GC, string interning — and **passes the official Lua 5.5 test suite** (`all.lua` — 28/30 test files).

## Highlights

- **Lua 5.5**: Compiler, VM, and standard libraries implement the Lua 5.5 specification
- **Pure Rust**: No C dependencies, no `unsafe` FFI — the entire runtime is self-contained Rust
- **Official Test Suite**: Passes 28 of 30 official Lua 5.5 test files — `all.lua` runs to `final OK` (see [Compatibility](#compatibility))
- **UserData API**: Derive macros to expose Rust structs to Lua with fields, methods, and constructors (see [User Guide](docs/UserGuide.md))
- **~49K lines of Rust** across compiler, VM, GC, and standard libraries

## Architecture

```
luars (library crate)
├── compiler/        — Lexer, parser, code generator (from lparser.c / lcode.c)
├── lua_vm/          — Register-based VM, LuaState, call stack, upvalues
│   └── execute/     — Bytecode dispatch (11 files)
├── gc/              — Tri-color incremental + generational mark-and-sweep GC
├── lua_value/       — Value types (nil, bool, int, float, string, table, function, …)
│   └── lua_table/   — Hash + array hybrid table implementation
├── stdlib/          — Complete standard library
│   ├── basic/       — print, type, pcall, load, collectgarbage, …
│   ├── string/      — Pattern matching, format, pack/unpack, dump
│   ├── math.rs      — Full math library
│   ├── table.rs     — insert, remove, move, concat, sort, pack, unpack
│   ├── io/          — File I/O with streams
│   ├── os.rs        — clock, date, execute, getenv, …
│   ├── coroutine.rs — create, resume, yield, wrap, close
│   ├── debug.rs     — getinfo, getlocal, traceback, …
│   ├── utf8.rs      — UTF-8 library
│   └── package.rs   — require, searchers, module loading
└── serde/           — Optional Lua ↔ JSON serialization (feature: serde)

luars_interpreter (binary crate)
├── lua              — Full CLI interpreter (-e, -i, -l, -v, …)
└── bytecode_dump    — Bytecode disassembler for debugging

luars_wasm (library crate)
└── WASM bindings    — Run Lua in the browser via wasm-bindgen
```

## Garbage Collector

Ported from Lua 5.5's `lgc.c`. Supports three collection modes:

| Mode | Description |
|------|-------------|
| **Incremental** (`KGC_INC`) | Tri-color mark-and-sweep, interleaved with program execution |
| **Generational Minor** (`KGC_GENMINOR`) | Collects young objects only, promoting survivors |
| **Generational Major** (`KGC_GENMAJOR`) | Full collection when minor cycles are insufficient |

Features: object aging (NEW → SURVIVAL → OLD), weak table / ephemeron cleanup, `__gc` finalizers with resurrection, configurable pause / step multiplier / minor multiplier.

## Compatibility

Passes the official Lua 5.5 test suite (`lua_tests/testes/all.lua`):

| Test File | Status | | Test File | Status |
|-----------|--------|-|-----------|--------|
| gc.lua | ✅ | | pm.lua | ✅ |
| calls.lua | ✅ | | utf8.lua | ✅ |
| strings.lua | ✅ | | api.lua | ✅ * |
| literals.lua | ✅ | | memerr.lua | ✅ * |
| tpack.lua | ✅ | | events.lua | ✅ |
| attrib.lua | ✅ | | vararg.lua | ✅ |
| gengc.lua | ✅ | | closure.lua | ✅ |
| locals.lua | ✅ | | coroutine.lua | ✅ * |
| constructs.lua | ✅ | | goto.lua | ✅ |
| code.lua | ✅ | | errors.lua | ✅ * |
| big.lua | ✅ | | math.lua | ✅ |
| cstack.lua | ✅ | | sort.lua | ✅ |
| nextvar.lua | ✅ | | bitwise.lua | ✅ |
| verybig.lua | ✅ | | files.lua | ✅ |
| main.lua | ⏭️ | | db.lua | ⏭️ |

\* Some C-API-dependent test sections are skipped (no `testC` library).

**Skipped tests:** `main.lua` (interactive CLI tests), `db.lua` (debug hooks not yet implemented).

For a full list of behavioral differences, see [docs/Different.md](docs/Different.md).

### Key Differences from C Lua

- **No C API / C module loading** — pure Rust, no `lua_State*` C interface
- **No string-to-number coercion in arithmetic** — `"3" + 1` raises an error
- **No debug hooks** — `debug.sethook` is a stub; `debug.getinfo` / `debug.getlocal` / `debug.traceback` work
- **Own bytecode format** — `string.dump` output is not compatible with C Lua
- **UTF-8 strings** — strings are UTF-8 encoded; no arbitrary binary bytes (use the separate `binary` type)
- **Deterministic `#t`** — length operator uses array lenhint, no hash-part search

## UserData API — Exposing Rust Types to Lua

luars provides derive macros to expose Rust structs to Lua with automatic field access, methods, and constructors:

```rust
#[derive(LuaUserData)]
#[lua_impl(Display)]
struct Point { pub x: f64, pub y: f64 }

#[lua_methods]
impl Point {
    pub fn new(x: f64, y: f64) -> Self { Point { x, y } }
    pub fn distance(&self) -> f64 { (self.x * self.x + self.y * self.y).sqrt() }
}

// Register and use from Lua:
state.register_type_of::<Point>("Point")?;
```

```lua
local p = Point.new(3, 4)
print(p.x, p:distance())   -- 3.0  5.0
```

**Full documentation:** [docs/Guide.md](docs/Guide.md) — covers VM creation, executing code, values, Rust functions, UserData, error handling, and API reference.

## Building

### Prerequisites

- Rust 1.93+ (edition 2024)

### Build

```bash
cargo build --release
```

Produces two binaries in `target/release/`:
- `lua` — the interpreter
- `bytecode_dump` — bytecode disassembler

### Usage

```bash
# Run a script
./target/release/lua script.lua

# Interactive REPL
./target/release/lua -i

# Execute inline code
./target/release/lua -e "print('hello')"

# Disassemble bytecode
./target/release/bytecode_dump script.lua
```

### Run the Official Lua 5.5 Test Suite

```bash
# Windows
.\run_lua_tests.ps1

# Linux / macOS
cd lua_tests/testes
../../target/release/lua all.lua
```

Runs the complete official Lua 5.5 test suite (`all.lua`). All 28 test files pass to `final OK` on Windows, Linux, and macOS. See [Lua Test Suite CI](https://github.com/CppCXY/lua-rs/actions/workflows/lua_testes.yml) for cross-platform results.

### Run Benchmarks

```bash
# Windows
.\run_benchmarks.ps1

# Linux / macOS
./run_benchmarks.sh
```

16 benchmark files covering arithmetic, closures, control flow, coroutines, error handling, functions, iterators, locals, math, metatables, multi-return, OOP, strings, string library, tables, and table library.

## Optional Features

| Feature | Description |
|---------|-------------|
| `serde` | Enables Lua ↔ JSON serialization via `serde` / `serde_json` |

```bash
cargo build --release --features serde
```

## Project Structure

```
lua_rt/
├── crates/
│   ├── luars/               — Core library (compiler, VM, GC, stdlib)
│   ├── luars_interpreter/   — CLI binaries (lua, bytecode_dump)
│   └── luars_wasm/          — WebAssembly bindings + demo pages
├── lua_tests/testes/        — Official Lua 5.5 test suite
├── benchmarks/              — Performance benchmarks (16 files)
├── bytecode_comparison_output/ — Compiler correctness: our vs official bytecode
├── docs/
│   ├── Guide.md             — Comprehensive usage guide (start here)
│   │   └── guide/           — Guide sub-docs (7 files)
│   ├── Different.md         — Full behavioral differences documentation
│   └── UserGuide.md         — UserData API guide
│       └── userdata/        — Detailed sub-docs (6 files)
└── libs/                    — Test helper modules
```

## Contributing

Contributions are welcome. Please open issues for bugs, performance observations, or semantic deviations from Lua 5.5.

## License

MIT — see [LICENSE](LICENSE).

## Acknowledgments

- [Lua 5.5](https://www.lua.org/) — language design and reference implementation
- The Lua team (PUC-Rio) for the test suite