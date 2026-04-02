# luars High-Level UserData Guide

This guide explains how to expose Rust types through the high-level `Lua` API.

If you want the broader embedding workflow, start with [Guide.md](Guide.md). This document focuses on `LuaUserData`, `lua_methods`, and `register_type()`.

## The workflow

1. Define a Rust struct and derive `LuaUserData`
2. Add constructors and methods with `#[lua_methods]`
3. Register the type with `lua.register_type::<T>("Name")`
4. Construct and use the type directly from Lua

## Minimal example

```rust
use luars::{Lua, LuaUserData, SafeOption, Stdlib, lua_methods};

#[derive(LuaUserData)]
struct Point {
	pub x: f64,
	pub y: f64,
}

#[lua_methods]
impl Point {
	pub fn new(x: f64, y: f64) -> Self {
		Self { x, y }
	}

	pub fn translate(&mut self, dx: f64, dy: f64) {
		self.x += dx;
		self.y += dy;
	}

	pub fn distance(&self) -> f64 {
		(self.x * self.x + self.y * self.y).sqrt()
	}
}
```

After registration:

```rust
let mut lua = Lua::new(SafeOption::default());
lua.load_stdlibs(Stdlib::All)?;
lua.register_type::<Point>("Point")?;

let distance: f64 = lua
	.load(
		r#"
		local p = Point.new(3.0, 4.0)
		p:translate(1.0, 2.0)
		return p:distance()
		"#,
	)
	.eval()?;

assert!(distance > 0.0);
```

## What the derive gives you

- Public fields become Lua-visible fields unless you opt out
- Methods in `#[lua_methods]` become callable with `:` from Lua
- Constructors like `new(...)` become available as `TypeName.new(...)`
- Parameters and return values use the same typed conversions as `register_function()` callbacks

## Recommended pattern

Keep business logic inside normal Rust impl blocks, and expose only the methods you want Lua to see.

```rust
#[derive(LuaUserData)]
struct Counter {
	pub count: i64,
}

impl Counter {
	fn bump_by(&mut self, delta: i64) {
		self.count += delta;
	}
}

#[lua_methods]
impl Counter {
	pub fn new(count: i64) -> Self {
		Self { count }
	}

	pub fn inc(&mut self, delta: i64) {
		self.bump_by(delta);
	}

	pub fn get(&self) -> i64 {
		self.count
	}
}
```

This keeps the Lua-facing surface explicit and reviewable.

## Registration

```rust
lua.register_type::<Counter>("Counter")?;
```

After registration, Lua can create and use values immediately:

```lua
local counter = Counter.new(10)
counter:inc(5)
print(counter:get())
```

## Example references

- [../examples/luars-example/src/main.rs](../examples/luars-example/src/main.rs) shows a compact userdata example
- [../examples/rust-bind-bench/src/main.rs](../examples/rust-bind-bench/src/main.rs) shows repeated userdata method calls under load
