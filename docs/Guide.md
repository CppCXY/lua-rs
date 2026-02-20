# luars User Guide

A comprehensive guide to embedding the luars Lua 5.5 runtime in your Rust application.

## Getting Started

| Document | Description |
|----------|-------------|
| [Getting Started](guide/01-GettingStarted.md) | Create a VM, load stdlib, run your first Lua script |
| [Executing Code](guide/02-ExecutingCode.md) | `execute_string`, `execute`, compilation, return values |
| [Working with Values](guide/03-WorkingWithValues.md) | `LuaValue`, globals, tables, strings |
| [Rust Functions in Lua](guide/04-RustFunctions.md) | `CFunction`, `RClosure`, `create_closure` |
| [FromLua / IntoLua](guide/05-FromLuaIntoLua.md) | Automatic Rust ↔ Lua type conversion traits |
| [Error Handling](guide/06-ErrorHandling.md) | `LuaError`, `pcall`, `xpcall`, `Result` |
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

## Other

| Document | Description |
|----------|-------------|
| [Differences from C Lua](Different.md) | All known behavioral differences between luars and C Lua 5.5 |

## Quick Example

```rust
use luars::lua_vm::{LuaVM, SafeOption};
use luars::Stdlib;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create a VM
    let mut vm = LuaVM::new(SafeOption::default());

    // 2. Load the standard library
    vm.open_stdlib(Stdlib::All)?;

    // 3. Execute Lua code
    let results = vm.execute_string(r#"
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

## Dependencies

```toml
[dependencies]
luars = "0.4"

# Optional: Enable JSON serialization support
luars = { version = "0.4", features = ["serde"] }
```

The `#[derive(LuaUserData)]` and `#[lua_methods]` macros are re-exported by `luars` automatically — no need to add `luars-derive` separately.
