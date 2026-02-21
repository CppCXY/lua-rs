# luars UserData API Guide

This guide explains how to define custom Rust types and expose them to Lua scripts.

> **Looking for the full usage guide?** See [Guide.md](Guide.md) for documentation covering the entire API — from VM creation to executing code, working with values, Rust functions, and more.

luars provides a declarative, derive-macro-based API that lets you map a Rust struct to Lua with just a few annotations — including field access, method calls, constructors, operator overloading, and more.

## Contents

| Document | Description |
|----------|-------------|
| [Getting Started](userdata/GettingStarted.md) | 5-minute quickstart: define a Point, create and use it in Lua |
| [#\[derive(LuaUserData)\]](userdata/DeriveUserData.md) | Field exposure and attributes (`skip` / `readonly` / `name`) |
| [#\[lua_methods\]](userdata/LuaMethods.md) | Instance methods, constructors, `#[lua(skip)]` on methods |
| [register_type](userdata/RegisterType.md) | Type registration: `register_type` and `register_type_of` |
| [Type Conversions](userdata/TypeConversions.md) | Parameter types, return types, `Option` / `Result` handling, `FromLua` / `IntoLua` |
| [Complete Examples](userdata/Examples.md) | End-to-end examples: Vec2, AppConfig, Calculator, and more |

## Overall Workflow

```
┌─────────────────────────────────────────────────────────┐
│  1. Define a Rust struct                                 │
│     #[derive(LuaUserData)]                               │
│     struct Point { pub x: f64, pub y: f64 }              │
├─────────────────────────────────────────────────────────┤
│  2. Define methods and constructors                      │
│     #[lua_methods]                                       │
│     impl Point {                                         │
│         pub fn new(x: f64, y: f64) -> Self { ... }       │
│         pub fn distance(&self) -> f64 { ... }            │
│     }                                                    │
├─────────────────────────────────────────────────────────┤
│  3. Register with the Lua VM                             │
│     state.register_type_of::<Point>("Point")?;           │
├─────────────────────────────────────────────────────────┤
│  4. Use in Lua                                           │
│     local p = Point.new(3, 4)                            │
│     print(p.x, p.y)       -- 3.0  4.0                   │
│     print(p:distance())   -- 5.0                         │
│     p:translate(10, 20)                                  │
│     print(tostring(p))    -- Point(13, 24)               │
└─────────────────────────────────────────────────────────┘
```

## Dependencies

Add to your `Cargo.toml`:

```toml
[dependencies]
luars = "0.7"
```

The `#[derive(LuaUserData)]` and `#[lua_methods]` macros are re-exported by `luars` automatically — no need to add `luars-derive` separately.

## Runnable Examples

See [`examples/luars-example/src/main.rs`](../examples/luars-example/src/main.rs) for complete, runnable code covering all the features described in this guide.
