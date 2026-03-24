# luars

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)
[![Crates.io](https://img.shields.io/crates/v/luars.svg)](https://crates.io/crates/luars)

A Lua 5.5 interpreter written in pure Rust — embeddable, async-capable, with derive macros for UserData.

## Features

- **Lua 5.5** — full language semantics: compiler, register-based VM, GC
- **Pure Rust** — no C dependencies, no `unsafe` FFI
- **Ergonomic API** — typed-first `call`, `call1`, `call_global`, `call1_global`, typed callback registration, `TableBuilder`, typed getters
- **UserData** — derive macros to expose Rust structs/enums to Lua (fields, methods, operators)
- **Async** — run async Rust functions from Lua via transparent coroutine bridging
- **Closures** — register Rust closures with captured state as Lua globals
- **FromLua / IntoLua** — automatic type conversion for seamless Rust ↔ Lua interop
- **Optional Serde** — JSON serialization via `serde` / `serde_json` (feature-gated)
- **Optional Sandbox** — feature-gated env isolation, injected globals, and runtime limits
- **Standard Libraries** — `basic`, `string`, `table`, `math`, `io`, `os`, `coroutine`, `utf8`, `package`, partial `debug`

## Usage

```toml
[dependencies]
luars = "0.12"

# With JSON support:
luars = { version = "0.12", features = ["serde"] }

# With sandbox support:
luars = { version = "0.12", features = ["sandbox"] }
```

### Basic Example

```rust
use luars::{LuaVM, Stdlib, LuaValue};
use luars::lua_vm::SafeOption;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    // Execute Lua code
    let results = vm.execute("return 1 + 2")?;
    assert_eq!(results[0].as_integer(), Some(3));

    // Set / get globals
    vm.set_global("x", LuaValue::integer(42))?;
    let x: i64 = vm.get_global_as::<i64>("x")?.unwrap();

    // Call a Lua function with typed arguments / returns
    vm.execute("function add(a,b) return a+b end")?;
    let sum: i64 = vm.call1_global("add", (1, 2))?;
    assert_eq!(sum, 3);

    Ok(())
}
```

### Register Rust Functions

```rust
vm.register_function_typed("add", |a: i64, b: i64| a + b)?;
let result: i64 = vm.call1_global("add", (10, 20))?;
assert_eq!(result, 30);
```

Raw callbacks remain available when you want direct stack access:

```rust
vm.register_function("greet", |state| {
    let name = state.get_arg(1).and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "World".into());
    let msg = state.create_string(&format!("Hello, {}!", name))?;
    state.push_value(msg)?;
    Ok(1)
})?;

vm.execute("print(greet('Rust'))")?;  // Hello, Rust!
```

### Load, Dofile, Call

```rust
// Compile without executing
let f = vm.load("return 1 + 1")?;
let result: i64 = vm.call1(f, ())?;

// Load named source
let f = vm.load_with_name("return 42", "my_chunk")?;

// Raw fallback when you already have LuaValue vectors
let results = vm.call_raw(f, vec![])?;

// Execute a file
vm.dofile("scripts/init.lua")?;
```

### TableBuilder

```rust
use luars::TableBuilder;

let config = TableBuilder::new()
    .set("host", vm.create_string("localhost")?)
    .set("port", LuaValue::integer(8080))
    .push(LuaValue::integer(1))
    .build(&mut vm)?;
vm.set_global("config", config)?;

// Iterate
for (k, v) in vm.table_pairs(&config)? {
    println!("{:?} = {:?}", k, v);
}
let len = vm.table_length(&config)?;
```

### Rust Closures

```rust
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

let counter = Arc::new(AtomicUsize::new(0));
let counter_clone = counter.clone();
let func = vm.create_closure(move |state| {
    let n = counter_clone.fetch_add(1, Ordering::SeqCst);
    state.push_value(LuaValue::integer(n as i64))?;
    Ok(1)
})?;
vm.set_global("next_id", func)?;
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
vm.execute(r#"
    local p = Point.new(3, 4)
    print(p.x, p:distance())   -- 3.0  5.0
"#)?;
```

Use `UserDataRef<T>` when Rust callbacks need typed access to userdata values:

```rust
vm.register_function_typed("shift_x", |mut point: UserDataRef<Point>, delta: f64| {
    point.get_mut()?.x += delta;
    point.get()?.x
})?;
```

### Error Handling

```rust
use luars::LuaError;

match vm.execute("error('boom')") {
    Ok(_) => {}
    Err(e) => {
        // Lightweight message
        let msg = vm.get_error_message(e);
        eprintln!("Lua error: {}", msg);

        // Rich error (implements std::error::Error)
        let full = vm.into_full_error(e);
        eprintln!("{}", full);
    }
}
```

`LuaError` is a 1-byte enum (`Runtime`, `Syntax`, `OutOfMemory`, `MessageHandler`, `IndexOutOfBounds`). For rich errors with messages and `std::error::Error` impl, use `vm.into_full_error()` → `LuaFullError`.

### Selective Standard Libraries

```rust
use luars::Stdlib;

vm.open_stdlib(Stdlib::All)?;                          // everything
vm.open_stdlibs(&[Stdlib::Base, Stdlib::String])?;     // specific set
```

### Async

```rust
vm.register_async_typed("fetch", |url: String| async move {
    let body = reqwest::get(&url).await?.text().await?;
    Ok(body)
})?;

let results = vm.execute_async("return fetch('https://example.com')").await?;
```

If you need full manual control, `register_async` still accepts and returns raw `LuaValue`/`AsyncReturnValue` vectors.

## Known Limitations

- **UTF-8 Only Strings** — unlike C Lua, strings must be valid UTF-8. Use `string.pack`/`string.unpack` for binary data.
- **Custom Bytecode Format** — `string.dump` output is not compatible with C Lua bytecode.
- **No C API** — pure Rust; cannot load C Lua modules.
- **Partial Debug Library** — `debug.sethook` is a stub; `getinfo`/`getlocal`/`traceback` work.
- **No string-to-number coercion in arithmetic** — `"3" + 1` raises an error.

## Documentation

| Document | Description |
|----------|-------------|
| [Guide](../../docs/Guide.md) | VM, execution, values, functions, errors, API reference |
| [UserData Guide](../../docs/UserGuide.md) | Derive macros, fields, methods, constructors |
| [Async Guide](../../docs/Async.md) | Async Rust functions, architecture, HTTP server example |
| [Differences](../../docs/Different.md) | Behavioral differences from C Lua 5.5 |

## License

MIT — see [LICENSE](../../LICENSE).
