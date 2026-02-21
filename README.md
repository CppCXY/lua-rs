# luars

[![CI](https://github.com/CppCXY/lua-rs/workflows/CI/badge.svg)](https://github.com/CppCXY/lua-rs/actions)
[![Lua Test Suite](https://github.com/CppCXY/lua-rs/workflows/Lua%20Test%20Suite/badge.svg)](https://github.com/CppCXY/lua-rs/actions/workflows/lua_testes.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![crate](https://img.shields.io/crates/v/luars.svg?style=flat-square)](https://crates.io/crates/luars)

> **Note**: This is an experimental **Lua 5.5** interpreter crafted primarily through AI-assisted programming.

A Lua 5.5 interpreter written in pure Rust. Faithfully ported from the official C Lua source code architecture — register-based VM, incremental/generational GC, string interning — and **passes the official Lua 5.5 test suite** (`all.lua` — 28/30 test files, 435 unit tests).

## Highlights

- **Lua 5.5** — compiler, VM, and standard libraries implement the Lua 5.5 specification
- **Pure Rust** — no C dependencies, no `unsafe` FFI — the entire runtime is self-contained Rust
- **Official Test Suite** — passes 28 of 30 official Lua 5.5 test files (see [Compatibility](#compatibility))
- **Ergonomic Rust API** — `call_global`, `register_function`, `load`, `dofile`, `TableBuilder`, typed getters
- **UserData** — derive macros to expose Rust structs/enums to Lua with fields, methods, operators
- **Async** — run async Rust functions from Lua via transparent coroutine-based bridging

## Quick Start

```toml
[dependencies]
luars = "0.7"
```

```rust
use luars::{LuaVM, Stdlib, LuaValue};
use luars::lua_vm::SafeOption;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    // Execute Lua code
    let results = vm.execute("return 1 + 2")?;
    assert_eq!(results[0].as_integer(), Some(3));

    // Register a Rust function
    vm.register_function("greet", |state| {
        let name = state.get_arg(1).and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "World".into());
        let msg = state.create_string(&format!("Hello, {}!", name))?;
        state.push_value(msg)?;
        Ok(1)
    })?;

    vm.execute("print(greet('Rust'))")?;  // Hello, Rust!
    Ok(())
}
```

## Embedding API Overview

### Execution

```rust
vm.execute("return 42")?;                          // compile & run a string
vm.dofile("scripts/init.lua")?;                    // compile & run a file
let f = vm.load("return 1+1")?;                    // compile without running
let results = vm.call(f, vec![])?;                 // call a function value
let results = vm.call_global("func", vec![])?;     // look up global & call
```

### Globals & Types

```rust
vm.set_global("x", LuaValue::integer(42))?;
let x: i64 = vm.get_global_as::<i64>("x")?.unwrap();

vm.register_function("add", |s| { /* ... */ Ok(1) })?;
vm.register_type_of::<Point>("Point")?;
vm.register_enum::<Color>("Color")?;
```

### Tables

```rust
use luars::TableBuilder;

let config = TableBuilder::new()
    .set("host", vm.create_string("localhost")?)
    .set("port", LuaValue::integer(8080))
    .push(LuaValue::integer(1))           // array part: t[1] = 1
    .build(&mut vm)?;
vm.set_global("config", config)?;

// Iterate
for (k, v) in vm.table_pairs(&config)? {
    println!("{:?} = {:?}", k, v);
}
```

### Error Handling

```rust
match vm.execute("error('boom')") {
    Ok(_) => {}
    Err(e) => {
        // Option 1: lightweight (1-byte enum)
        let msg = vm.get_error_message(e);

        // Option 2: rich error (implements std::error::Error)
        let full = vm.into_full_error(e);
        eprintln!("{}", full);  // prints full message with source location
    }
}
```

### Async

```rust
vm.register_async("fetch", |args| async move {
    let url = args[0].as_str().unwrap_or("").to_string();
    let body = reqwest::get(&url).await?.text().await?;
    Ok(vec![AsyncReturnValue::string(body)])
})?;

let results = vm.execute_async("return fetch('https://example.com')").await?;
```

### UserData

```rust
use luars::{LuaUserData, lua_methods};

#[derive(LuaUserData)]
#[lua_impl(Display)]
struct Point { pub x: f64, pub y: f64 }

#[lua_methods]
impl Point {
    pub fn new(x: f64, y: f64) -> Self { Point { x, y } }
    pub fn distance(&self) -> f64 { (self.x * self.x + self.y * self.y).sqrt() }
}

vm.register_type_of::<Point>("Point")?;
```

```lua
local p = Point.new(3, 4)
print(p.x, p:distance())   -- 3.0  5.0
```

## Documentation

| Document | Description |
|----------|-------------|
| **[Guide](docs/Guide.md)** | Embedding guide — VM, execution, values, functions, errors, API reference |
| **[UserData Guide](docs/UserGuide.md)** | Derive macros, fields, methods, constructors, type conversion |
| **[Async Guide](docs/Async.md)** | Async Rust functions in Lua, architecture, HTTP server example |
| **[Differences](docs/Different.md)** | All known behavioral differences from C Lua 5.5 |

## Architecture

```
luars (library crate)
├── compiler/        — Lexer, parser, code generator (from lparser.c / lcode.c)
├── lua_vm/          — Register-based VM, LuaState, call stack, upvalues
│   ├── execute/     — Bytecode dispatch loop
│   ├── async_thread — Coroutine ↔ Future bridging
│   └── table_builder — Fluent table construction API
├── gc/              — Tri-color incremental + generational mark-and-sweep GC
├── lua_value/       — Tagged values, strings, tables, functions, userdata
│   └── lua_table/   — Hash + array hybrid table (from ltable.c)
├── stdlib/          — Complete standard library (basic, string, table, math, io, os, …)
└── serde/           — Optional Lua ↔ JSON (feature: serde)

luars_interpreter (binary crate)
├── lua              — CLI interpreter (-e, -i, -l, -v, …)
└── bytecode_dump    — Bytecode disassembler

luars_wasm (library crate)
└── WASM bindings    — Run Lua in the browser via wasm-bindgen
```

## Garbage Collector

Ported from Lua 5.5's `lgc.c`. Three collection modes:

| Mode | Description |
|------|-------------|
| **Incremental** | Tri-color mark-and-sweep, interleaved with program execution |
| **Generational Minor** | Collects young objects only, promoting survivors |
| **Generational Major** | Full collection when minor cycles are insufficient |

Features: object aging (NEW → SURVIVAL → OLD), weak table / ephemeron cleanup, `__gc` finalizers with resurrection, configurable pause / step / minor multiplier.

## Compatibility

Passes the official Lua 5.5 test suite (`lua_tests/testes/all.lua`):

| Test File | Status | | Test File | Status |
|-----------|--------|-|-----------|--------|
| gc.lua | ✅ | | pm.lua | ✅ |
| calls.lua | ✅ | | utf8.lua | ✅ |
| strings.lua | ✅ | | api.lua | ✅ \* |
| literals.lua | ✅ | | memerr.lua | ✅ \* |
| tpack.lua | ✅ | | events.lua | ✅ |
| attrib.lua | ✅ | | vararg.lua | ✅ |
| gengc.lua | ✅ | | closure.lua | ✅ |
| locals.lua | ✅ | | coroutine.lua | ✅ \* |
| constructs.lua | ✅ | | goto.lua | ✅ |
| code.lua | ✅ | | errors.lua | ✅ \* |
| big.lua | ✅ | | math.lua | ✅ |
| cstack.lua | ✅ | | sort.lua | ✅ |
| nextvar.lua | ✅ | | bitwise.lua | ✅ |
| verybig.lua | ✅ | | files.lua | ✅ |
| main.lua | ⏭️ | | db.lua | ⏭️ |

\* Some C-API-dependent test sections are skipped (no `testC` library).

**Skipped:** `main.lua` (interactive CLI), `db.lua` (debug hooks not yet implemented).

For the full list of behavioral differences, see [docs/Different.md](docs/Different.md).

### Key Differences from C Lua

- **No C API / C module loading** — pure Rust, no `lua_State*` interface
- **No string-to-number coercion in arithmetic** — `"3" + 1` raises an error
- **No debug hooks** — `debug.sethook` is a stub; `getinfo` / `getlocal` / `traceback` work
- **Own bytecode format** — `string.dump` output is not compatible with C Lua
- **UTF-8 strings** — no arbitrary binary bytes (use the separate `binary` type)
- **Deterministic `#t`** — length operator uses array lenhint, not hash-part search

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
./target/release/lua script.lua           # run a script
./target/release/lua -i                   # interactive REPL
./target/release/lua -e "print('hello')"  # inline code
./target/release/bytecode_dump script.lua # disassemble
```

### Run the Official Test Suite

```bash
# Windows
.\run_lua_tests.ps1

# Linux / macOS
cd lua_tests/testes && ../../target/release/lua all.lua
```

### Run Benchmarks

```bash
# Windows                    # Linux / macOS
.\run_benchmarks.ps1          ./run_benchmarks.sh
```

16 benchmark files covering arithmetic, closures, control flow, coroutines, error handling, functions, iterators, locals, math, metatables, multi-return, OOP, strings, string library, tables, and table library.

## Optional Features

| Feature | Description |
|---------|-------------|
| `serde` | Lua ↔ JSON serialization via `serde` / `serde_json` |

```bash
cargo build --release --features serde
```

## Contributing

Contributions are welcome. Please open issues for bugs, performance observations, or semantic deviations from Lua 5.5.

## License

MIT — see [LICENSE](LICENSE).

## Acknowledgments

- [Lua 5.5](https://www.lua.org/) — language design and reference implementation
- The Lua team (PUC-Rio) for the test suite
