# luars

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)
[![Crates.io](https://img.shields.io/crates/v/luars.svg)](https://crates.io/crates/luars)

luars is an embeddable pure Rust Lua 5.5 runtime crate. It includes the compiler, VM, GC, standard library, and a high-level host API built around `Lua`.

If you want the repository-level overview, including examples and companion crates, start with [../../README.md](../../README.md). This README focuses on the crate surface that application code should use directly.

## Installation

```toml
[dependencies]
luars = "0.18"
```

Optional features:

```toml
[dependencies]
luars = { version = "0.18", features = ["serde", "sandbox"] }
```

## Quick Start

```rust
use luars::{Lua, SafeOption, Stdlib};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut lua = Lua::new(SafeOption::default());
    lua.load_stdlibs(Stdlib::All)?;

    lua.register_function("add", |a: i64, b: i64| a + b)?;
    let sum: i64 = lua.load("return add(1, 2)").eval()?;

    assert_eq!(sum, 3);
    Ok(())
}
```

## High-Level Workflow

### Execute code

```rust
lua.load("x = 40 + 2").exec()?;
let answer: i64 = lua.load("return x").eval()?;
let pair: (i64, i64) = lua.load("return 20, 22").eval_multi()?;
```

### Call Lua globals

```rust
lua.load(
    r#"
    function greet(name)
        return "hello, " .. name
    end
    "#,
)
.exec()?;

let text: String = lua.call_global1("greet", "luars")?;
```

### Register Rust functions

```rust
lua.register_function("slugify", |value: String| {
    value.trim().to_lowercase().replace(' ', "-")
})?;
```

### Exchange tables and globals

```rust
let config = lua.create_table_from([("host", "127.0.0.1"), ("mode", "dev")])?;
lua.globals().set("config", config)?;

let globals = lua.globals();
let host: String = globals.get("config")?.get("host")?;
assert_eq!(host, "127.0.0.1");
```

### Expose Rust types as userdata

```rust
use luars::{LuaUserData, lua_methods};

#[derive(LuaUserData)]
struct Counter {
    pub value: i64,
}

#[lua_methods]
impl Counter {
    pub fn new(value: i64) -> Self {
        Self { value }
    }

    pub fn inc(&mut self, delta: i64) {
        self.value += delta;
    }

    pub fn get(&self) -> i64 {
        self.value
    }
}

lua.register_type::<Counter>("Counter")?;
let count: i64 = lua
    .load(
        r#"
        local counter = Counter.new(1)
        counter:inc(41)
        return counter:get()
        "#,
    )
    .eval()?;

assert_eq!(count, 42);
```

### Use scoped borrowed values

```rust
let result: String = lua.scope(|scope| {
    let prefix = String::from("user:");
    let format_name = scope.create_function_with(&prefix, |prefix: &String, value: String| {
        format!("{prefix}{value}")
    })?;

    scope.globals().set("format_name", &format_name)?;
    scope.load("return format_name('alice')").eval()
})?;

assert_eq!(result, "user:alice");
```

## Async And Sandbox

The high-level API supports both typed async callbacks and async execution entry points:

```rust
lua.register_async_function("double_async", |value: i64| async move {
    Ok(value * 2)
})?;

let result: i64 = lua.load("return double_async(21)").eval_async().await?;
assert_eq!(result, 42);
```

You can also drive existing Lua functions asynchronously with `call_async()`, `call_async1()`, `call_async_global()`, and `call_async_global1()`.

When the `sandbox` feature is enabled, the same high-level surface exposes isolated execution helpers:

```rust
use luars::SandboxConfig;

let mut sandbox = SandboxConfig::default();
lua.sandbox_insert_global(&mut sandbox, "answer", 42_i64)?;

let answer: i64 = lua.eval_sandboxed("return answer", &sandbox)?;
assert_eq!(answer, 42);
```

For more detail, see [../../docs/Async.md](../../docs/Async.md).

## Feature Flags

| Feature | Description |
|---------|-------------|
| `serde` | Enable serde-based conversions |
| `sandbox` | Enable sandbox execution helpers |
| `shared-proto` | Enable shared prototypes for multi-VM scenarios |

## Known Boundaries

- No C API and no direct loading of native C Lua modules
- `string.dump` produces luars-specific bytecode
- Some corners of `debug`, `io`, and `package` still differ from the official C Lua implementation

See [../../docs/Different.md](../../docs/Different.md) for the detailed compatibility notes.

## Documentation

| Document | Description |
|----------|-------------|
| [../../docs/Guide.md](../../docs/Guide.md) | High-level embedding guide |
| [../../docs/UserGuide.md](../../docs/UserGuide.md) | High-level userdata guide |
| [../../docs/Async.md](../../docs/Async.md) | High-level async and sandbox guide |
| [../../docs/Different.md](../../docs/Different.md) | Known differences from C Lua |

## License

MIT. See [../../LICENSE](../../LICENSE).
