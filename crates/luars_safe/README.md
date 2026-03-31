# luars_safe

luars_safe is a safe, embedding-oriented wrapper crate built on top of [luars](../luars/README.md).

It keeps the original low-level `luars` API untouched, but gives application code a narrower surface that prefers Rust types, table and function handles, and common host-side workflows over direct `LuaValue` plumbing.

## Positioning

- Use `luars` when you are building low-level integrations, custom bridges, debuggers, raw value transforms, or other infrastructure that needs `LuaVM`, `LuaState`, and `LuaValue` directly.
- Use `luars_safe` when you are embedding Lua into an application and want the default API to be typed and harder to misuse.

`luars_safe` is intentionally a wrapper, not a replacement. The low-level crate remains the source of truth.

## Installation

```toml
[dependencies]
luars_safe = { version = "0.17" }
```

Optional features are forwarded to `luars`:

```toml
[dependencies]
luars_safe = {  version = "0.17", features = ["serde", "sandbox"] }
```

## Quick Start

```rust
use luars_safe::{Lua, SafeOption, Stdlib};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut lua = Lua::new(SafeOption::default());
    lua.open_stdlib(Stdlib::All)?;

    lua.set_global("name", "Lua")?;
    lua.register_function("sum", |a: i64, b: i64| a + b)?;

    let greeting: String = lua.eval("return 'hello ' .. name")?;
    let total: i64 = lua.eval("return sum(20, 22)")?;

    assert_eq!(greeting, "hello Lua");
    assert_eq!(total, 42);
    Ok(())
}
```

## Safe Table API

For table-heavy host code, prefer `Table` handles or the safe `TableBuilder`.

```rust
use luars_safe::{Lua, SafeOption, TableBuilder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut lua = Lua::new(SafeOption::default());

    let config = lua.build_table(
        TableBuilder::new()
            .set("host", "localhost")
            .set("port", 8080_i64)
            .push("alpha")
            .push("beta"),
    )?;

    lua.set_global_table("config", &config)?;

    assert_eq!(config.get::<String>("host")?, "localhost");
    assert_eq!(lua.table_geti::<String>(&config, 1)?, "alpha");
    Ok(())
}
```

`Table` and `Function` implement `IntoLua`, so they can be passed back into Lua through typed APIs like `set_global`, `table_set`, or callback returns. They also implement `FromLua`, so you can receive them directly from `eval`, `get_global`, or typed callback arguments.

## Table Traversal

Table traversal is split into two layers:

- `Table::pairs_raw()` returns a snapshot of `(LuaValue, LuaValue)` pairs when you need direct low-level access.
- `Lua::table_pairs<K, V>(&Table)` converts that snapshot into typed pairs.
- `Lua::table_array<T>(&Table)` reads the sequential array part in order from `1..=#t`.

```rust
use luars_safe::{Lua, SafeOption, TableBuilder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut lua = Lua::new(SafeOption::default());
    let pairs_table = lua.build_table(
        TableBuilder::new()
            .set("alpha", 1_i64)
            .set("beta", 2_i64),
    )?;

    let array_table = lua.build_table(
        TableBuilder::new()
            .push("x")
            .push("y"),
    )?;

    let mut pairs = lua.table_pairs::<String, i64>(&pairs_table)?;
    pairs.sort_by(|left, right| left.0.cmp(&right.0));

    let array = lua.table_array::<String>(&array_table)?;

    assert_eq!(pairs, vec![("alpha".to_owned(), 1), ("beta".to_owned(), 2)]);
    assert_eq!(array, vec!["x".to_owned(), "y".to_owned()]);
    Ok(())
}
```

## Current API Surface

`Lua` currently covers the high-frequency embedding path:

- `execute` and `eval`
- typed globals via `set_global` and `get_global`
- typed calls via `call_global` and `call_global1`
- typed Rust callback registration via `register_function` and `register_async_function`
- type registration via `register_type`
- safe `Table` and `Function` handles
- `FromLua` / `IntoLua` support for `Table` and `Function`
- raw and typed table traversal helpers
- safe `TableBuilder`

This crate deliberately does not expose raw `LuaVM` accessors. If your use case needs direct low-level access, depend on `luars` alongside or instead of `luars_safe`.

## Relationship To luars

`luars_safe` does not try to hide the existence of low-level APIs in the ecosystem. It only removes them from the default path for host application code.

That gives the repository a clean split:

- `luars`: low-level runtime and embedding primitives
- `luars_safe`: safe host-facing wrapper for common embedding scenarios

## License

MIT. See [../../LICENSE](../../LICENSE).