# luars User Guide

A comprehensive guide to embedding the luars Lua 5.5 runtime in your Rust application.

## Getting Started

| Document | Description |
|----------|-------------|
| [Getting Started](guide/01-GettingStarted.md) | Create a VM, load stdlib, run your first Lua script |
| [Executing Code](guide/02-ExecutingCode.md) | `execute`, `load`, `dofile`, `call_global`, return values |
| [Working with Values](guide/03-WorkingWithValues.md) | `LuaValue`, globals, tables, strings |
| [Rust Functions in Lua](guide/04-RustFunctions.md) | `CFunction`, `RClosure`, `create_closure` |
| [FromLua / IntoLua](guide/05-FromLuaIntoLua.md) | Automatic Rust ↔ Lua type conversion traits |
| [Error Handling](guide/06-ErrorHandling.md) | `LuaError`, `LuaFullError`, `pcall`, `xpcall`, `Result` |
| [API Reference](guide/07-APIReference.md) | Quick reference of all public methods |

## UserData (Custom Rust Types in Lua)

| Document | Description |
|----------|-------------|
| [Getting Started](userdata/GettingStarted.md) | 5-minute quickstart: define a Point, use it in Lua |
| [#\[derive(LuaUserData)\]](userdata/DeriveUserData.md) | Field exposure and attributes (`skip` / `readonly` / `name`) |
| [#\[lua_methods\]](userdata/LuaMethods.md) | Instance methods, constructors, `#[lua(skip)]` on methods |
| [register_type](userdata/RegisterType.md) | Type registration: `register_type` and `register_type_of` |
| [Type Conversions](userdata/TypeConversions.md) | Parameter types, return types, `Option` / `Result` handling |
| [Complete Examples](userdata/Examples.md) | End-to-end examples: Vec2, AppConfig, Calculator, and more |

## Async (Rust Async Functions in Lua)

| Document | Description |
|----------|-------------|
| [Getting Started](async/01-getting-started.md) | 5-minute async quickstart |
| [API Reference](async/02-api-reference.md) | All async types and methods |
| [Examples](async/03-examples.md) | Code examples from simple to complex |
| [Architecture](async/04-architecture.md) | Coroutine↔Future bridging internals |
| [Multi-VM Patterns](async/05-multi-vm.md) | Concurrent multi-VM design patterns |
| [HTTP Server Example](async/06-http-server.md) | Complete async HTTP server walkthrough |

## Other

| Document | Description |
|----------|-------------|
| [Differences from C Lua](Different.md) | All known behavioral differences between luars and C Lua 5.5 |

## Quick Example

```rust
use luars::{LuaVM, SafeOption, Stdlib};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create a VM
    let mut vm = LuaVM::new(SafeOption::default());

    // 2. Load the standard library
    vm.open_stdlib(Stdlib::All)?;

    // 3. Execute Lua code
    let results = vm.execute(r#"
        local sum = 0
        for i = 1, 100 do
            sum = sum + i
        end
        return sum
    "#)?;

    println!("Sum = {:?}", results[0].as_integer()); // Some(5050)
    Ok(())
}
```

## Typed-First Embedding Style

The public embedding API is now typed-first.

Prefer these methods in application code:

- `call` / `call1`
- `call_global` / `call1_global`
- `register_function_typed`
- `register_async_typed`
- `UserDataRef<T>` for typed userdata access inside Rust callbacks

Use the raw fallbacks only when you intentionally want to manipulate `LuaValue` vectors yourself:

- `call_raw`
- `call_global_raw`
- `register_function`
- `register_async`

Example:

```rust
use luars::{LuaUserData, LuaVM, SafeOption, Stdlib, UserDataRef, lua_methods};

#[derive(LuaUserData)]
struct Counter {
    pub count: i64,
}

#[lua_methods]
impl Counter {
    pub fn new(count: i64) -> Self {
        Self { count }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    vm.execute("function add(a, b) return a + b end")?;
    let sum: i64 = vm.call1_global("add", (10, 20))?;
    assert_eq!(sum, 30);

    vm.register_function_typed("bump", |mut counter: UserDataRef<Counter>, delta: i64| {
        let counter_ref = counter.get_mut().unwrap();
        counter_ref.count += delta;
        counter_ref.count
    })?;

    vm.register_async_typed("double_async", |n: i64| async move {
        Ok(n * 2)
    })?;

    Ok(())
}
```

## Dependencies

```toml
[dependencies]
luars = "0.12"

# With JSON support:
luars = { version = "0.12", features = ["serde"] }
```

The `#[derive(LuaUserData)]` and `#[lua_methods]` macros are re-exported by `luars` automatically — no need to add `luars-derive` separately.
