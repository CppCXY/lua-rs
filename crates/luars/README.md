# luars

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)
[![Crates.io](https://img.shields.io/crates/v/luars.svg)](https://crates.io/crates/luars)

luars is an embeddable pure Rust Lua 5.5 runtime crate. It provides the compiler, bytecode VM, garbage collector, standard library, and a typed-first host API.

If you want the repository-level view, including the CLI, WASM target, and example entry points, start with [README.md](../../README.md). This document focuses on the `luars` crate itself.

## Good Fit For

- Embedding Lua as a scripting layer inside a Rust application
- Moving rules, orchestration, or plugin logic into Lua
- Exposing Rust functions, tables, and custom types to Lua
- Running async callbacks, multi-VM setups, or sandboxed execution

## Features

- Lua 5.5 compiler, register-based VM, and GC
- Typed-first API: `call`, `call1`, `call_global`, `call1_global`
- Raw fallback APIs when you need direct `LuaValue` control
- `LuaUserData` and `lua_methods` macros for exposing Rust types
- Typed registration APIs such as `register_function_typed` and `register_async_typed`
- Byte-string Lua string semantics, plus an optional UTF-8 view on the Rust side
- `TableBuilder`, typed getters, and `FromLua` / `IntoLua` conversions
- Optional `serde`, `sandbox`, and `shared-proto` features

## Installation

```toml
[dependencies]
luars = "0.17"
```

Optional features:

```toml
[dependencies]
luars = { version = "0.17", features = ["serde", "sandbox"] }
```

## Basic Example

```rust
use luars::{LuaVM, Stdlib};
use luars::lua_vm::SafeOption;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    vm.execute("function add(a, b) return a + b end")?;
    let sum: i64 = vm.call1_global("add", (1, 2))?;

    assert_eq!(sum, 3);
    Ok(())
}
```

## Host API Overview

### Execute And Call

```rust
let results = vm.execute("return 42")?;
let chunk = vm.load("return 1 + 1")?;
let value: i64 = vm.call1(chunk, ())?;
let pair: (i64, i64) = vm.call_global("divmod", (9, 4))?;

let raw = vm.call_global_raw("legacy_func", vec![])?;
```

### Register Rust Functions

Most host code should prefer the typed APIs:

```rust
vm.register_function_typed("add", |a: i64, b: i64| a + b)?;
vm.register_function_typed("greet", |name: String| format!("hello, {name}"))?;
```

Drop to the raw API only when you intentionally need direct stack and value access:

```rust
vm.register_function("greet_raw", |state| {
    let name = state
        .get_arg(1)
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "world".to_string());

    state.push_value(state.create_string(&format!("hello, {name}"))?)?;
    Ok(1)
})?;
```

### Lua Strings And Raw Bytes

At the Lua level, strings still follow standard byte-string semantics. On the Rust side, you can choose a text view or exact raw bytes as needed:

```rust
let text = vm.create_string("hello")?;
assert_eq!(text.as_str(), Some("hello"));

let raw = vm.create_bytes(&[0xff, 0x00, b'A'])?;
assert_eq!(raw.as_str(), None);
assert_eq!(raw.as_bytes(), Some(&[0xff, 0x00, b'A'][..]));
```

### Tables And Globals

```rust
use luars::{LuaValue, TableBuilder};

let config = TableBuilder::new()
    .set("host", vm.create_string("localhost")?)
    .set("port", LuaValue::integer(8080))
    .push(LuaValue::integer(1))
    .build(&mut vm)?;

vm.set_global("config", config)?;
```

### UserData

```rust
use luars::{LuaUserData, lua_methods};

#[derive(LuaUserData)]
#[lua_impl(Display)]
struct Point {
    pub x: f64,
    pub y: f64,
}

#[lua_methods]
impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

vm.register_type_of::<Point>("Point")?;
```

### Async

```rust
vm.register_async_typed("fetch_len", |url: String| async move {
    let body = reqwest::get(&url).await?.text().await?;
    Ok(body.len() as i64)
})?;

let results = vm.execute_async("return fetch_len('https://example.com')").await?;
```

### Error Handling

```rust
match vm.execute("error('boom')") {
    Ok(_) => {}
    Err(err) => {
        let msg = vm.get_error_message(err);
        let full = vm.into_full_error(err);
        eprintln!("{msg}");
        eprintln!("{full}");
    }
}
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `serde` | Enable conversions between `LuaValue` and serde / JSON data |
| `sandbox` | Enable sandbox execution APIs for environment isolation, capability injection, and timeout/instruction/memory limits |
| `shared-proto` | Enable shared function prototypes for multi-VM scenarios |

## Known Boundaries

- No C API, and no direct loading of native C Lua modules
- `string.dump` produces luars-specific bytecode
- On the Rust side, `as_str()` only returns `Some(&str)` when the underlying bytes are valid UTF-8
- Some corners of `debug`, `io`, and `package` still differ from the official C Lua implementation

See [../../docs/Different.md](../../docs/Different.md) for the full list of differences.

## Documentation

| Document | Description |
|----------|-------------|
| [../../docs/Guide.md](../../docs/Guide.md) | Core embedding guide |
| [../../docs/UserGuide.md](../../docs/UserGuide.md) | UserData and type exposure |
| [../../docs/Async.md](../../docs/Async.md) | Async documentation and multi-VM patterns |
| [../../docs/Different.md](../../docs/Different.md) | Known differences from C Lua |

## License

MIT. See [../../LICENSE](../../LICENSE).
