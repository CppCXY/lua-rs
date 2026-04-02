# luars

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![crate](https://img.shields.io/crates/v/luars.svg?style=flat-square)](https://crates.io/crates/luars)

luars is a pure Rust Lua 5.5 runtime and embedding toolkit. This repository contains the core library, derive macros, the interpreter, debugger integration, a WASM target, and several host-facing examples.

The repository-level documentation is intentionally focused on the high-level `Lua` API. Lower-level `LuaVM` APIs still exist, but the default examples and guides now use the high-level surface first.

## Quick Start

```toml
[dependencies]
luars = "0.17"
```

```rust
use luars::{Lua, SafeOption, Stdlib};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut lua = Lua::new(SafeOption::default());
    lua.load_stdlibs(Stdlib::All)?;

    lua.register_function("add", |a: i64, b: i64| a + b)?;
    let sum: i64 = lua.load("return add(20, 22)").eval()?;

    assert_eq!(sum, 42);
    Ok(())
}
```

## What The High-Level API Covers

- Execute chunks with `lua.load(...).exec()`, `eval()`, and `eval_multi()`
- Execute async chunks with `exec_async()`, `eval_async()`, and `eval_multi_async()`
- Call Lua globals with `call_global()` / `call_global1()` and their async variants
- Register Rust functions with `register_function()` and `register_async_function()`
- Expose Rust types with `register_type()` and `LuaUserData`
- Work with globals and tables through `globals()`, `create_table()`, and `create_table_from()`
- Create scoped borrowed callbacks and userdata through `scope(...)`
- Run isolated chunks through `load_sandboxed()` and `execute_sandboxed()` when the `sandbox` feature is enabled

## Repository Layout

| Path | Description |
|------|-------------|
| `crates/luars` | Core library with the compiler, VM, GC, and high-level `Lua` API |
| `crates/luars-derive` | `LuaUserData` and `lua_methods` macros |
| `crates/luars_interpreter` | CLI interpreter and bytecode tools |
| `crates/luars_debugger` | Debugger integration |
| `crates/luars_wasm` | WASM bindings |
| `docs/` | High-level embedding guides |
| `examples/` | Host examples built around the high-level API |

## Examples

| Example | Description |
|---------|-------------|
| [examples/luars-example](examples/luars-example) | Minimal high-level API example: globals, userdata, and scope |
| [examples/rules-engine-demo](examples/rules-engine-demo) | Business rules engine with Rust host functions and Lua policy |
| [examples/http-server](examples/http-server) | Async HTTP example using high-level async calls and sandboxed Lua request handlers |
| [examples/rust-bind-bench](examples/rust-bind-bench) | High-level userdata registration benchmark |

## Documentation

| Document | Description |
|----------|-------------|
| [docs/Guide.md](docs/Guide.md) | High-level `Lua` API overview |
| [docs/UserGuide.md](docs/UserGuide.md) | High-level userdata guide |
| [docs/Async.md](docs/Async.md) | High-level async and sandbox status |
| [docs/Different.md](docs/Different.md) | Known differences from C Lua |
| [crates/luars/README.md](crates/luars/README.md) | Crate-level documentation |

## Validate

```bash
cargo test
```

## License

MIT. See [LICENSE](LICENSE).
